//! One loop owns all state. The app's commands come first; what the server sends waits its turn
//! in `backlog`, and a batch of changes is applied in small steps, so a command never waits long
//! behind sync. Anything that waits on the network or reads at length runs in a task of its own
//! and sends its result back as a closure to run on the loop.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use chrono::Local;
use mail_protocol::api::{DevLoginRequest, ExchangeRequest, JmapAccountRequest, LinkTicket, TokenResponse};
use mail_protocol::wire::{ClientMessage, ServerMessage};
use mail_protocol::{
    ACCOUNT_COLORS, Account, Address, Draft, DraftAttachment, Label, Message, MessageState, Op, PROTOCOL_VERSION,
    Preference, SavedDraft, role,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::api::{
    ActReply, Action, Boot, Command, Config, Contacts, DraftReply, Event, Filter, MessageView, PreferenceValue,
    Preferences, ThreadView, UiState,
};
use crate::link::{Link, LinkEvent};
use crate::render::drafts::{self, ReplyKind};
use crate::render::html::{self, Printed};
use crate::render::{compose, dates, rows, times};
use crate::store::{Arrived, Batch, Reader, Store, Touched, shown_in, state_of};
use crate::undo;

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// How many bodies of the newest inbox mail are fetched ahead, once caught up.
const PREFETCH: usize = 200;
/// How many body requests wait on the server at once, besides those of the thread being opened.
const BODIES_IN_FLIGHT: usize = 4;
/// How long a body the server failed to fetch twice is shown as failed before it is asked again.
const BODY_RETRY: Duration = Duration::from_secs(30);
/// How many messages of a batch are applied in one step.
const STEP: usize = 100;
/// How often `Changed` is sent at most while the server streams.
const CHANGED_EVERY: Duration = Duration::from_millis(80);
/// How many actions `Undo` can take back.
const UNDO_DEPTH: usize = 30;
/// How long a `Refresh` waits for the server before answering anyway.
const REFRESH_WAIT: Duration = Duration::from_secs(15);
/// Only mail this recent is announced.
const NEW_MAIL_WINDOW: i64 = 2 * 86_400_000;

enum Input {
    Link(u64, LinkEvent),
    Run(Box<dyn FnOnce(&mut Core) + Send>),
}

/// Work done in order between the app's commands: what the server sent, and the rest of a long
/// action.
enum Step {
    Link(LinkEvent),
    Changes(Box<Part>),
    /// An op of the action that `Undo` would take back as `action`.
    Mutate {
        action: u64,
        op: Op,
    },
}

/// A part of one `Changes` batch: everything but messages comes with the first, the cursor with
/// the last.
#[derive(Default)]
struct Part {
    accounts: Vec<Account>,
    labels: Vec<Label>,
    messages: Vec<Message>,
    preferences: Vec<Preference>,
    drafts: Vec<SavedDraft>,
    cursor: Option<i64>,
    /// On the last part: whether another batch follows.
    more: Option<bool>,
}

#[derive(Clone)]
pub struct Handle {
    commands: mpsc::UnboundedSender<(u64, Command)>,
}

impl Handle {
    pub fn send(&self, id: u64, command: Command) {
        let _ = self.commands.send((id, command));
    }
}

pub fn start(config: Config, sink: Sink) -> Result<Handle> {
    mail_protocol::tls::install();
    std::fs::create_dir_all(&config.data_dir)?;
    let data_dir = PathBuf::from(&config.data_dir);
    let mut store = Store::open(&data_dir.join("mail.db"))?;
    if config.demo {
        crate::demo::fill(&mut store, mail_protocol::now_ms())?;
    }
    // The app's own server moves with the app; one the user picked stays.
    if store.meta("server_chosen").is_none()
        && let Some(url) = &config.server_url
    {
        store.set_meta("server", Some(url))?;
    }
    let reader = Arc::new(Mutex::new(store.reader()?));
    let (commands_tx, mut commands) = mpsc::unbounded_channel();
    let (tx, mut inputs) = mpsc::unbounded_channel();
    let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
    let mut core = Core {
        store,
        reader,
        data_dir,
        demo: config.demo,
        sink,
        tx,
        http,
        link: None,
        generation: 0,
        connection: "offline".into(),
        google: None,
        next_request: 0,
        backlog: VecDeque::new(),
        pending: Touched::default(),
        flush_due: false,
        bodies_in_flight: HashSet::new(),
        body_failures: HashMap::new(),
        soon: VecDeque::new(),
        later: VecDeque::new(),
        searches: HashMap::new(),
        latest_search: Arc::new(AtomicU64::new(0)),
        cancelled: HashMap::new(),
        uploading: HashSet::new(),
        upload_tries: HashMap::new(),
        announcing: HashSet::new(),
        undo: Vec::new(),
        actions: 0,
        first_sync: false,
        arrived: Vec::new(),
        refreshing: Vec::new(),
        refresh: 0,
    };
    core.announcing = core.ready_accounts();
    tokio::spawn(async move {
        core.connect();
        loop {
            while let Ok((id, command)) = commands.try_recv() {
                core.command(id, command);
            }
            while let Ok(input) = inputs.try_recv() {
                core.input(input);
            }
            if let Some(step) = core.backlog.pop_front() {
                core.step(step);
                continue;
            }
            tokio::select! {
                biased;
                command = commands.recv() => {
                    let Some((id, command)) = command else { break };
                    core.command(id, command);
                }
                Some(input) = inputs.recv() => core.input(input),
            }
        }
    });
    Ok(Handle { commands: commands_tx })
}

struct Core {
    store: Store,
    reader: Arc<Mutex<Reader>>,
    data_dir: PathBuf,
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
    backlog: VecDeque<Step>,
    /// What changed and the apps weren't told yet.
    pending: Touched,
    flush_due: bool,
    bodies_in_flight: HashSet<String>,
    /// The bodies the server failed to fetch: how many times in a row, and when last.
    body_failures: HashMap<String, (u32, Instant)>,
    /// Bodies to ask for: of threads about to be opened, then of the inbox's newest mail.
    soon: VecDeque<String>,
    later: VecDeque<String>,
    /// The local matches of each search still waiting for the server's.
    searches: HashMap<u64, Vec<String>>,
    latest_search: Arc<AtomicU64>,
    /// Sends the user took back, whose refusal is no failure, with the saved draft they went back to.
    cancelled: HashMap<String, String>,
    /// Sends whose files are going up.
    uploading: HashSet<String>,
    /// How many times each send's files failed to go up in a row.
    upload_tries: HashMap<String, u32>,
    /// The accounts that were caught up and ready at the last catch-up: only their new mail is
    /// announced, not that of an account still syncing for the first time.
    announcing: HashSet<String>,
    /// The ops that take back each recent action, the last last, with the action's number.
    undo: Vec<(u64, Vec<Op>)>,
    actions: u64,
    /// The first sync of this copy is under way: its mail isn't announced. Kept as `caught_up`
    /// in meta once done.
    first_sync: bool,
    arrived: Vec<Arrived>,
    /// The `Refresh` commands waiting for the server's `Synced`, and the number of the wait.
    refreshing: Vec<u64>,
    refresh: u64,
}

fn random_hex() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS provides randomness");
    hex::encode(bytes)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A send whose files haven't all gone up yet.
fn waits_for_upload(op: &Op) -> bool {
    let Op::Send { draft, .. } = op else { return false };
    draft.attachments.iter().any(|attachment| attachment.upload.is_none() && attachment.path.is_some())
}

/// A name that stays one file in one folder.
fn file_name(name: &str) -> String {
    let name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect();
    let name = name.trim().trim_start_matches('.');
    if name.is_empty() { "attachment".into() } else { name.to_string() }
}

fn pick(messages: &[Message], keep: impl Fn(&Message) -> bool) -> Vec<String> {
    messages.iter().filter(|message| keep(message)).map(|message| message.id.clone()).collect()
}

fn has(message: &Message, label: &str) -> bool {
    message.labels.iter().any(|existing| existing == label)
}

enum Upload {
    /// A file is gone from the device: the send can't happen.
    Missing(String),
    Failed(anyhow::Error),
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

    fn changed(&mut self, touched: Touched) {
        self.pending.merge(touched);
    }

    /// Tells the apps what changed since the last time.
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let Touched { threads, mailboxes, preferences } = std::mem::take(&mut self.pending);
        self.emit(Event::Changed { mailboxes, threads: threads.into_iter().collect(), preferences });
    }

    /// Tells the apps a little later, so a stream of changes is one reload.
    fn flush_soon(&mut self) {
        if self.flush_due || self.pending.is_empty() {
            return;
        }
        self.flush_due = true;
        self.spawn(tokio::time::sleep(CHANGED_EVERY), |core, _| core.flush_due = false);
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
        self.backlog.clear();
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

    fn input(&mut self, input: Input) {
        match input {
            // Bodies and search results don't depend on the order of changes: they skip the line.
            Input::Link(
                generation,
                LinkEvent::Message(message @ (ServerMessage::Body { .. } | ServerMessage::SearchResults { .. })),
            ) if generation == self.generation => {
                self.server_message(message);
                self.flush_soon();
            }
            Input::Link(generation, event) if generation == self.generation => {
                self.backlog.push_back(Step::Link(event))
            }
            Input::Link(..) => {}
            Input::Run(run) => {
                run(self);
                if !self.flush_due {
                    self.flush();
                }
            }
        }
    }

    fn step(&mut self, step: Step) {
        match step {
            Step::Link(event) => {
                self.link_event(event);
                self.flush_soon();
            }
            Step::Changes(part) => self.apply_part(*part),
            Step::Mutate { op, .. } => {
                if let Err(error) = self.mutate(op) {
                    tracing::error!("couldn't apply an op: {error:#}");
                }
                self.flush_soon();
            }
        }
    }

    fn command(&mut self, id: u64, command: Command) {
        let answer = self.answer(id, command);
        self.flush();
        if let Some(result) = answer {
            self.reply(id, result);
        }
    }

    /// Answers at once, or returns `None` when a task will answer.
    fn answer(&mut self, id: u64, command: Command) -> Option<Result<Value>> {
        let to_value = |value: Result<ActReply>| value.and_then(|value| Ok(serde_json::to_value(value)?));
        Some(match command {
            Command::Status => self.status(),
            Command::Boot => self.boot(),
            Command::SaveUi { ui } => self.save_ui(&ui),
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
            Command::SetAccountColor { account, color } => self.set_account_color(account, color),
            Command::Mailboxes => self.mailboxes(),
            Command::Threads { mailbox, offset, limit, filter, keep } => {
                self.threads(&mailbox, offset, limit, filter, &keep)
            }
            Command::Prefetch { threads } => self.prefetch(&threads),
            Command::OpenThread { thread, images } => self.open_thread(&thread, images),
            Command::Act { action, threads, until, label } => to_value(self.act(action, &threads, until, label)),
            Command::Undo => self.take_back(),
            Command::ArchiveAll { mailbox, before, filter, keep } => {
                to_value(self.archive_all(&mailbox, before, filter, &keep))
            }
            Command::CreateLabel { account, name } => self.create_label(account, &name),
            Command::ParseTime { text } => Ok(json!({ "choices": times::parse(&text, &Local::now()) })),
            Command::Contacts { query, limit } => {
                self.store.contacts(&query, limit).and_then(|contacts| Ok(serde_json::to_value(Contacts { contacts })?))
            }
            Command::Person { email } => {
                self.store.person(&email, &Local::now()).and_then(|person| Ok(serde_json::to_value(person)?))
            }
            Command::Preferences => self.preferences(),
            Command::SetPreference { key, value } => self.set_preference(key, value),
            Command::NewDraft { account } => self.new_draft(account),
            Command::ReplyDraft { thread, kind } => self.reply_draft(&thread, kind),
            Command::SaveDraft { draft_id, draft } => self.save_draft(draft_id, draft),
            Command::OpenDraft { draft_id } => self.open_draft(&draft_id),
            Command::DeleteDraft { draft_id } => self.mutate(Op::DeleteDraft { draft_id }).map(|_| json!({})),
            Command::Send { draft, delay, send_at, remind_at, draft_id } => {
                self.queue_send(draft, delay, send_at, remind_at, draft_id)
            }
            Command::CancelSend { op_id } => self.cancel_send(&op_id),
            Command::OpenAttachment { message, index } => return self.open_attachment(id, &message, index),
            Command::PrintThread { thread } => self.print_thread(&thread),
            Command::Search { query } => return self.search(id, query),
            Command::Refresh => return self.refresh(id),
        })
    }

    /// Asks the server for a sync now and answers once it has synced. Refreshes asked for
    /// meanwhile join the one under way. Offline, the link is made anew at once instead of
    /// after its backoff, and the ask goes out with the hello.
    fn refresh(&mut self, id: u64) -> Option<Result<Value>> {
        if self.demo || self.token().is_none() {
            return Some(Ok(json!({})));
        }
        self.refreshing.push(id);
        if self.refreshing.len() > 1 {
            return None;
        }
        self.next_request += 1;
        self.refresh = self.next_request;
        let request = self.refresh;
        match self.online() {
            true => self.send(ClientMessage::Sync { request }),
            false => self.connect(),
        }
        self.spawn(tokio::time::sleep(REFRESH_WAIT), move |core, _| core.refreshed(request));
        None
    }

    fn refreshed(&mut self, request: u64) {
        if request != self.refresh {
            return;
        }
        for id in std::mem::take(&mut self.refreshing) {
            self.reply(id, Ok(json!({})));
        }
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

    /// Everything the first frame shows, as the app was left: a page the app draws in one go.
    fn boot(&mut self) -> Result<Value> {
        let ui = self.ui();
        let status = self.status()?;
        let mailboxes = self.mailboxes()?;
        let limit = ui.rows.clamp(100, 1000);
        let page = self.store.thread_page(&ui.mailbox, 0, limit, ui.filter, &[], &Local::now())?;
        let thread = ui.thread.as_deref().and_then(|thread| self.open_thread(thread, ui.images).ok());
        let search = match ui.search.trim() {
            "" => None,
            query => {
                let reader = self.reader.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                reader.search(query, &Local::now()).ok().map(|(_, rows)| rows)
            }
        };
        let values = self.store.preferences()?.into_iter().map(|(key, value)| PreferenceValue { key, value }).collect();
        let boot = Boot { status, mailboxes, page, thread, search, preferences: Preferences { values }, ui };
        Ok(serde_json::to_value(boot)?)
    }

    fn ui(&self) -> UiState {
        self.store.meta("ui").and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
    }

    fn save_ui(&mut self, ui: &UiState) -> Result<Value> {
        self.store.set_meta("ui", Some(&serde_json::to_string(ui)?))?;
        Ok(json!({}))
    }

    fn set_server(&mut self, url: &str) -> Result<Value> {
        let url = url.trim().trim_end_matches('/');
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            bail!("The server's address should start with https://");
        }
        if self.server().as_deref() == Some(url) {
            return Ok(json!({}));
        }
        self.forget()?;
        self.store.set_meta("server", Some(url))?;
        self.store.set_meta("server_chosen", Some("1"))?;
        self.connect();
        Ok(json!({}))
    }

    /// Forgets all mail and everything waiting on it.
    fn forget(&mut self) -> Result<()> {
        self.store.clear()?;
        self.store.set_meta("ui", None)?;
        self.backlog.clear();
        self.bodies_in_flight.clear();
        self.body_failures.clear();
        self.soon.clear();
        self.later.clear();
        self.undo.clear();
        self.arrived.clear();
        self.announcing.clear();
        self.changed(Touched { mailboxes: true, preferences: true, ..Default::default() });
        Ok(())
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
        self.forget()?;
        self.set_connection("signed_out", None);
        Ok(json!({}))
    }

    fn mailboxes(&self) -> Result<Value> {
        let (unified, accounts) = self.store.mailboxes()?;
        Ok(json!({ "unified": unified, "accounts": accounts }))
    }

    fn threads(
        &self,
        mailbox: &str,
        offset: usize,
        limit: usize,
        filter: Option<Filter>,
        keep: &[String],
    ) -> Result<Value> {
        let page = self.store.thread_page(mailbox, offset, limit, filter, keep, &Local::now())?;
        Ok(serde_json::to_value(page)?)
    }

    fn prefetch(&mut self, threads: &[String]) -> Result<Value> {
        let mut ids = Vec::new();
        for thread in threads {
            ids.extend(self.store.bodies_to_fetch(thread)?);
        }
        self.queue_bodies(ids, true);
        Ok(json!({}))
    }

    fn open_thread(&mut self, thread: &str, images: bool) -> Result<Value> {
        let messages = self.store.thread_messages(thread)?;
        let Some(last) = messages.last() else { bail!("This conversation is gone.") };
        let images = images || self.store.preference("remote_images") != Some(json!(false));
        let dark = self.store.preference("dark_mail") != Some(json!(false));
        let me = self.store.me();
        let now = Local::now();
        let mut people: Vec<Address> = Vec::new();
        let mut views = Vec::new();
        let mut missing = Vec::new();
        for (index, message) in messages.iter().enumerate() {
            people.push(message.from.clone());
            people.extend(message.recipients.to.iter().cloned());
            people.extend(message.recipients.cc.iter().cloned());
            let body = self.store.body(&message.id);
            let designed = body.as_ref().and_then(|body| body.html.as_deref()).is_some_and(html::is_designed);
            let page = body.map(|body| html::page(&body, images, dark));
            let failed = page.is_none() && self.body_failed_lately(&message.id);
            if page.is_none() && !failed {
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
                snippet: rows::snippet(&message.snippet),
                unread: message.unread,
                folded: index + 1 < messages.len() && !message.unread,
                blocked_images: page.as_ref().is_some_and(|page| page.blocked_images),
                designed,
                html: page.map(|page| page.html),
                failed,
                attachments: message
                    .attachments
                    .iter()
                    .filter(|attachment| !attachment.name.is_empty())
                    .cloned()
                    .collect(),
            });
        }
        self.fetch_bodies_now(&missing);
        let color = self
            .store
            .accounts()?
            .into_iter()
            .find(|account| account.id == last.account_id)
            .map(|account| account.color)
            .unwrap_or_default();
        let mut custom: Vec<String> = Vec::new();
        for label in messages.iter().flat_map(|message| &message.labels) {
            if !role::ALL.contains(&label.as_str()) && !custom.contains(label) {
                custom.push(label.clone());
            }
        }
        let with = messages.iter().rev().find(|message| !me.contains(&message.from.email));
        let person = match with {
            Some(message) => Some(message.from.email.clone()),
            None => last
                .recipients
                .to
                .iter()
                .find(|address| !me.contains(&address.email))
                .map(|address| address.email.clone()),
        };
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
            muted: self.store.preference(&format!("muted:{thread}")) == Some(json!(true)),
            labels: self.store.labels_named(&custom)?,
            unsubscribe: messages.iter().any(|message| {
                message.unsubscribe.as_ref().is_some_and(|way| way.url.is_some() || way.mailto.is_some())
            }),
            draft_id: self.store.reply_draft_of(thread),
            person,
            messages: views,
        };
        let unread: Vec<String> =
            messages.iter().filter(|message| message.unread).map(|message| message.id.clone()).collect();
        if !unread.is_empty() {
            self.mutate(Op::SetUnread { ids: unread, unread: false })?;
        }
        Ok(serde_json::to_value(view)?)
    }

    // ---------- bodies ----------

    /// Asks for the bodies of the thread being opened, ahead of everything else.
    fn fetch_bodies_now(&mut self, ids: &[String]) {
        if !self.online() {
            return;
        }
        for id in ids {
            if self.bodies_in_flight.insert(id.clone()) {
                self.next_request += 1;
                self.send(ClientMessage::Body { request: self.next_request, message_id: id.clone() });
            }
        }
    }

    fn queue_bodies(&mut self, ids: Vec<String>, soon: bool) {
        for id in ids {
            if self.bodies_in_flight.contains(&id) || self.soon.contains(&id) || self.later.contains(&id) {
                continue;
            }
            match soon {
                true => self.soon.push_back(id),
                false => self.later.push_back(id),
            }
        }
        self.pump_bodies();
    }

    fn pump_bodies(&mut self) {
        if !self.online() {
            return;
        }
        while self.bodies_in_flight.len() < BODIES_IN_FLIGHT {
            let Some(id) = self.soon.pop_front().or_else(|| self.later.pop_front()) else { break };
            if self.store.body(&id).is_some()
                || self.body_failed_lately(&id)
                || !self.bodies_in_flight.insert(id.clone())
            {
                continue;
            }
            self.next_request += 1;
            self.send(ClientMessage::Body { request: self.next_request, message_id: id });
        }
    }

    /// A body the server failed to fetch is asked for again at once; failing again, it shows as
    /// failed for a while.
    fn body_failed(&mut self, id: String) {
        let (count, at) = self.body_failures.entry(id.clone()).or_insert((0, Instant::now()));
        *count += 1;
        *at = Instant::now();
        if *count < 2 {
            self.fetch_bodies_now(&[id]);
            return;
        }
        if let Some(thread) = self.store.thread_of_message(&id) {
            self.changed(Touched { threads: HashSet::from([thread]), ..Default::default() });
        }
    }

    fn body_failed_lately(&self, id: &str) -> bool {
        self.body_failures.get(id).is_some_and(|(count, at)| *count >= 2 && at.elapsed() < BODY_RETRY)
    }

    // ---------- ops ----------

    /// Shows the op, keeps it in the outbox and sends it. Returns its id.
    fn mutate(&mut self, op: Op) -> Result<String> {
        let op_id = new_id();
        self.mutate_as(&op_id, op)?;
        Ok(op_id)
    }

    fn mutate_as(&mut self, op_id: &str, op: Op) -> Result<()> {
        let touched = self.store.apply_local(op_id, &op)?;
        self.changed(touched);
        self.dispatch(op_id.to_string(), op);
        Ok(())
    }

    /// Sends an op to the server, once its files are up.
    fn dispatch(&mut self, op_id: String, op: Op) {
        if !self.online() || self.demo {
            return;
        }
        if waits_for_upload(&op) {
            self.upload(op_id, op);
            return;
        }
        self.send(ClientMessage::Mutate { op_id, op });
    }

    fn upload(&mut self, op_id: String, op: Op) {
        let Op::Send { draft, .. } = op else { return };
        let (Some(server), Some(token)) = (self.server(), self.token()) else { return };
        if !self.uploading.insert(op_id.clone()) {
            return;
        }
        let files: Vec<(usize, DraftAttachment)> = draft
            .attachments
            .into_iter()
            .enumerate()
            .filter(|(_, attachment)| attachment.upload.is_none() && attachment.path.is_some())
            .collect();
        let (http, tx, sending) = (self.http.clone(), self.tx.clone(), op_id.clone());
        self.spawn(
            async move {
                for (index, file) in files {
                    let path = file.path.unwrap_or_default();
                    let bytes = tokio::fs::read(&path)
                        .await
                        .map_err(|error| Upload::Missing(format!("{} couldn't be read: {error}", file.name)))?;
                    let upload = crate::http::upload(&http, &server, &token, &file.name, &file.mime, bytes)
                        .await
                        .map_err(Upload::Failed)?;
                    // Kept as each one is up, so a retry doesn't send it again.
                    let sending = sending.clone();
                    let _ = tx.send(Input::Run(Box::new(move |core| {
                        if let Err(error) = core.store.set_uploads(&sending, &[(index, upload)]) {
                            tracing::error!("couldn't keep an upload: {error:#}");
                        }
                    })));
                }
                Ok(())
            },
            move |core, result: Result<(), Upload>| {
                core.uploading.remove(&op_id);
                match result {
                    Ok(()) => {
                        core.upload_tries.remove(&op_id);
                        if let Some(op) = core.store.outbox_op(&op_id) {
                            core.dispatch(op_id, op);
                        }
                    }
                    Err(Upload::Failed(error)) => {
                        tracing::warn!("couldn't upload a file: {error:#}");
                        core.retry_upload(op_id);
                    }
                    Err(Upload::Missing(error)) => {
                        core.upload_tries.remove(&op_id);
                        let original = core.store.original(&op_id);
                        let Ok((Some(Op::Send { draft, .. }), _)) = core.store.settle(&op_id, false) else { return };
                        let (draft, id) = core.save_back(original, *draft);
                        core.emit(Event::SendFailed { op_id, error, draft: Box::new(draft), draft_id: Some(id) });
                    }
                }
            },
        );
    }

    /// Tries a send's files again later, waiting longer each time, up to five minutes.
    fn retry_upload(&mut self, op_id: String) {
        let tries = self.upload_tries.entry(op_id.clone()).or_insert(0);
        *tries += 1;
        let wait = Duration::from_secs((1u64 << (*tries).min(9)).min(300));
        self.spawn(tokio::time::sleep(wait), move |core, _| {
            let Some(op) = core.store.outbox_op(&op_id) else {
                core.upload_tries.remove(&op_id);
                return;
            };
            if waits_for_upload(&op) {
                core.dispatch(op_id, op);
            }
        });
    }

    /// Keeps a send that won't go as a saved draft again, as the user wrote it and under the id
    /// it had, so it shows in Drafts. `sent` stands in when the original is gone. Returns the
    /// draft and its id.
    fn save_back(&mut self, original: Option<(Draft, Option<String>)>, sent: Draft) -> (Draft, String) {
        let (draft, id) = original.unwrap_or((sent, None));
        let id = id.unwrap_or_else(new_id);
        if let Err(error) = self.mutate(Op::SaveDraft { draft_id: id.clone(), draft: Box::new(draft.clone()) }) {
            tracing::error!("couldn't keep a draft: {error:#}");
        }
        (draft, id)
    }

    /// Keeps the ops that take an action back. Returns whether there are any.
    fn remember_undo(&mut self, ops: Vec<Op>) -> bool {
        if ops.is_empty() {
            return false;
        }
        self.actions += 1;
        self.undo.push((self.actions, ops));
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        true
    }

    fn act(
        &mut self,
        action: Action,
        threads: &[String],
        until: Option<i64>,
        label: Option<String>,
    ) -> Result<ActReply> {
        let needs_label = matches!(action, Action::AddLabel | Action::RemoveLabel | Action::Move);
        let label = label.filter(|label| !label.is_empty());
        if needs_label && label.is_none() {
            bail!("Say which label.");
        }
        let label = label.unwrap_or_default();
        // A custom label belongs to one account: only that account's mail can have it.
        let label_account = self.store.label(&label).map(|(account, _)| account);
        let me = self.store.me();
        let mut states: HashMap<String, MessageState> = HashMap::new();
        let (mut ids, mut out_of_inbox) = (Vec::new(), Vec::new());
        let mut preferences: Vec<(String, Option<Value>)> = Vec::new();
        let (mut reminder, mut url, mut unsubscribed) = (false, None, false);
        for thread in threads {
            let messages = self.store.thread_messages(thread)?;
            states.extend(messages.iter().map(|message| (message.id.clone(), state_of(message))));
            let newest = messages.last().map(|message| vec![message.id.clone()]).unwrap_or_default();
            let in_inbox = pick(&messages, |message| has(message, role::INBOX));
            let elsewhere = pick(&messages, |message| {
                !has(message, role::INBOX) && shown_in(message).iter().any(|label| label != role::SENT)
            });
            let labelable =
                |message: &Message| label_account.as_ref().is_none_or(|account| *account == message.account_id);
            match action {
                Action::Archive => ids.extend(in_inbox),
                Action::Trash => ids.extend(pick(&messages, |message| !has(message, role::TRASH))),
                Action::Spam => ids.extend(pick(&messages, |message| !has(message, role::SPAM))),
                Action::Read => ids.extend(pick(&messages, |message| message.unread)),
                Action::Unread | Action::Star => ids.extend(newest),
                Action::Unstar => ids.extend(pick(&messages, |message| message.starred)),
                Action::Inbox => ids.extend(elsewhere),
                Action::Snooze if in_inbox.is_empty() => {
                    reminder = true;
                    ids.extend(newest);
                }
                Action::Snooze => ids.extend(in_inbox),
                Action::AddLabel => ids.extend(pick(&messages, |message| labelable(message) && !has(message, &label))),
                Action::RemoveLabel => ids.extend(pick(&messages, |message| has(message, &label))),
                Action::Move => {
                    ids.extend(pick(&messages, |message| labelable(message) && !has(message, &label)));
                    out_of_inbox.extend(in_inbox);
                }
                Action::Mute => {
                    ids.extend(in_inbox);
                    preferences.push((format!("muted:{thread}"), Some(json!(true))));
                }
                Action::Unmute => {
                    ids.extend(elsewhere);
                    preferences.push((format!("muted:{thread}"), None));
                }
                Action::Block => {
                    let Some(sender) = messages.iter().rev().find(|message| !me.contains(&message.from.email)) else {
                        continue;
                    };
                    preferences.push((format!("blocked:{}", sender.from.email), Some(json!(true))));
                    ids.extend(pick(&messages, |message| !has(message, role::TRASH)));
                }
                Action::Unsubscribe => {
                    let way = messages.iter().rev().find_map(|message| {
                        let way = message.unsubscribe.as_ref()?;
                        (way.url.is_some() || way.mailto.is_some()).then(|| (message.id.clone(), way.clone()))
                    });
                    let Some((id, way)) = way else { continue };
                    match way.one_click || way.mailto.is_some() {
                        true => {
                            self.mutate(Op::Unsubscribe { id })?;
                            unsubscribed = true;
                        }
                        false => url = way.url,
                    }
                }
            }
        }
        let ops: Vec<Op> = match action {
            Action::Archive | Action::Mute => vec![Op::Archive { ids }],
            Action::Trash | Action::Block => vec![Op::Trash { ids }],
            Action::Spam => vec![Op::Spam { ids }],
            Action::Read => vec![Op::SetUnread { ids, unread: false }],
            Action::Unread => vec![Op::SetUnread { ids, unread: true }],
            Action::Star => vec![Op::SetStarred { ids, starred: true }],
            Action::Unstar => vec![Op::SetStarred { ids, starred: false }],
            Action::Inbox | Action::Unmute => vec![Op::MoveToInbox { ids }],
            Action::Snooze => {
                let Some(until) = until else { bail!("Say until when.") };
                vec![Op::Snooze { ids, until }]
            }
            Action::AddLabel => vec![Op::AddLabel { ids, label: label.clone() }],
            Action::RemoveLabel => vec![Op::RemoveLabel { ids, label: label.clone() }],
            Action::Move => vec![Op::AddLabel { ids, label: label.clone() }, Op::Archive { ids: out_of_inbox }],
            Action::Unsubscribe => Vec::new(),
        };
        let ops: Vec<Op> = ops.into_iter().filter(|op| !op.ids().is_empty()).flat_map(undo::in_chunks).collect();
        if action == Action::Unsubscribe && !unsubscribed && url.is_none() {
            bail!("This mail has no way to unsubscribe.");
        }
        if ops.is_empty() && preferences.is_empty() {
            let message = unsubscribed.then(|| "Unsubscribed.".to_string());
            return Ok(ActReply { message, undo: false, url });
        }
        let touched: HashSet<&String> = ops.iter().flat_map(|op| op.ids()).collect();
        let before: Vec<(String, MessageState)> =
            touched.into_iter().filter_map(|id| Some((id.clone(), states.get(id)?.clone()))).collect();
        let mut back = undo::inverse(&before, &ops);
        for (key, _) in &preferences {
            back.push(Op::SetPreference { key: key.clone(), value: self.store.preference(key) });
        }
        for op in ops {
            self.mutate(op)?;
        }
        for (key, value) in preferences {
            self.mutate(Op::SetPreference { key, value })?;
        }
        let now = Local::now();
        let when = until.and_then(|until| chrono::TimeZone::timestamp_millis_opt(&Local, until).single());
        let name = || match self.store.label(&label) {
            Some((_, name)) => name,
            None => label.clone(),
        };
        let message = match action {
            Action::Archive => Some("Archived.".to_string()),
            Action::Trash => Some("Moved to Trash.".into()),
            Action::Spam => Some("Marked as spam.".into()),
            Action::Inbox => Some("Moved to Inbox.".into()),
            Action::Read | Action::Unread | Action::Star | Action::Unstar => None,
            Action::Snooze => when.map(|when| match reminder {
                true => format!("Reminder set for {}.", times::label(&when, &now)),
                false => format!("Snoozed until {}.", times::label(&when, &now)),
            }),
            Action::AddLabel => Some(format!("Labeled {}.", name())),
            Action::RemoveLabel => Some(format!("Removed {}.", name())),
            Action::Move => Some(format!("Moved to {}.", name())),
            Action::Mute => Some("Muted.".into()),
            Action::Unmute => Some("Unmuted.".into()),
            Action::Block => Some("Blocked.".into()),
            Action::Unsubscribe => Some("Unsubscribed.".into()),
        };
        let undo = self.remember_undo(back);
        Ok(ActReply { message, undo, url })
    }

    fn take_back(&mut self) -> Result<Value> {
        let Some((action, ops)) = self.undo.pop() else { bail!("There's nothing to undo.") };
        self.backlog.retain(|step| !matches!(step, Step::Mutate { action: pending, .. } if *pending == action));
        // Like the action, the first part goes at once and the rest follows between commands.
        let mut ops = ops.into_iter();
        if let Some(first) = ops.next() {
            self.mutate(first)?;
        }
        self.backlog.extend(ops.map(|op| Step::Mutate { action: 0, op }));
        Ok(json!({ "message": "Undone." }))
    }

    fn archive_all(
        &mut self,
        mailbox: &str,
        before: Option<i64>,
        filter: Option<Filter>,
        keep: &[String],
    ) -> Result<ActReply> {
        let ids = self.store.inbox_messages(mailbox, before, filter, keep)?;
        if ids.is_empty() {
            return Ok(ActReply::default());
        }
        let back = undo::in_chunks(Op::AddLabel { ids: ids.clone(), label: role::INBOX.into() });
        // The newest go at once; the rest follow between the app's next commands.
        let mut ops = undo::in_chunks(Op::Archive { ids }).into_iter();
        if let Some(first) = ops.next() {
            self.mutate(first)?;
        }
        let undo = self.remember_undo(back);
        let action = self.actions;
        self.backlog.extend(ops.map(|op| Step::Mutate { action, op }));
        Ok(ActReply { message: Some("Archived.".into()), undo, url: None })
    }

    fn set_account_color(&mut self, account: String, color: String) -> Result<Value> {
        if !ACCOUNT_COLORS.contains(&color.as_str()) {
            bail!("That colour doesn't exist.");
        }
        if self.store.account_address(&account).is_none() {
            bail!("That account is gone.");
        }
        self.mutate(Op::SetAccountColor { account_id: account, color })?;
        Ok(json!({}))
    }

    fn create_label(&mut self, account: String, name: &str) -> Result<Value> {
        let name = name.trim();
        if name.is_empty() {
            bail!("Give the label a name.");
        }
        if self.store.account_address(&account).is_none() {
            bail!("That account is gone.");
        }
        let label_id = new_id();
        self.mutate(Op::CreateLabel { account_id: account, label_id: label_id.clone(), name: name.to_string() })?;
        Ok(json!({ "id": label_id }))
    }

    fn preferences(&self) -> Result<Value> {
        let values = self.store.preferences()?.into_iter().map(|(key, value)| PreferenceValue { key, value }).collect();
        Ok(serde_json::to_value(Preferences { values })?)
    }

    fn set_preference(&mut self, key: String, value: Option<Value>) -> Result<Value> {
        if key.trim().is_empty() {
            bail!("A preference needs a key.");
        }
        let value = value.filter(|value| !value.is_null());
        self.mutate(Op::SetPreference { key, value })?;
        Ok(json!({}))
    }

    // ---------- drafts ----------

    /// The address a draft is written from.
    fn sender_of(&self, draft: &Draft) -> String {
        match &draft.from {
            Some(from) => from.email.clone(),
            None => self.store.account_address(&draft.account_id).unwrap_or_default(),
        }
    }

    fn new_draft(&self, account: Option<String>) -> Result<Value> {
        let accounts = self.store.accounts()?;
        let account = match account {
            Some(id) => accounts.into_iter().find(|account| account.id == id),
            None => accounts.into_iter().next(),
        };
        let Some(account) = account else { bail!("Add an account first.") };
        let draft = Draft { account_id: account.id, ..Default::default() };
        Ok(serde_json::to_value(DraftReply { id: None, draft, from: account.address })?)
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
        let mut draft = drafts::reply(message, &body, kind, &me, &Local);
        // Answer from the alias the mail was sent to.
        let account = self.store.accounts()?.into_iter().find(|account| account.id == draft.account_id);
        let recipients = &message.recipients;
        let addressed: HashSet<&str> = recipients
            .to
            .iter()
            .chain(&recipients.cc)
            .chain(&recipients.bcc)
            .map(|address| address.email.as_str())
            .collect();
        if let Some(account) = &account
            && let Some(identity) = account.identities.iter().find(|identity| {
                let email = identity.email.to_lowercase();
                addressed.contains(email.as_str()) && email != account.address.to_lowercase()
            })
        {
            draft.from = Some(Address::new(identity.name.as_deref(), &identity.email));
        }
        let from = self.sender_of(&draft);
        Ok(serde_json::to_value(DraftReply { id: None, draft, from })?)
    }

    fn save_draft(&mut self, id: Option<String>, draft: Draft) -> Result<Value> {
        let id = id.filter(|id| !id.is_empty()).unwrap_or_else(new_id);
        if draft.account_id.is_empty() {
            bail!("Say which account the draft is from.");
        }
        self.mutate(Op::SaveDraft { draft_id: id.clone(), draft: Box::new(draft) })?;
        Ok(json!({ "id": id }))
    }

    fn open_draft(&self, id: &str) -> Result<Value> {
        let Some(draft) = self.store.saved_draft(id) else { bail!("This draft is gone.") };
        let from = self.sender_of(&draft);
        Ok(serde_json::to_value(DraftReply { id: Some(id.to_string()), draft, from })?)
    }

    /// The signature for a draft: the user's for its account, else its identity's own.
    fn signature(&self, draft: &Draft) -> Option<String> {
        if let Some(Value::String(signature)) = self.store.preference(&format!("signature:{}", draft.account_id)) {
            return Some(signature);
        }
        let account = self.store.accounts().ok()?.into_iter().find(|account| account.id == draft.account_id)?;
        let identity = match &draft.from {
            Some(from) => account.identities.iter().find(|identity| identity.email.eq_ignore_ascii_case(&from.email)),
            None => account.identities.first(),
        };
        identity?.signature.clone()
    }

    fn queue_send(
        &mut self,
        mut draft: Draft,
        delay: u64,
        send_at: Option<i64>,
        remind_at: Option<i64>,
        draft_id: Option<String>,
    ) -> Result<Value> {
        if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
            bail!("Add someone to send it to.");
        }
        if draft.account_id.is_empty() {
            let Some(first) = self.store.accounts()?.into_iter().next() else { bail!("Add an account first.") };
            draft.account_id = first.id;
        }
        let original = draft.clone();
        let signature = self.signature(&draft);
        let quote = draft.quote.take();
        draft.html = Some(compose::html(&draft.text, signature.as_deref(), quote.as_deref()));
        draft.text = compose::text(&draft.text, signature.as_deref(), quote.as_deref());
        let send_at = send_at.unwrap_or_else(|| mail_protocol::now_ms() + delay as i64 * 1000);
        let op_id = new_id();
        let draft_id = draft_id.filter(|id| !id.is_empty());
        self.store.keep_original(&op_id, &original, draft_id.as_deref())?;
        self.mutate_as(&op_id, Op::Send { draft: Box::new(draft), send_at, remind_at })?;
        if let Some(draft_id) = draft_id {
            self.mutate(Op::DeleteDraft { draft_id })?;
        }
        Ok(json!({ "op_id": op_id, "send_at": send_at }))
    }

    /// Takes a send back before it goes, keeping it as a saved draft.
    fn cancel_send(&mut self, op_id: &str) -> Result<Value> {
        let Some(op @ Op::Send { .. }) = self.store.outbox_op(op_id) else { bail!("It was already sent.") };
        let original = self.store.original(op_id);
        let here_only = !self.online() || self.demo || waits_for_upload(&op);
        if here_only {
            let (_, threads) = self.store.settle(op_id, false)?;
            self.changed(Touched { threads: threads.into_iter().collect(), ..Default::default() });
        } else {
            self.mutate(Op::CancelSend { op_id: op_id.to_string() })?;
        }
        let Op::Send { draft, .. } = op else { bail!("It was already sent.") };
        let (draft, id) = self.save_back(original, *draft);
        if !here_only {
            self.cancelled.insert(op_id.to_string(), id.clone());
        }
        Ok(json!({ "draft": draft, "id": id }))
    }

    // ---------- files, printing, search ----------

    fn open_attachment(&mut self, id: u64, message: &str, index: usize) -> Option<Result<Value>> {
        let found = match self.store.message(message) {
            Ok(Some(found)) => found,
            Ok(None) => return Some(Err(anyhow!("This message is gone."))),
            Err(error) => return Some(Err(error)),
        };
        let named =
            found.attachments.iter().enumerate().filter(|(_, attachment)| !attachment.name.is_empty()).nth(index);
        let Some((position, attachment)) = named else { return Some(Err(anyhow!("This attachment is gone."))) };
        let path = self.data_dir.join("attachments").join(file_name(message)).join(file_name(&attachment.name));
        let answer = |path: &Path| json!({ "path": path.display().to_string() });
        if path.exists() {
            return Some(Ok(answer(&path)));
        }
        if self.demo {
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|_| std::fs::write(&path, format!("{} is made up for the demo.\n", attachment.name)));
            return Some(written.map(|_| answer(&path)).map_err(Into::into));
        }
        let (Some(server), Some(token)) = (self.server(), self.token()) else {
            return Some(Err(anyhow!("Sign in first.")));
        };
        let http = self.http.clone();
        let url = format!("/api/messages/{}/attachments/{position}", crate::http::encode(message));
        self.spawn(
            async move {
                let bytes = crate::http::download(&http, &server, &url, &token).await?;
                if let Some(folder) = path.parent() {
                    tokio::fs::create_dir_all(folder).await?;
                }
                let partial = path.with_extension("part");
                tokio::fs::write(&partial, bytes).await?;
                tokio::fs::rename(&partial, &path).await?;
                Ok(path)
            },
            move |core, result: Result<PathBuf>| core.reply(id, result.map(|path| answer(&path))),
        );
        None
    }

    fn print_thread(&self, thread: &str) -> Result<Value> {
        let messages = self.store.thread_messages(thread)?;
        if messages.is_empty() {
            bail!("This conversation is gone.");
        }
        let line = |addresses: &[Address]| drafts::line(addresses);
        let printed: Vec<Printed> = messages
            .iter()
            .map(|message| Printed {
                from: line(std::slice::from_ref(&message.from)),
                to: line(&message.recipients.to),
                cc: line(&message.recipients.cc),
                date: dates::quoted(message.date, &Local),
                body: self.store.body(&message.id),
                snippet: &message.snippet,
            })
            .collect();
        let subject = messages.iter().map(|message| message.subject.as_str()).find(|subject| !subject.is_empty());
        Ok(json!({ "html": html::print(subject.unwrap_or("(no subject)"), &printed) }))
    }

    /// Searches this copy on the reader, away from the loop, then asks the server too.
    fn search(&mut self, id: u64, query: String) -> Option<Result<Value>> {
        self.next_request += 1;
        let request = self.next_request;
        self.latest_search.store(request, Ordering::Relaxed);
        let (reader, latest, text) = (self.reader.clone(), self.latest_search.clone(), query.clone());
        self.spawn(
            async move {
                let search = move || {
                    // A newer search came: this one's answer would be thrown away.
                    if latest.load(Ordering::Relaxed) != request {
                        return Ok(Default::default());
                    }
                    // A search that panicked leaves the connection as good as it was.
                    let reader = reader.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    reader.search(&text, &Local::now())
                };
                tokio::task::spawn_blocking(search).await.map_err(anyhow::Error::from).and_then(|result| result)
            },
            move |core, result| {
                let result = result.map(|(threads, rows)| {
                    if core.online() && !query.trim().is_empty() {
                        core.searches.retain(|earlier, _| *earlier + 20 > request);
                        core.searches.insert(request, threads);
                        core.send(ClientMessage::Search { request, query });
                    }
                    json!({ "request": request, "rows": rows })
                });
                core.reply(id, result);
            },
        );
        None
    }

    // ---------- the server ----------

    fn link_event(&mut self, event: LinkEvent) {
        match event {
            LinkEvent::Opened => {
                let Some(token) = self.token() else { return };
                self.first_sync = self.store.meta("caught_up").is_none();
                self.send(ClientMessage::Hello { token, cursor: self.store.cursor(), protocol: PROTOCOL_VERSION });
            }
            LinkEvent::Closed(error) => {
                if self.token().is_some() {
                    self.bodies_in_flight.clear();
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
                    self.dispatch(op_id, op);
                }
                self.pump_bodies();
                if !self.refreshing.is_empty() {
                    self.send(ClientMessage::Sync { request: self.refresh });
                }
            }
            ServerMessage::Changes { accounts, labels, messages, preferences, drafts, cursor, more } => {
                let mut parts: Vec<Part> = Vec::new();
                let mut messages = messages.into_iter().peekable();
                loop {
                    parts.push(Part { messages: messages.by_ref().take(STEP).collect(), ..Default::default() });
                    if messages.peek().is_none() {
                        break;
                    }
                }
                if let Some(first) = parts.first_mut() {
                    (first.accounts, first.labels, first.preferences, first.drafts) =
                        (accounts, labels, preferences, drafts);
                }
                if let Some(last) = parts.last_mut() {
                    (last.cursor, last.more) = (Some(cursor), Some(more));
                }
                for part in parts.into_iter().rev() {
                    self.backlog.push_front(Step::Changes(Box::new(part)));
                }
            }
            ServerMessage::Applied { op_id, ok, error } => self.applied(&op_id, ok, error),
            ServerMessage::Body { message_id, body, .. } => {
                self.bodies_in_flight.remove(&message_id);
                let Some(body) = body else {
                    self.body_failed(message_id);
                    self.pump_bodies();
                    return;
                };
                self.body_failures.remove(&message_id);
                match self.store.save_body(&message_id, &body) {
                    Ok(Some(thread)) => {
                        self.changed(Touched { threads: HashSet::from([thread]), ..Default::default() })
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!("couldn't keep a body: {error:#}"),
                }
                self.pump_bodies();
            }
            ServerMessage::SearchResults { request, message_ids } => {
                let Some(mut threads) = self.searches.remove(&request) else { return };
                for thread in self.store.threads_of_messages(&message_ids) {
                    if !threads.contains(&thread) {
                        threads.push(thread);
                    }
                }
                let rows = self.store.rows_of(&threads, &Local::now()).unwrap_or_default();
                self.emit(Event::SearchResults { request, rows });
            }
            ServerMessage::Synced { request } => {
                if request == self.refresh {
                    self.refreshed(request);
                }
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

    fn apply_part(&mut self, part: Part) {
        let batch = Batch {
            accounts: &part.accounts,
            labels: &part.labels,
            messages: &part.messages,
            preferences: &part.preferences,
            drafts: &part.drafts,
            cursor: part.cursor,
        };
        match self.store.apply_changes(batch) {
            Ok(applied) => {
                self.changed(applied.touched);
                self.arrived.extend(applied.arrived);
            }
            Err(error) => {
                tracing::error!("couldn't store changes: {error:#}");
                self.emit(Event::Error { message: "Mail couldn't be saved on this device.".into() });
            }
        }
        if part.more != Some(false) {
            self.flush_soon();
            return;
        }
        // Caught up.
        self.flush();
        let arrived = std::mem::take(&mut self.arrived);
        match self.first_sync {
            true => {
                self.first_sync = false;
                self.announcing = self.ready_accounts();
                if let Err(error) = self.store.set_meta("caught_up", Some("1")) {
                    tracing::warn!("couldn't note the first sync is done: {error:#}");
                }
            }
            false => self.announce(arrived),
        }
        let missing = self.store.missing_bodies(PREFETCH).unwrap_or_default();
        self.queue_bodies(missing, false);
    }

    /// The accounts that are ready, and want their new mail announced.
    fn ready_accounts(&self) -> HashSet<String> {
        self.store
            .accounts()
            .unwrap_or_default()
            .into_iter()
            .filter(|account| account.status == "ready")
            .filter(|account| self.store.preference(&format!("notify:{}", account.id)) != Some(json!(false)))
            .map(|account| account.id)
            .collect()
    }

    /// Tells the apps about new mail of the accounts that were already caught up.
    fn announce(&mut self, arrived: Vec<Arrived>) {
        let since = mail_protocol::now_ms() - NEW_MAIL_WINDOW;
        let now_ready = self.ready_accounts();
        let ready = std::mem::replace(&mut self.announcing, now_ready);
        let messages: Vec<_> = arrived
            .into_iter()
            .filter(|arrived| arrived.date >= since && ready.contains(&arrived.mail.account_id))
            .filter(|arrived| self.announcing.contains(&arrived.mail.account_id))
            .map(|arrived| arrived.mail)
            .collect();
        if !messages.is_empty() {
            self.emit(Event::NewMail { messages });
        }
    }

    fn applied(&mut self, op_id: &str, ok: bool, error: Option<String>) {
        let original = self.store.original(op_id);
        let (op, threads) = match self.store.settle(op_id, ok) {
            Ok(settled) => settled,
            Err(error) => {
                tracing::error!("couldn't settle an op: {error:#}");
                return;
            }
        };
        if !threads.is_empty() {
            self.changed(Touched { threads: threads.into_iter().collect(), mailboxes: true, ..Default::default() });
        }
        let error = error.unwrap_or_else(|| "The server refused the change.".into());
        match op {
            Some(Op::Send { .. }) if ok => self.emit(Event::Sent { op_id: op_id.to_string() }),
            Some(Op::Send { draft, .. }) => {
                if self.cancelled.remove(op_id).is_none() {
                    let (draft, id) = self.save_back(original, *draft);
                    let draft = Box::new(draft);
                    self.emit(Event::SendFailed { op_id: op_id.to_string(), error, draft, draft_id: Some(id) });
                }
            }
            Some(Op::CancelSend { op_id: target }) if !ok => {
                // It went after all: the draft kept for it would be a copy of sent mail.
                if let Some(draft_id) = self.cancelled.remove(&target)
                    && let Err(error) = self.mutate(Op::DeleteDraft { draft_id })
                {
                    tracing::error!("couldn't delete a draft: {error:#}");
                }
                self.emit(Event::Error { message: error });
            }
            Some(_) if !ok => self.emit(Event::Error { message: error }),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core on the demo's mail, driven as an app drives it.
    struct Demo {
        handle: Handle,
        events: Arc<Mutex<Vec<Event>>>,
        next: u64,
        data: tempfile::TempDir,
    }

    impl Demo {
        fn start() -> Self {
            let data = tempfile::tempdir().unwrap();
            let events: Arc<Mutex<Vec<Event>>> = Arc::default();
            let sink = events.clone();
            let config = Config { data_dir: data.path().display().to_string(), server_url: None, demo: true };
            let handle = start(config, Arc::new(move |event| sink.lock().unwrap().push(event))).unwrap();
            Self { handle, events, next: 0, data }
        }

        async fn call(&mut self, command: Value) -> Result<Value, Value> {
            self.next += 1;
            let id = self.next;
            self.handle.send(id, serde_json::from_value(command).unwrap());
            for _ in 0..500 {
                let reply = self.events.lock().unwrap().iter().find_map(|event| match event {
                    Event::Reply { id: replied, ok, value } if *replied == id => Some((*ok, value.clone())),
                    _ => None,
                });
                if let Some((ok, value)) = reply {
                    return if ok { Ok(value) } else { Err(value) };
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("no reply to command {id}");
        }

        async fn inbox(&mut self) -> Vec<String> {
            let page = self.call(json!({ "type": "threads", "mailbox": "inbox" })).await.unwrap();
            page["rows"].as_array().unwrap().iter().map(|row| row["id"].as_str().unwrap().to_string()).collect()
        }

        /// The demo thread whose subject starts so.
        async fn thread(&mut self, subject: &str) -> String {
            let page = self.call(json!({ "type": "search", "query": subject })).await.unwrap();
            page["rows"][0]["id"].as_str().unwrap().to_string()
        }
    }

    #[tokio::test]
    async fn the_apps_server_moves_with_the_app_but_a_chosen_one_stays() {
        let data = tempfile::tempdir().unwrap();
        let status = |server_url: &str| {
            let config = Config {
                data_dir: data.path().display().to_string(),
                server_url: Some(server_url.into()),
                demo: false,
            };
            let events: Arc<Mutex<Vec<Event>>> = Arc::default();
            let sink = events.clone();
            let handle = start(config, Arc::new(move |event| sink.lock().unwrap().push(event))).unwrap();
            (handle, events)
        };
        let server = |events: &Arc<Mutex<Vec<Event>>>| {
            events.lock().unwrap().iter().find_map(|event| match event {
                Event::Reply { id: 1, value, .. } => Some(value["server"].clone()),
                _ => None,
            })
        };
        let ask = async |handle: &Handle, events: &Arc<Mutex<Vec<Event>>>| {
            handle.send(1, Command::Status);
            for _ in 0..200 {
                if let Some(found) = server(events) {
                    return found;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("no answer");
        };

        let (handle, events) = status("https://old.example");
        assert_eq!(ask(&handle, &events).await, "https://old.example");
        drop(handle);
        let (handle, events) = status("https://new.example");
        assert_eq!(ask(&handle, &events).await, "https://new.example");

        handle.send(2, Command::SetServer { url: "https://mine.example".into() });
        drop(handle);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (handle, events) = status("https://new.example");
        assert_eq!(ask(&handle, &events).await, "https://mine.example");
    }

    #[tokio::test]
    async fn boot_answers_the_screen_the_app_was_left_on() {
        let mut demo = Demo::start();
        let first = demo.call(json!({ "type": "boot" })).await.unwrap();
        assert_eq!(first["ui"]["mailbox"], "inbox");
        assert_eq!(first["status"]["signed_in"], true);
        assert!(!first["page"]["rows"].as_array().unwrap().is_empty());
        assert!(first["thread"].is_null());

        let thread = demo.thread("Dinner on Saturday").await;
        let ui = json!({
            "mailbox": "demo-home/inbox", "thread": thread, "selected": thread, "rows": 3, "list_offset": 120.5,
            "thread_offset": 300.0, "selection": [thread], "search": "dinner",
            "compose": { "draft": { "text": "hello" } }, "sidebar": true, "unfolded": ["m1"],
            "heights": { "m1": 412.5 }, "body_width": 640.0,
        });
        demo.call(json!({ "type": "save_ui", "ui": ui })).await.unwrap();
        let boot = demo.call(json!({ "type": "boot" })).await.unwrap();
        assert_eq!(boot["ui"]["mailbox"], "demo-home/inbox");
        assert_eq!(boot["ui"]["list_offset"], 120.5);
        assert_eq!(boot["ui"]["thread_offset"], 300.0);
        assert_eq!(boot["ui"]["selection"], json!([thread]));
        assert_eq!(boot["ui"]["compose"]["draft"]["text"], "hello");
        assert_eq!(boot["ui"]["unfolded"], json!(["m1"]));
        assert_eq!(boot["ui"]["heights"]["m1"], 412.5);
        assert_eq!(boot["ui"]["body_width"], 640.0);
        assert_eq!(boot["ui"]["filter"], Value::Null, "what wasn't saved has its default");
        assert!(boot["page"]["rows"].as_array().unwrap().iter().all(|row| row["account_id"] == "demo-home"));
        assert_eq!(boot["thread"]["id"], thread.as_str());
        assert!(boot["search"].as_array().unwrap().iter().any(|row| row["id"] == thread.as_str()));
        assert!(!boot["preferences"]["values"].as_array().unwrap().is_empty());

        demo.call(json!({ "type": "save_ui", "ui": { "thread": "gone" } })).await.unwrap();
        let boot = demo.call(json!({ "type": "boot" })).await.unwrap();
        assert!(boot["thread"].is_null(), "a thread that is gone is left closed");
        assert!(boot["search"].is_null());

        demo.call(json!({ "type": "sign_out" })).await.unwrap();
        let boot = demo.call(json!({ "type": "boot" })).await.unwrap();
        assert_eq!(boot["status"]["signed_in"], false);
        assert_eq!(boot["ui"]["thread"], Value::Null, "signing out forgets where the app was");
    }

    #[tokio::test]
    async fn actions_answer_with_a_toast_and_undo_takes_them_back() {
        let mut demo = Demo::start();
        demo.call(json!({ "type": "set_preference", "key": "split_inbox", "value": false })).await.unwrap();
        let first = demo.inbox().await[0].clone();
        let reply = demo.call(json!({ "type": "act", "action": "archive", "threads": [first] })).await.unwrap();
        assert_eq!(reply, json!({ "message": "Archived.", "undo": true, "url": null }));
        assert!(!demo.inbox().await.contains(&first));
        assert_eq!(demo.call(json!({ "type": "undo" })).await.unwrap(), json!({ "message": "Undone." }));
        assert_eq!(demo.inbox().await[0], first);
        assert!(demo.call(json!({ "type": "undo" })).await.is_err());

        let all = demo.call(json!({ "type": "archive_all", "mailbox": "demo-work/inbox" })).await.unwrap();
        assert_eq!(all["undo"], true);
        assert_eq!(demo.inbox().await.len(), 8);
        demo.call(json!({ "type": "undo" })).await.unwrap();
        assert_eq!(demo.inbox().await.len(), 16);
    }

    #[tokio::test]
    async fn muting_archives_and_keeps_the_thread_muted() {
        let mut demo = Demo::start();
        let thread = demo.thread("Dinner on Saturday").await;
        let reply = demo.call(json!({ "type": "act", "action": "mute", "threads": [thread] })).await.unwrap();
        assert_eq!(reply["message"], "Muted.");
        assert!(!demo.inbox().await.contains(&thread));
        let view = demo.call(json!({ "type": "open_thread", "thread": thread })).await.unwrap();
        assert_eq!(view["muted"], true);
        let preferences = demo.call(json!({ "type": "preferences" })).await.unwrap();
        let key = format!("muted:{thread}");
        assert!(preferences["values"].as_array().unwrap().iter().any(|value| value["key"] == key.as_str()));
        let told =
            demo.events.lock().unwrap().iter().any(|event| matches!(event, Event::Changed { preferences: true, .. }));
        assert!(told);

        demo.call(json!({ "type": "undo" })).await.unwrap();
        let view = demo.call(json!({ "type": "open_thread", "thread": thread })).await.unwrap();
        assert_eq!(view["muted"], false);
        assert!(demo.inbox().await.contains(&thread));
    }

    #[tokio::test]
    async fn remote_images_are_shown_unless_the_preference_turns_them_off() {
        let mut demo = Demo::start();
        let kettle = demo.thread("Autumn blends").await;
        let blocked = async |demo: &mut Demo, images: bool| {
            let view = demo.call(json!({ "type": "open_thread", "thread": kettle, "images": images })).await.unwrap();
            view["messages"][0]["blocked_images"] == true
        };
        assert!(!blocked(&mut demo, false).await);

        demo.call(json!({ "type": "set_preference", "key": "remote_images", "value": false })).await.unwrap();
        assert!(blocked(&mut demo, false).await);
        assert!(!blocked(&mut demo, true).await);
    }

    #[tokio::test]
    async fn unsubscribing_from_a_list_that_only_has_a_page_answers_it() {
        let mut demo = Demo::start();
        let kettle = demo.thread("Autumn blends").await;
        let reply = demo.call(json!({ "type": "act", "action": "unsubscribe", "threads": [kettle] })).await.unwrap();
        assert_eq!(reply["url"], "https://kettle.example/preferences");
        let orbit = demo.thread("Three launches").await;
        let reply = demo.call(json!({ "type": "act", "action": "unsubscribe", "threads": [orbit] })).await.unwrap();
        assert_eq!(reply, json!({ "message": "Unsubscribed.", "undo": false, "url": null }));
        let mum = demo.thread("Call me").await;
        assert!(demo.call(json!({ "type": "act", "action": "unsubscribe", "threads": [mum] })).await.is_err());
    }

    #[tokio::test]
    async fn sending_signs_folds_the_quote_and_cancelling_gives_the_draft_back() {
        let mut demo = Demo::start();
        let thread = demo.thread("Photos from the hike").await;
        let reply = demo.call(json!({ "type": "reply_draft", "thread": thread, "kind": "reply" })).await.unwrap();
        assert_eq!(reply["from"], "sam@example.com");
        let mut draft = reply["draft"].clone();
        assert_eq!(draft["text"], "");
        assert!(draft["quote"].as_str().unwrap().contains("Jonas Weber wrote:\n> Attached."));
        draft["text"] = json!("Yes, same time!");
        let saved = demo.call(json!({ "type": "save_draft", "draft": draft })).await.unwrap();
        let view = demo.call(json!({ "type": "open_thread", "thread": thread })).await.unwrap();
        assert_eq!(view["draft_id"], saved["id"]);

        let sent = demo
            .call(json!({ "type": "send", "draft": draft, "delay": 60, "draft_id": saved["id"], "remind_at": 99 }))
            .await
            .unwrap();
        let op_id = sent["op_id"].as_str().unwrap().to_string();
        let store = Store::open(&demo.data.path().join("mail.db")).unwrap();
        let Some(Op::Send { draft: outgoing, remind_at, .. }) = store.outbox_op(&op_id) else {
            panic!("no send waits")
        };
        assert_eq!(remind_at, Some(99));
        assert!(outgoing.text.starts_with("Yes, same time!\n\nSam\n\nOn "), "{}", outgoing.text);
        assert!(outgoing.quote.is_none());
        let html = outgoing.html.unwrap();
        assert!(html.contains("<div class=\"signature\"") && html.contains("<blockquote"), "{html}");
        assert!(store.saved_draft(saved["id"].as_str().unwrap()).is_none(), "sending deletes the saved draft");

        let cancelled = demo.call(json!({ "type": "cancel_send", "op_id": op_id })).await.unwrap();
        assert_eq!(cancelled["draft"]["text"], "Yes, same time!");
        assert!(cancelled["draft"]["quote"].is_string());
        assert_eq!(cancelled["id"], saved["id"], "kept again as the draft it was");
        assert!(store.outbox_op(&op_id).is_none());
        let drafts = demo.call(json!({ "type": "threads", "mailbox": "drafts" })).await.unwrap();
        assert!(drafts["rows"].as_array().unwrap().iter().any(|row| row["draft_id"] == saved["id"]));
    }

    #[tokio::test]
    async fn labels_made_here_are_usable_at_once() {
        let mut demo = Demo::start();
        let made =
            demo.call(json!({ "type": "create_label", "account": "demo-home", "name": "Receipts" })).await.unwrap();
        let label = made["id"].as_str().unwrap().to_string();
        let thread = demo.thread("Your order has shipped").await;
        let reply =
            demo.call(json!({ "type": "act", "action": "move", "threads": [thread], "label": label })).await.unwrap();
        assert_eq!(reply["message"], "Moved to Receipts.");
        let page =
            demo.call(json!({ "type": "threads", "mailbox": format!("demo-home/label/{label}") })).await.unwrap();
        assert_eq!(page["rows"][0]["id"], thread.as_str());
        let view = demo.call(json!({ "type": "open_thread", "thread": thread })).await.unwrap();
        assert_eq!(view["labels"], json!([{ "id": label, "name": "Receipts" }]));
    }

    #[tokio::test]
    async fn attachments_open_as_files_on_this_device() {
        let mut demo = Demo::start();
        let thread = demo.thread("Your tickets for Friday").await;
        let view = demo.call(json!({ "type": "open_thread", "thread": thread })).await.unwrap();
        let message = view["messages"][0]["id"].as_str().unwrap().to_string();
        let opened = demo.call(json!({ "type": "open_attachment", "message": message, "index": 0 })).await.unwrap();
        let path = opened["path"].as_str().unwrap();
        assert!(path.ends_with("Tickets.pdf") && std::path::Path::new(path).exists(), "{path}");
        assert!(demo.call(json!({ "type": "open_attachment", "message": message, "index": 3 })).await.is_err());
        let printed = demo.call(json!({ "type": "print_thread", "thread": thread })).await.unwrap();
        assert!(printed["html"].as_str().unwrap().contains("Coach C, seats 41 and 42"));
    }

    #[tokio::test]
    async fn a_drafts_own_id_survives_the_apps_envelope() {
        let mut demo = Demo::start();
        let page = demo.call(json!({ "type": "threads", "mailbox": "drafts" })).await.unwrap();
        let id = page["rows"][0]["draft_id"].as_str().unwrap().to_string();
        let json = json!({ "id": 7, "type": "open_draft", "draft_id": id }).to_string();
        let envelope: crate::api::Envelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope.id, 7);
        let opened = demo.call(json!({ "type": "open_draft", "draft_id": id })).await.unwrap();
        assert_eq!(opened["id"], id.as_str());
        assert_eq!(opened["draft"]["subject"], "Release notes for 4.2");
        demo.call(json!({ "type": "delete_draft", "draft_id": id })).await.unwrap();
        let page = demo.call(json!({ "type": "threads", "mailbox": "drafts" })).await.unwrap();
        assert!(page["rows"].as_array().unwrap().is_empty());
    }
}
