//! What happens at a time: snoozed mail comes back to the inbox, and mail waiting to be sent
//! (undo-send, send-later) goes out.

use std::time::Duration;

use chrono::{DateTime, Utc};
use mail_protocol::{Address, Draft, Identity, MessageState, Op};
use uuid::Uuid;

use crate::db::{self, AccountRow, MESSAGE_COLUMNS, MessageRow, UserTx};
use crate::mime::File;
use crate::ops::{MISSING_FORWARD, MISSING_UPLOAD};
use crate::{AppState, hub, workers};

pub fn start(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        let stopping = state.stopping.subscribe();
        loop {
            interval.tick().await;
            if *stopping.borrow() {
                return;
            }
            if let Err(error) = wake_snoozed(&state).await {
                tracing::warn!("couldn't bring back snoozed mail: {error}");
            }
            if let Err(error) = send_due(&state).await {
                tracing::warn!("couldn't send waiting mail: {error}");
            }
        }
    });
}

async fn wake_snoozed(state: &AppState) -> sqlx::Result<()> {
    let db = &state.background;
    let users: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT user_id FROM messages WHERE snoozed_until <= now() AND NOT deleted LIMIT 100",
    )
    .fetch_all(db)
    .await?;
    for user_id in users {
        let mut tx = UserTx::begin(db, user_id).await?;
        let rows: Vec<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
             WHERE user_id = $1 AND snoozed_until <= now() AND NOT deleted FOR UPDATE SKIP LOCKED"
        )))
        .bind(user_id)
        .fetch_all(&mut *tx.tx)
        .await?;
        let states: Vec<(Uuid, MessageState)> = rows.iter().map(|row| (row.id, row.state())).collect();
        db::apply_op(&mut tx, &states, &Op::MoveToInbox { ids: Vec::new() }).await?;
        tx.commit().await?;
        let mut accounts: Vec<Uuid> = rows.iter().map(|row| row.account_id).collect();
        accounts.sort();
        accounts.dedup();
        for account_id in accounts {
            hub::notify_ops(db, account_id).await;
        }
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct Due {
    id: String,
    user_id: Uuid,
    account_id: Uuid,
    draft: sqlx::types::Json<Draft>,
    remind_at: Option<DateTime<Utc>>,
}

async fn send_due(state: &AppState) -> sqlx::Result<()> {
    let due: Vec<Due> = sqlx::query_as(
        "UPDATE outgoing SET status = 'sending' WHERE id IN (
             SELECT id FROM outgoing WHERE status = 'pending' AND send_at <= now()
             ORDER BY send_at LIMIT 20 FOR UPDATE SKIP LOCKED)
         RETURNING id, user_id, account_id, draft, remind_at",
    )
    .fetch_all(&state.background)
    .await?;
    for Due { id, user_id, account_id, draft, remind_at } in due {
        let state = state.clone();
        tokio::spawn(async move {
            let result = send(&state, account_id, &draft.0, remind_at).await;
            let (status, error) = match &result {
                Ok(()) => ("sent", None),
                Err(error) => {
                    tracing::warn!("sending {id} failed: {error:#}");
                    ("failed", Some(mail_text(error)))
                }
            };
            let saved = sqlx::query("UPDATE outgoing SET status = $2, error = $3 WHERE id = $1")
                .bind(&id)
                .bind(status)
                .bind(error)
                .execute(&state.db)
                .await;
            if let Err(error) = saved {
                tracing::error!("couldn't record how sending {id} went: {error}");
            }
            let _ = sqlx::query("SELECT pg_notify('changes', $1)").bind(user_id.to_string()).execute(&state.db).await;
            state.workers.wake(account_id);
        });
    }
    Ok(())
}

async fn send(
    state: &AppState,
    account_id: Uuid,
    draft: &Draft,
    remind_at: Option<DateTime<Utc>>,
) -> anyhow::Result<()> {
    let Some(account) = db::account(&state.db, account_id).await? else {
        anyhow::bail!("The account was removed.");
    };
    let sent = deliver(state, &account, draft).await?;
    if let (Some(at), Some(provider_id)) = (remind_at, sent)
        && let Err(error) = db::remind(&state.db, &account, &provider_id, at).await
    {
        tracing::error!("couldn't keep the reminder of a sent message: {error}");
    }
    Ok(())
}

/// Sends a draft from the account as the identity it names, with its uploads and the attachments
/// of the message it forwards. Answers the sent message's provider id, when the provider says it.
pub async fn deliver(state: &AppState, account: &AccountRow, draft: &Draft) -> anyhow::Result<Option<String>> {
    let upload_ids: Vec<Uuid> =
        draft.attachments.iter().filter_map(|attachment| attachment.upload.as_deref()?.parse().ok()).collect();
    let uploads = db::uploads(&state.db, account.user_id, &upload_ids).await?;
    let mut files = Vec::new();
    for attachment in &draft.attachments {
        let upload =
            attachment.upload.as_deref().and_then(|id| uploads.iter().find(|upload| upload.id.to_string() == id));
        let Some(upload) = upload else {
            anyhow::bail!(MISSING_UPLOAD);
        };
        files.push(File { name: upload.name.clone(), mime: upload.mime.clone(), bytes: upload.bytes.clone() });
    }
    if let Some(forwarded) = &draft.forward_attachments_of {
        files.extend(forwarded_files(state, account.user_id, forwarded).await?);
    }

    let connection = state.workers.connect(state, account).await?;
    let identities = match account.identities.is_empty() {
        true => connection.identities().await.unwrap_or_default(),
        false => account.identities.0.clone(),
    };
    let sent = connection.send(draft, &sender(draft, account, &identities), &files).await?;
    if let Err(error) = db::delete_uploads(&state.db, account.user_id, &upload_ids).await {
        tracing::warn!("couldn't delete the uploads of a sent message: {error}");
    }
    Ok(sent)
}

async fn forwarded_files(state: &AppState, user_id: Uuid, id: &str) -> anyhow::Result<Vec<File>> {
    let message = match id.parse::<Uuid>() {
        Ok(id) => db::message(&state.db, user_id, id).await?.filter(|message| !message.deleted),
        Err(_) => None,
    };
    let Some(message) = message else {
        anyhow::bail!(MISSING_FORWARD);
    };
    Ok(crate::mime::files(&workers::raw_message(state, &message).await?))
}

/// Who a draft goes out as: the identity it names, else the account's first, with its name.
fn sender(draft: &Draft, account: &AccountRow, identities: &[Identity]) -> Address {
    let named = draft
        .from
        .as_ref()
        .and_then(|from| identities.iter().find(|identity| identity.email.eq_ignore_ascii_case(&from.email)));
    match named.or(identities.first()) {
        Some(identity) => Address::new(identity.name.as_deref(), &identity.email),
        None => Address::new(None, &account.address),
    }
}

fn mail_text(error: &anyhow::Error) -> String {
    let text = error.to_string();
    match text.is_empty() {
        true => "It couldn't be sent.".into(),
        false => text,
    }
}
