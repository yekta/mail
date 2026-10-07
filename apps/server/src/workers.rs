//! One task per account owns its provider connection. It makes the labels created in the apps,
//! sends the changes clients made, syncs, fetches the bodies of the newest inbox mail, and sleeps
//! until it is woken (an op, a push from the provider) or it is time to poll again. Once a day it
//! renews Gmail's push and reads the addresses the account sends as.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{StreamExt, stream};
use mail_protocol::Op;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::AppState;
use crate::db::{self, AccountRow, MessageRow};
use crate::mime;
use crate::providers::{Batch, Connection, Reauth, Refused};

const PREFETCH: i64 = 100;
const MAX_ATTEMPTS: i32 = 5;
const DAILY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
pub struct Workers {
    running: Mutex<HashMap<Uuid, Running>>,
}

struct Running {
    wake: Arc<Notify>,
    connection: Arc<Mutex<Option<Connection>>>,
    task: JoinHandle<()>,
}

impl Workers {
    pub async fn start_all(&self, state: &AppState) {
        if !state.config.workers {
            return;
        }
        match db::live_account_ids(&state.db).await {
            Ok(ids) => ids.into_iter().for_each(|id| self.start(state, id)),
            Err(error) => tracing::error!("couldn't list the accounts to sync: {error}"),
        }
    }

    /// Starts the account's worker, or wakes it if it runs.
    pub fn start(&self, state: &AppState, account_id: Uuid) {
        if !state.config.workers {
            return;
        }
        let mut running = self.running.lock().unwrap();
        if let Some(worker) = running.get(&account_id).filter(|worker| !worker.task.is_finished()) {
            worker.wake.notify_one();
            return;
        }
        let wake = Arc::new(Notify::new());
        let connection = Arc::new(Mutex::new(None));
        let task = tokio::spawn(run(state.clone(), account_id, wake.clone(), connection.clone()));
        running.insert(account_id, Running { wake, connection, task });
    }

    pub fn stop(&self, account_id: Uuid) {
        if let Some(worker) = self.running.lock().unwrap().remove(&account_id) {
            worker.task.abort();
        }
    }

    pub fn stop_all(&self) {
        for (_, worker) in self.running.lock().unwrap().drain() {
            worker.task.abort();
        }
    }

    pub fn wake(&self, account_id: Uuid) {
        if let Some(worker) = self.running.lock().unwrap().get(&account_id) {
            worker.wake.notify_one();
        }
    }

    pub fn connection(&self, account_id: Uuid) -> Option<Connection> {
        let running = self.running.lock().unwrap();
        running.get(&account_id).and_then(|worker| worker.connection.lock().unwrap().clone())
    }

    /// The account's connection, opened now if its worker hasn't one (or runs elsewhere).
    pub async fn connect(&self, state: &AppState, account: &AccountRow) -> anyhow::Result<Connection> {
        if let Some(connection) = self.connection(account.id) {
            return Ok(connection);
        }
        Connection::open(state, account).await
    }
}

/// Waits longer after each failure in a row, so a provider that refuses isn't asked again at once.
async fn run(state: AppState, account_id: Uuid, wake: Arc<Notify>, slot: Arc<Mutex<Option<Connection>>>) {
    let mut backoff = Duration::from_secs(2);
    loop {
        let Ok(Some(account)) = db::account(&state.db, account_id).await else { return };
        match Connection::open(&state, &account).await {
            Ok(connection) => {
                *slot.lock().unwrap() = Some(connection.clone());
                if let Err(error) = work(&state, account_id, &connection, &wake, &mut backoff).await
                    && handle(&state, &account, &error).await
                {
                    return;
                }
            }
            Err(error) => {
                if handle(&state, &account, &error).await {
                    return;
                }
            }
        }
        *slot.lock().unwrap() = None;
        let _ = tokio::time::timeout(backoff, wake.notified()).await;
        backoff = (backoff * 2).min(Duration::from_secs(300));
    }
}

/// Records what went wrong. True when the worker should stop: the account needs signing in again.
async fn handle(state: &AppState, account: &AccountRow, error: &anyhow::Error) -> bool {
    if error.is::<Reauth>() {
        tracing::info!("{} needs signing in again: {error}", account.address);
        let _ = db::set_account_status(&state.db, account, "reauth").await;
        return true;
    }
    tracing::warn!("syncing {} failed: {error:#}", account.address);
    let _ = db::set_account_status(&state.db, account, "error").await;
    false
}

async fn work(
    state: &AppState,
    account_id: Uuid,
    connection: &Connection,
    wake: &Notify,
    backoff: &mut Duration,
) -> anyhow::Result<()> {
    let mut daily_at: Option<tokio::time::Instant> = None;
    loop {
        let Some(account) = db::account(&state.db, account_id).await? else { return Ok(()) };
        if daily_at.is_none_or(|at| at.elapsed() > DAILY) {
            if let Err(error) = connection.watch(state).await {
                tracing::warn!("couldn't watch {} for pushes: {error:#}", account.address);
            }
            match connection.identities().await {
                Ok(identities) => db::set_identities(&state.db, &account, &identities).await?,
                Err(error) => tracing::warn!("couldn't read who {} sends as: {error:#}", account.address),
            }
            daily_at = Some(tokio::time::Instant::now());
        }
        create_labels(state, &account, connection).await?;
        flush_ops(state, &account, connection).await?;
        let synced = connection.sync(state, &account).await?;
        db::set_account_status(&state.db, &account, "ready").await?;
        *backoff = Duration::from_secs(2);
        prefetch(state, &account, connection).await;
        let wait = if synced.backfilling { Duration::from_secs(2) } else { state.config.poll_interval };
        let _ = tokio::time::timeout(wait, wake.notified()).await;
    }
}

/// Makes the labels created in the apps at the provider, so the ops that use them can be sent.
/// One the provider refuses for good (a name it has already) is deleted; any other failure is
/// tried again on the next pass.
pub(crate) async fn create_labels(
    state: &AppState,
    account: &AccountRow,
    connection: &Connection,
) -> anyhow::Result<()> {
    let labels = db::labels(&state.db, account.id).await?;
    for label in labels.iter().filter(|label| label.provider_id.is_none() && !label.deleted) {
        match connection.create_label(&label.name).await {
            Ok(provider_id) => db::set_label_provider_id(&state.db, label.id, &provider_id).await?,
            Err(error) if error.is::<Refused>() => {
                tracing::warn!("{} refused the label {:?}: {error:#}", account.address, label.name);
                db::delete_label(&state.db, account, label.id).await?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Sends the account's waiting changes in order, alike ones together.
pub(crate) async fn flush_ops(state: &AppState, account: &AccountRow, connection: &Connection) -> anyhow::Result<()> {
    loop {
        let rows: Vec<(i64, serde_json::Value, String, i32)> = sqlx::query_as(
            "SELECT provider_ops.id, provider_ops.op, messages.provider_id, provider_ops.attempts
             FROM provider_ops JOIN messages ON messages.id = provider_ops.message_id
             WHERE provider_ops.account_id = $1 ORDER BY provider_ops.id LIMIT 500",
        )
        .bind(account.id)
        .fetch_all(&state.db)
        .await?;
        if rows.is_empty() {
            return Ok(());
        }
        let all = db::labels(&state.db, account.id).await?;
        let waiting: HashSet<String> = all
            .iter()
            .filter(|label| label.provider_id.is_none() && !label.deleted)
            .map(|label| label.id.to_string())
            .collect();
        let labels: HashMap<Uuid, String> =
            all.into_iter().filter_map(|label| Some((label.id, label.provider_id?))).collect();
        let mut index = 0;
        while index < rows.len() {
            let same: Vec<_> = rows[index..].iter().take_while(|row| row.1 == rows[index].1).collect();
            index += same.len();
            let ids: Vec<i64> = same.iter().map(|row| row.0).collect();
            let Ok(op) = serde_json::from_value::<Op>(same[0].1.clone()) else {
                delete_ops(state, &ids).await?;
                continue;
            };
            // A label made in the apps since the last pass is made at the provider first, then
            // the ops are read again.
            if let Op::AddLabel { label, .. } | Op::RemoveLabel { label, .. } = &op
                && waiting.contains(label)
            {
                create_labels(state, account, connection).await?;
                break;
            }
            let batch = Batch { op, provider_ids: same.iter().map(|row| row.2.clone()).collect() };
            match connection.apply(&batch, &labels).await {
                Ok(()) => delete_ops(state, &ids).await?,
                Err(error) if error.is::<Reauth>() => return Err(error),
                Err(error) => {
                    tracing::warn!("{} refused a change: {error:#}", account.address);
                    sqlx::query("UPDATE provider_ops SET attempts = attempts + 1 WHERE id = ANY($1)")
                        .bind(&ids)
                        .execute(&state.db)
                        .await?;
                    // Given up on: the next sync brings back what the provider has.
                    sqlx::query("DELETE FROM provider_ops WHERE id = ANY($1) AND attempts >= $2")
                        .bind(&ids)
                        .bind(MAX_ATTEMPTS)
                        .execute(&state.db)
                        .await?;
                    return Err(error);
                }
            }
        }
    }
}

async fn delete_ops(state: &AppState, ids: &[i64]) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM provider_ops WHERE id = ANY($1)").bind(ids).execute(&state.db).await?;
    Ok(())
}

async fn prefetch(state: &AppState, account: &AccountRow, connection: &Connection) {
    let Ok(missing) = db::missing_bodies(&state.db, account.id, PREFETCH).await else { return };
    stream::iter(missing)
        .for_each_concurrent(4, |message| async move {
            if let Err(error) = fetch_body(state, account, connection, &message).await {
                tracing::debug!("couldn't prefetch a body: {error:#}");
            }
        })
        .await;
}

/// A message's raw MIME, from its account's provider.
pub async fn raw_message(state: &AppState, message: &MessageRow) -> anyhow::Result<Vec<u8>> {
    let Some(account) = db::account(&state.db, message.account_id).await? else {
        anyhow::bail!("the account was removed");
    };
    let connection = state.workers.connect(state, &account).await?;
    connection.raw(&message.provider_id).await
}

pub async fn fetch_body(
    state: &AppState,
    account: &AccountRow,
    connection: &Connection,
    message: &MessageRow,
) -> anyhow::Result<mail_protocol::Body> {
    let raw = connection.raw(&message.provider_id).await?;
    let Some(parsed) = mime::parse(&raw) else {
        anyhow::bail!("the message can't be read");
    };
    db::save_body(&state.db, message, account.user_id, &parsed.body, &parsed.attachments, &parsed.plain).await?;
    Ok(parsed.body)
}
