//! The sync socket. After `Hello` the server sends every change after the client's cursor, in
//! batches, then whatever changes next as it happens. Ops, body fetches and searches are
//! answered on the same socket.

use std::collections::HashSet;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message as Frame, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use mail_protocol::PROTOCOL_VERSION;
use mail_protocol::wire::{ClientMessage, ServerMessage};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::ops::{self, Outcome};
use crate::{AppState, api, changes, db, workers};

pub async fn socket(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| run(state, socket))
}

async fn run(state: AppState, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let hello = tokio::time::timeout(Duration::from_secs(10), stream.next()).await;
    let Ok(Some(Ok(Frame::Text(text)))) = hello else { return };
    let Ok(ClientMessage::Hello { token, cursor, protocol }) = serde_json::from_str(&text) else { return };

    let refuse = |reason: &str| ServerMessage::Refused { reason: reason.to_string() };
    let refused = match (protocol == PROTOCOL_VERSION, api::user_of_token(&state, &token).await) {
        (false, _) => Some(refuse("This app is too old or too new for the server. Update it.")),
        (true, Ok(None)) => Some(refuse("signed_out")),
        (true, Err(_)) => Some(refuse("The server can't check the session right now.")),
        (true, Ok(Some(_))) => None,
    };
    if let Some(message) = refused {
        let _ = sink.send(Frame::text(serde_json::to_string(&message).unwrap_or_default())).await;
        return;
    }
    let Ok(Some(user_id)) = api::user_of_token(&state, &token).await else { return };

    let (out, mut outbox) = mpsc::channel::<ServerMessage>(256);
    let writer = tokio::spawn(async move {
        while let Some(message) = outbox.recv().await {
            let Ok(text) = serde_json::to_string(&message) else { continue };
            if sink.send(Frame::text(text)).await.is_err() {
                return;
            }
        }
    });

    let mut changed = state.hub.subscribe(user_id);
    let mut session =
        Session { state: state.clone(), user_id, cursor, waiting_sends: HashSet::new(), caught_up: false, out };
    let _ = session.out.send(ServerMessage::Welcome { protocol: PROTOCOL_VERSION }).await;
    let mut running = session.send_changes().await;
    while running {
        tokio::select! {
            result = changed.changed() => {
                running = result.is_ok() && session.send_changes().await && session.report_sends().await;
            }
            frame = stream.next() => {
                running = match frame {
                    Some(Ok(Frame::Text(text))) => session.handle(&text).await,
                    Some(Ok(Frame::Ping(_) | Frame::Pong(_) | Frame::Binary(_))) => true,
                    _ => false,
                };
            }
        }
    }
    writer.abort();
}

struct Session {
    state: AppState,
    user_id: Uuid,
    cursor: i64,
    /// Sends this client is waiting to hear about.
    waiting_sends: HashSet<String>,
    /// The first batch goes out even when empty, so the client knows it is up to date.
    caught_up: bool,
    out: mpsc::Sender<ServerMessage>,
}

impl Session {
    async fn send(&self, message: ServerMessage) -> bool {
        self.out.send(message).await.is_ok()
    }

    async fn send_changes(&mut self) -> bool {
        loop {
            let batch = match changes::read(&self.state.db, self.user_id, self.cursor).await {
                Ok(batch) => batch,
                Err(error) => {
                    tracing::error!("couldn't read changes: {error}");
                    return false;
                }
            };
            let ServerMessage::Changes { accounts, labels, messages, cursor, more } = &batch else { return false };
            let empty = accounts.is_empty() && labels.is_empty() && messages.is_empty();
            let (cursor, more) = (*cursor, *more);
            if (!empty || !self.caught_up) && !self.send(batch).await {
                return false;
            }
            self.caught_up = true;
            self.cursor = cursor;
            if !more {
                return true;
            }
        }
    }

    async fn report_sends(&mut self) -> bool {
        let waiting: Vec<String> = self.waiting_sends.iter().cloned().collect();
        for op_id in waiting {
            let Ok(Some(Outcome::Done { ok, error })) = ops::send_outcome(&self.state.db, self.user_id, &op_id).await
            else {
                continue;
            };
            self.waiting_sends.remove(&op_id);
            if !self.send(ServerMessage::Applied { op_id, ok, error }).await {
                return false;
            }
        }
        true
    }

    async fn handle(&mut self, text: &str) -> bool {
        let message = match serde_json::from_str::<ClientMessage>(text) {
            Ok(message) => message,
            Err(error) => {
                tracing::debug!("ignored a malformed message: {error}");
                return true;
            }
        };
        match message {
            ClientMessage::Hello { .. } => true,
            ClientMessage::Ping => self.send(ServerMessage::Pong).await,
            ClientMessage::Mutate { op_id, op } => match ops::apply(&self.state, self.user_id, &op_id, op).await {
                Ok(Outcome::Done { ok, error }) => self.send(ServerMessage::Applied { op_id, ok, error }).await,
                Ok(Outcome::Waiting) => {
                    self.waiting_sends.insert(op_id);
                    true
                }
                Err(error) => {
                    tracing::error!("couldn't apply an op: {error:#}");
                    let error = Some("The server couldn't make the change.".to_string());
                    self.send(ServerMessage::Applied { op_id, ok: false, error }).await
                }
            },
            ClientMessage::Search { request, query } => {
                let message_ids = db::search(&self.state.db, self.user_id, &query).await.unwrap_or_default();
                let message_ids = message_ids.iter().map(Uuid::to_string).collect();
                self.send(ServerMessage::SearchResults { request, message_ids }).await
            }
            ClientMessage::Body { request, message_id } => {
                let (state, user_id, out) = (self.state.clone(), self.user_id, self.out.clone());
                tokio::spawn(async move {
                    let (body, error) = match body(&state, user_id, &message_id).await {
                        Ok(body) => (Some(body), None),
                        Err(error) => {
                            tracing::warn!("couldn't fetch a body: {error:#}");
                            (None, Some("The message couldn't be loaded.".to_string()))
                        }
                    };
                    let _ = out.send(ServerMessage::Body { request, message_id, body, error }).await;
                });
                true
            }
        }
    }
}

async fn body(state: &AppState, user_id: Uuid, message_id: &str) -> anyhow::Result<mail_protocol::Body> {
    let id: Uuid = message_id.parse()?;
    let Some(message) = db::message(&state.db, user_id, id).await? else {
        anyhow::bail!("no such message");
    };
    if let Some(body) = db::body(&state.db, id).await? {
        return Ok(body);
    }
    let Some(account) = db::account(&state.db, message.account_id).await? else {
        anyhow::bail!("the account was removed");
    };
    let connection = state.workers.connect(state, &account).await?;
    workers::fetch_body(state, &account, &connection, &message).await
}
