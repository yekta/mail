//! One loop owns all state. Commands from the app, messages from the server and the results of
//! work that waited on the network all arrive here, one at a time; anything that waits runs in a
//! task of its own and sends its result back as a closure to run on the loop.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use chrono::Local;
use mail_protocol::api::{DevLoginRequest, ExchangeRequest, JmapAccountRequest, LinkTicket, TokenResponse};
use mail_protocol::wire::{ClientMessage, ServerMessage};
use mail_protocol::{Address, Draft, Op, PROTOCOL_VERSION, role};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::api::{Action, Command, Config, Event, MessageView, ThreadView};
use crate::link::{Link, LinkEvent};
use crate::render::drafts::{self, ReplyKind};
use crate::render::{dates, html, rows};
use crate::store::{Store, shown_in};

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// How many bodies of the newest inbox mail are fetched ahead, once caught up.
const PREFETCH: usize = 30;

enum Input {
    Command(u64, Command),
    Link(u64, LinkEvent),
    Run(Box<dyn FnOnce(&mut Core) + Send>),
}

#[derive(Clone)]
pub struct Handle {
    tx: mpsc::UnboundedSender<Input>,
}

impl Handle {
    pub fn send(&self, id: u64, command: Command) {
        let _ = self.tx.send(Input::Command(id, command));
    }
}

pub fn start(config: Config, sink: Sink) -> Result<Handle> {
    mail_protocol::tls::install();
    std::fs::create_dir_all(&config.data_dir)?;
    let mut store = Store::open(&Path::new(&config.data_dir).join("mail.db"))?;
    if config.demo {
        crate::demo::fill(&mut store, mail_protocol::now_ms())?;
    }
    if store.meta("server").is_none()
        && let Some(url) = &config.server_url
    {
        store.set_meta("server", Some(url))?;
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let mut core = Core {
        store,
        demo: config.demo,
        sink,
        tx: tx.clone(),
        http,
        link: None,
        generation: 0,
        connection: "offline".into(),
        google: None,
        next_request: 0,
        requested_bodies: HashSet::new(),
        searches: HashMap::new(),
        cancelled: HashSet::new(),
    };
    tokio::spawn(async move {
        core.connect();
        while let Some(input) = rx.recv().await {
            core.handle(input);
        }
    });
    Ok(Handle { tx })
}

struct Core {
    store: Store,
    /// Shows the demo's mail and never connects.
    demo: bool,
    sink: Sink,
    tx: mpsc::UnboundedSender<Input>,
    http: reqwest::Client,
    link: Option<Link>,
    generation: u64,
    connection: String,
    /// The secret and the state of the Google sign-in in progress.
    google: Option<(String, String)>,
    next_request: u64,
    requested_bodies: HashSet<String>,
    /// The local matches of each search still waiting for the server's.
    searches: HashMap<u64, Vec<String>>,
    /// Sends the user took back, whose refusal is no failure.
    cancelled: HashSet<String>,
}

fn random_hex() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS provides randomness");
    hex::encode(bytes)
}

fn new_op_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Core {
    fn emit(&self, event: Event) {
        (self.sink)(event);
    }

    fn reply(&self, id: u64, result: Result<Value>) {
        let event = match result {
            Ok(value) => Event::Reply { id, ok: true, value },
            Err(error) => Event::Reply { id, ok: false, value: json!({ "error": error.to_string() }) },
        };
        self.emit(event);
    }

    fn changed(&self, mailboxes: bool, threads: Vec<String>) {
        self.emit(Event::Changed { mailboxes, threads });
    }

    fn set_connection(&mut self, state: &str, error: Option<String>) {
        if self.connection == state && error.is_none() {
            return;
        }
        self.connection = state.to_string();
        self.emit(Event::Connection { state: state.to_string(), error });
    }

    fn server(&self) -> Option<String> {
        self.store.meta("server")
    }

    fn token(&self) -> Option<String> {
        self.store.meta("token")
    }

    fn online(&self) -> bool {
        self.connection == "online"
    }

    /// Runs `work` in a task and `then` on the loop with its result.
    fn spawn<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
        then: impl FnOnce(&mut Core, T) + Send + 'static,
    ) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = work.await;
            let _ = tx.send(Input::Run(Box::new(move |core| then(core, result))));
        });
    }

    fn connect(&mut self) {
        self.link = None;
        let (Some(server), Some(_)) = (self.server(), self.token()) else {
            self.set_connection("signed_out", None);
            return;
        };
        if self.demo {
            self.set_connection("online", None);
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        let tx = self.tx.clone();
        self.set_connection("connecting", None);
        self.link = Some(Link::start(&server, move |event| {
            let _ = tx.send(Input::Link(generation, event));
        }));
    }

    fn send(&self, message: ClientMessage) {
        if let Some(link) = &self.link {
            link.send(message);
        }
    }

    fn handle(&mut self, input: Input) {
        match input {
            Input::Command(id, command) => {
                if let Some(result) = self.command(id, command) {
                    self.reply(id, result);
                }
            }
            Input::Link(generation, event) if generation == self.generation => self.link_event(event),
            Input::Link(..) => {}
            Input::Run(run) => run(self),
        }
    }

    /// Answers at once, or returns `None` when a task will answer.
    fn command(&mut self, id: u64, command: Command) -> Option<Result<Value>> {
        Some(match command {
            Command::Status => self.status(),
            Command::SetServer { url } => self.set_server(&url),
            Command::SignInGoogle => return self.sign_in_google(id),
            Command::FinishSignIn { url } => return self.finish_sign_in(id, &url),
            Command::AddJmapAccount { url, username, password } => {
                let request = JmapAccountRequest { url, username, password };
                return self.get_token(id, "/api/accounts/jmap", json!(request));
            }
            Command::DevLogin { email } => {
                return self.get_token(id, "/api/dev/login", json!(DevLoginRequest { email }));
            }
            Command::SignOut => self.sign_out(),
            Command::RemoveAccount { account } => {
                self.mutate(Op::RemoveAccount { account_id: account }).map(|op_id| json!({ "op_id": op_id }))
            }
            Command::Mailboxes => self.mailboxes(),
            Command::Threads { mailbox, offset, limit } => self.threads(&mailbox, offset, limit),
            Command::OpenThread { thread, images } => self.open_thread(&thread, images),
            Command::Act { action, threads, until } => self.act(action, &threads, until),
            Command::ReplyDraft { thread, kind } => self.reply_draft(&thread, kind),
            Command::Send { draft, delay, send_at } => self.queue_send(draft, delay, send_at),
            Command::CancelSend { op_id } => self.cancel_send(&op_id),
            Command::Search { query } => self.search(&query),
        })
    }

    fn status(&self) -> Result<Value> {
        let accounts: Vec<Value> = self
            .store
            .accounts()?
            .into_iter()
            .map(|account| json!({ "id": account.id, "provider": account.provider, "address": account.address, "status": account.status, "color": account.color }))
            .collect();
        Ok(json!({
            "signed_in": self.token().is_some(),
            "server": self.server(),
            "connection": self.connection,
            "accounts": accounts,
        }))
    }

    fn set_server(&mut self, url: &str) -> Result<Value> {
        let url = url.trim().trim_end_matches('/');
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            bail!("The server's address should start with https://");
        }
        if self.server().as_deref() == Some(url) {
            return Ok(json!({}));
        }
        self.store.clear()?;
        self.store.set_meta("server", Some(url))?;
        self.changed(true, Vec::new());
        self.connect();
        Ok(json!({}))
    }

    fn sign_in_google(&mut self, id: u64) -> Option<Result<Value>> {
        let Some(server) = self.server() else { return Some(Err(anyhow!("Set the server first."))) };
        let (verifier, state) = (random_hex(), random_hex());
        let challenge = hex::encode(Sha256::digest(verifier.as_bytes()));
        self.google = Some((verifier, state.clone()));
        let start = format!("{}/auth/google/start?challenge={challenge}&state={state}", server.trim_end_matches('/'));
        let Some(token) = self.token() else {
            return Some(Ok(json!({ "url": start })));
        };
        let http = self.http.clone();
        self.spawn(
            async move {
                crate::http::post::<LinkTicket>(&http, &server, "/api/link-ticket", Some(&token), &json!({})).await
            },
            move |core, ticket| {
                core.reply(id, ticket.map(|ticket| json!({ "url": format!("{start}&ticket={}", ticket.ticket) })))
            },
        );
        None
    }

    fn finish_sign_in(&mut self, id: u64, url: &str) -> Option<Result<Value>> {
        let Ok(url) = reqwest::Url::parse(url) else { return Some(Err(anyhow!("That isn't a sign-in link."))) };
        let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let Some((verifier, state)) = self.google.clone() else {
            return Some(Err(anyhow!("No sign-in is in progress. Start again.")));
        };
        if query.get("state") != Some(&state) {
            return Some(Err(anyhow!("This sign-in was started elsewhere. Start again.")));
        }
        if query.contains_key("error") {
            self.google = None;
            return Some(Err(anyhow!("The sign-in was cancelled.")));
        }
        let Some(code) = query.get("code").cloned() else { return Some(Err(anyhow!("The sign-in link has no code."))) };
        self.google = None;
        self.get_token(id, "/api/auth/exchange", json!(ExchangeRequest { code, verifier }))
    }

    /// Calls an endpoint that answers with a session, keeps it and connects.
    fn get_token(&mut self, id: u64, path: &'static str, body: Value) -> Option<Result<Value>> {
        let Some(server) = self.server() else { return Some(Err(anyhow!("Set the server first."))) };
        let (http, token) = (self.http.clone(), self.token());
        self.spawn(
            async move { crate::http::post::<TokenResponse>(&http, &server, path, token.as_deref(), &body).await },
            move |core, result| {
                let result = result.and_then(|response| {
                    let new = core.token().as_deref() != Some(response.token.as_str());
                    core.store.set_meta("token", Some(&response.token))?;
                    if new {
                        core.connect();
                    }
                    Ok(json!({}))
                });
                core.reply(id, result);
            },
        );
        None
    }

    fn sign_out(&mut self) -> Result<Value> {
        self.link = None;
        self.generation += 1;
        self.store.clear()?;
        self.requested_bodies.clear();
        self.set_connection("signed_out", None);
        self.changed(true, Vec::new());
        Ok(json!({}))
    }

    fn mailboxes(&self) -> Result<Value> {
        let (unified, accounts) = self.store.mailboxes()?;
        Ok(json!({ "unified": unified, "accounts": accounts }))
    }

    fn threads(&self, mailbox: &str, offset: usize, limit: usize) -> Result<Value> {
        let (rows, total) = self.store.thread_rows(mailbox, offset, limit, &Local::now())?;
        Ok(json!({ "rows": rows, "total": total }))
    }

    fn open_thread(&mut self, thread: &str, images: bool) -> Result<Value> {
        let messages = self.store.thread_messages(thread)?;
        let Some(last) = messages.last() else { bail!("This conversation is gone.") };
        let me = self.store.me();
        let now = Local::now();
        let mut people: Vec<Address> = Vec::new();
        let mut views = Vec::new();
        let mut missing = Vec::new();
        for (index, message) in messages.iter().enumerate() {
            people.push(message.from.clone());
            people.extend(message.recipients.to.iter().cloned());
            people.extend(message.recipients.cc.iter().cloned());
            let page = self.store.body(&message.id).map(|body| html::page(&body, images));
            if page.is_none() {
                missing.push(message.id.clone());
            }
            let to: Vec<String> = message
                .recipients
                .to
                .iter()
                .chain(&message.recipients.cc)
                .map(
                    |address| {
                        if me.contains(&address.email) { "me".to_string() } else { address.display().to_string() }
                    },
                )
                .collect();
            let from_name =
                if me.contains(&message.from.email) { "me".to_string() } else { message.from.display().to_string() };
            views.push(MessageView {
                id: message.id.clone(),
                from_name,
                from_email: message.from.email.clone(),
                initials: rows::initials(&message.from),
                to: if to.is_empty() { String::new() } else { format!("to {}", to.join(", ")) },
                date: dates::long(message.date, &now),
                snippet: message.snippet.clone(),
                unread: message.unread,
                folded: index + 1 < messages.len() && !message.unread,
                blocked_images: page.as_ref().is_some_and(|page| page.blocked_images),
                html: page.map(|page| page.html),
                attachments: message
                    .attachments
                    .iter()
                    .filter(|attachment| !attachment.name.is_empty())
                    .cloned()
                    .collect(),
            });
        }
        self.request_bodies(&missing);
        let color = self
            .store
            .accounts()?
            .into_iter()
            .find(|account| account.id == last.account_id)
            .map(|account| account.color)
            .unwrap_or_default();
        let view = ThreadView {
            id: thread.to_string(),
            account_id: last.account_id.clone(),
            color,
            subject: messages
                .iter()
                .map(|message| message.subject.clone())
                .find(|subject| !subject.is_empty())
                .unwrap_or_else(|| "(no subject)".into()),
            participants: rows::participants(&people, &me),
            starred: messages.iter().any(|message| message.starred),
            unread: messages.iter().any(|message| message.unread),
            messages: views,
        };
        let unread: Vec<String> =
            messages.iter().filter(|message| message.unread).map(|message| message.id.clone()).collect();
        if !unread.is_empty() {
            self.mutate(Op::SetUnread { ids: unread, unread: false })?;
        }
        Ok(serde_json::to_value(view)?)
    }

    fn request_bodies(&mut self, ids: &[String]) {
        if !self.online() {
            return;
        }
        for id in ids {
            if !self.requested_bodies.insert(id.clone()) {
                continue;
            }
            self.next_request += 1;
            self.send(ClientMessage::Body { request: self.next_request, message_id: id.clone() });
        }
    }

    /// Shows the op, keeps it in the outbox and sends it. Returns its id.
    fn mutate(&mut self, op: Op) -> Result<String> {
        let op_id = new_op_id();
        let threads = self.store.apply_local(&op_id, &op)?;
        if !threads.is_empty() {
            self.changed(true, threads);
        }
        if self.online() {
            self.send(ClientMessage::Mutate { op_id: op_id.clone(), op });
        }
        Ok(op_id)
    }

    fn act(&mut self, action: Action, threads: &[String], until: Option<i64>) -> Result<Value> {
        let mut ids = Vec::new();
        for thread in threads {
            let messages = self.store.thread_messages(thread)?;
            let has =
                |message: &mail_protocol::Message, label: &str| message.labels.iter().any(|existing| existing == label);
            let pick: Vec<String> = match action {
                Action::Archive | Action::Snooze => {
                    messages.iter().filter(|m| has(m, role::INBOX)).map(|m| m.id.clone()).collect()
                }
                Action::Trash => messages.iter().filter(|m| !has(m, role::TRASH)).map(|m| m.id.clone()).collect(),
                Action::Spam => messages.iter().filter(|m| !has(m, role::SPAM)).map(|m| m.id.clone()).collect(),
                Action::Read => messages.iter().filter(|m| m.unread).map(|m| m.id.clone()).collect(),
                Action::Unread | Action::Star => messages.last().map(|m| vec![m.id.clone()]).unwrap_or_default(),
                Action::Unstar => messages.iter().filter(|m| m.starred).map(|m| m.id.clone()).collect(),
                Action::Inbox => messages
                    .iter()
                    .filter(|m| !has(m, role::INBOX) && shown_in(m).iter().any(|label| label != role::SENT))
                    .map(|m| m.id.clone())
                    .collect(),
            };
            ids.extend(pick);
        }
        if ids.is_empty() {
            return Ok(json!({}));
        }
        let op = match action {
            Action::Archive => Op::Archive { ids },
            Action::Trash => Op::Trash { ids },
            Action::Spam => Op::Spam { ids },
            Action::Read => Op::SetUnread { ids, unread: false },
            Action::Unread => Op::SetUnread { ids, unread: true },
            Action::Star => Op::SetStarred { ids, starred: true },
            Action::Unstar => Op::SetStarred { ids, starred: false },
            Action::Inbox => Op::MoveToInbox { ids },
            Action::Snooze => {
                let Some(until) = until else { bail!("Say until when.") };
                Op::Snooze { ids, until }
            }
        };
        let op_id = self.mutate(op)?;
        Ok(json!({ "op_id": op_id }))
    }

    fn reply_draft(&self, thread: &str, kind: ReplyKind) -> Result<Value> {
        let messages = self.store.thread_messages(thread)?;
        let me = self.store.me();
        let answered = messages.iter().rev().find(|message| !me.contains(&message.from.email)).or(messages.last());
        let Some(message) = answered else { bail!("This conversation is gone.") };
        let body = self
            .store
            .body(&message.id)
            .map(|body| drafts::body_text(&body))
            .unwrap_or_else(|| message.snippet.clone());
        let draft = drafts::reply(message, &body, kind, &me, &Local);
        let from = self.store.account_address(&draft.account_id).unwrap_or_default();
        Ok(json!({ "draft": draft, "from": from }))
    }

    fn queue_send(&mut self, mut draft: Draft, delay: u64, send_at: Option<i64>) -> Result<Value> {
        if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
            bail!("Add someone to send it to.");
        }
        if draft.account_id.is_empty() {
            let Some(first) = self.store.accounts()?.into_iter().next() else { bail!("Add an account first.") };
            draft.account_id = first.id;
        }
        let send_at = send_at.unwrap_or_else(|| mail_protocol::now_ms() + delay as i64 * 1000);
        let op_id = self.mutate(Op::Send { draft: Box::new(draft), send_at })?;
        Ok(json!({ "op_id": op_id, "send_at": send_at }))
    }

    fn cancel_send(&mut self, op_id: &str) -> Result<Value> {
        let Some(Op::Send { draft, .. }) = self.store.outbox_op(op_id) else { bail!("It was already sent.") };
        self.cancelled.insert(op_id.to_string());
        if !self.online() {
            self.store.settle(op_id, false)?;
            return Ok(json!({ "draft": draft }));
        }
        self.mutate(Op::CancelSend { op_id: op_id.to_string() })?;
        Ok(json!({ "draft": draft }))
    }

    fn search(&mut self, query: &str) -> Result<Value> {
        self.next_request += 1;
        let request = self.next_request;
        let threads = self.store.search(query)?;
        let rows = self.store.rows_of(&threads, &Local::now())?;
        if self.online() && !query.trim().is_empty() {
            self.searches.insert(request, threads);
            self.send(ClientMessage::Search { request, query: query.to_string() });
        }
        Ok(json!({ "request": request, "rows": rows }))
    }

    fn link_event(&mut self, event: LinkEvent) {
        match event {
            LinkEvent::Opened => {
                let Some(token) = self.token() else { return };
                self.send(ClientMessage::Hello { token, cursor: self.store.cursor(), protocol: PROTOCOL_VERSION });
            }
            LinkEvent::Closed(error) => {
                if self.token().is_some() {
                    self.requested_bodies.clear();
                    self.set_connection("offline", error);
                }
            }
            LinkEvent::Message(message) => self.server_message(message),
        }
    }

    fn server_message(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Welcome { .. } => {
                self.set_connection("online", None);
                for (op_id, op) in self.store.outbox().unwrap_or_default() {
                    self.send(ClientMessage::Mutate { op_id, op });
                }
            }
            ServerMessage::Changes { accounts, labels, messages, cursor, more } => {
                match self.store.apply_changes(&accounts, &labels, &messages, cursor) {
                    Ok(threads) => self.changed(true, threads),
                    Err(error) => {
                        tracing::error!("couldn't store changes: {error:#}");
                        self.emit(Event::Error { message: "Mail couldn't be saved on this device.".into() });
                    }
                }
                if !more {
                    let missing = self.store.missing_bodies(PREFETCH).unwrap_or_default();
                    self.request_bodies(&missing);
                }
            }
            ServerMessage::Applied { op_id, ok, error } => self.applied(&op_id, ok, error),
            ServerMessage::Body { message_id, body, .. } => {
                let Some(body) = body else {
                    self.requested_bodies.remove(&message_id);
                    return;
                };
                match self.store.save_body(&message_id, &body) {
                    Ok(Some(thread)) => self.changed(false, vec![thread]),
                    Ok(None) => {}
                    Err(error) => tracing::warn!("couldn't keep a body: {error:#}"),
                }
            }
            ServerMessage::SearchResults { request, message_ids } => {
                let mut threads = self.searches.remove(&request).unwrap_or_default();
                for thread in self.store.threads_of_messages(&message_ids) {
                    if !threads.contains(&thread) {
                        threads.push(thread);
                    }
                }
                let rows = self.store.rows_of(&threads, &Local::now()).unwrap_or_default();
                self.emit(Event::SearchResults { request, rows });
            }
            ServerMessage::Pong => {}
            ServerMessage::Refused { reason } => {
                if reason == "signed_out" {
                    let _ = self.sign_out();
                    return;
                }
                self.link = None;
                self.set_connection("offline", Some(reason));
            }
        }
    }

    fn applied(&mut self, op_id: &str, ok: bool, error: Option<String>) {
        let (op, threads) = match self.store.settle(op_id, ok) {
            Ok(settled) => settled,
            Err(error) => {
                tracing::error!("couldn't settle an op: {error:#}");
                return;
            }
        };
        if !threads.is_empty() {
            self.changed(true, threads);
        }
        let error = error.unwrap_or_else(|| "The server refused the change.".into());
        match op {
            Some(Op::Send { draft, .. }) if ok => {
                let _ = draft;
                self.emit(Event::Sent { op_id: op_id.to_string() });
            }
            Some(Op::Send { draft, .. }) => {
                if !self.cancelled.remove(op_id) {
                    self.emit(Event::SendFailed { op_id: op_id.to_string(), error, draft });
                }
            }
            Some(Op::CancelSend { op_id: target }) if !ok => {
                self.cancelled.remove(&target);
                self.emit(Event::Error { message: error });
            }
            Some(_) if !ok => self.emit(Event::Error { message: error }),
            _ => {}
        }
    }
}
