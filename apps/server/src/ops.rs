//! A client's change: written here at once, with a new rev, and queued for the account's worker
//! to send to the provider. An op sent twice is applied once.

use std::collections::HashSet;

use mail_protocol::{Draft, MessageState, Op};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{self, MESSAGE_COLUMNS, MessageRow, UserTx};
use crate::{AppState, hub, unsubscribe};

pub const MISSING_UPLOAD: &str = "An attachment is no longer on the server. Attach it again.";
pub const MISSING_FORWARD: &str = "The forwarded message's attachments are gone.";

pub enum Outcome {
    Done {
        ok: bool,
        error: Option<String>,
    },
    /// A send that is waiting for its time. Its outcome comes later.
    Waiting,
}

impl Outcome {
    fn ok() -> Self {
        Outcome::Done { ok: true, error: None }
    }

    fn failed(error: &str) -> Self {
        Outcome::Done { ok: false, error: Some(error.to_string()) }
    }
}

pub async fn apply(state: &AppState, user_id: Uuid, op_id: &str, op: Op) -> anyhow::Result<Outcome> {
    if let Some(done) = recorded(&state.db, user_id, op_id).await? {
        return Ok(done);
    }
    let outcome = match op {
        Op::Send { draft, send_at, remind_at } => {
            return queue_send(&state.db, user_id, op_id, &draft, send_at, remind_at).await;
        }
        Op::CancelSend { op_id: target } => cancel_send(&state.db, user_id, &target).await?,
        Op::RemoveAccount { account_id } => {
            let Ok(account_id) = account_id.parse::<Uuid>() else {
                return Ok(Outcome::failed("No such account."));
            };
            state.workers.stop(account_id);
            match db::remove_account(&state.db, user_id, account_id).await? {
                true => Outcome::ok(),
                false => Outcome::failed("No such account."),
            }
        }
        Op::Unsubscribe { id } => match unsubscribe::run(state, user_id, &id).await? {
            Ok(()) => Outcome::ok(),
            Err(error) => Outcome::failed(&error),
        },
        Op::SetPreference { key, value } => set_preference(&state.db, user_id, &key, value.as_ref()).await?,
        Op::SaveDraft { draft_id, draft } => save_draft(&state.db, user_id, &draft_id, &draft).await?,
        Op::DeleteDraft { draft_id } => {
            db::delete_draft(&state.db, user_id, &draft_id).await?;
            Outcome::ok()
        }
        Op::CreateLabel { account_id, label_id, name } => {
            create_label(state, user_id, &account_id, &label_id, &name).await?
        }
        op => change_messages(state, user_id, op).await?,
    };
    record(&state.db, user_id, op_id, &outcome).await?;
    Ok(outcome)
}

/// What a send's row says now, for a client that asks again.
pub async fn send_outcome(db: &PgPool, user_id: Uuid, op_id: &str) -> sqlx::Result<Option<Outcome>> {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT status, error FROM outgoing WHERE id = $1 AND user_id = $2")
            .bind(op_id)
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    Ok(row.map(|(status, error)| match status.as_str() {
        "sent" => Outcome::ok(),
        "failed" => Outcome::Done { ok: false, error: error.or(Some("It couldn't be sent.".into())) },
        "canceled" => Outcome::failed("Cancelled."),
        _ => Outcome::Waiting,
    }))
}

async fn recorded(db: &PgPool, user_id: Uuid, op_id: &str) -> sqlx::Result<Option<Outcome>> {
    let row: Option<(bool, Option<String>)> =
        sqlx::query_as("SELECT ok, error FROM client_ops WHERE user_id = $1 AND op_id = $2")
            .bind(user_id)
            .bind(op_id)
            .fetch_optional(db)
            .await?;
    if let Some((ok, error)) = row {
        return Ok(Some(Outcome::Done { ok, error }));
    }
    send_outcome(db, user_id, op_id).await
}

async fn record(db: &PgPool, user_id: Uuid, op_id: &str, outcome: &Outcome) -> sqlx::Result<()> {
    let Outcome::Done { ok, error } = outcome else { return Ok(()) };
    sqlx::query("INSERT INTO client_ops (user_id, op_id, ok, error) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .bind(op_id)
        .bind(ok)
        .bind(error)
        .execute(db)
        .await?;
    Ok(())
}

async fn queue_send(
    db: &PgPool,
    user_id: Uuid,
    op_id: &str,
    draft: &Draft,
    send_at: i64,
    remind_at: Option<i64>,
) -> anyhow::Result<Outcome> {
    let Some(account_id) = draft.account_id.parse::<Uuid>().ok() else {
        return Ok(Outcome::failed("No such account."));
    };
    let account = db::account(db, account_id).await?.filter(|account| account.user_id == user_id);
    if account.is_none() {
        return Ok(Outcome::failed("No such account."));
    }
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return Ok(Outcome::failed("Add someone to send it to."));
    }
    if draft.attachments.iter().any(|attachment| attachment.upload.is_none()) {
        return Ok(Outcome::failed("Wait for the attachments to upload."));
    }
    let uploads: Option<Vec<Uuid>> =
        draft.attachments.iter().map(|attachment| attachment.upload.as_deref()?.parse().ok()).collect();
    let Some(uploads) = uploads else {
        return Ok(Outcome::failed(MISSING_UPLOAD));
    };
    if !db::owns_uploads(db, user_id, &uploads).await? {
        return Ok(Outcome::failed(MISSING_UPLOAD));
    }
    if let Some(forwarded) = &draft.forward_attachments_of {
        let found = match forwarded.parse::<Uuid>() {
            Ok(id) => db::message(db, user_id, id).await?.is_some_and(|message| !message.deleted),
            Err(_) => false,
        };
        if !found {
            return Ok(Outcome::failed(MISSING_FORWARD));
        }
    }
    sqlx::query(
        "INSERT INTO outgoing (id, user_id, account_id, draft, send_at, remind_at) VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(op_id)
    .bind(user_id)
    .bind(account_id)
    .bind(json!(draft))
    .bind(db::from_millis(send_at))
    .bind(remind_at.map(db::from_millis))
    .execute(db)
    .await?;
    Ok(Outcome::Waiting)
}

async fn cancel_send(db: &PgPool, user_id: Uuid, target: &str) -> sqlx::Result<Outcome> {
    let canceled =
        sqlx::query("UPDATE outgoing SET status = 'canceled' WHERE id = $1 AND user_id = $2 AND status = 'pending'")
            .bind(target)
            .bind(user_id)
            .execute(db)
            .await?;
    if canceled.rows_affected() == 0 {
        return Ok(Outcome::failed("It was already sent."));
    }
    sqlx::query("SELECT pg_notify('changes', $1)").bind(user_id.to_string()).execute(db).await?;
    Ok(Outcome::ok())
}

async fn change_messages(state: &AppState, user_id: Uuid, op: Op) -> anyhow::Result<Outcome> {
    let ids: Vec<Uuid> = op.ids().iter().filter_map(|id| id.parse().ok()).collect();
    if let Op::AddLabel { label, .. } | Op::RemoveLabel { label, .. } = &op
        && label.parse::<Uuid>().is_err()
        && !mail_protocol::role::ALL.contains(&label.as_str())
    {
        return Ok(Outcome::failed("No such label."));
    }
    let mut tx = UserTx::begin(&state.db, user_id).await?;
    let rows: Vec<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages WHERE user_id = $1 AND id = ANY($2) AND NOT deleted FOR UPDATE"
    )))
    .bind(user_id)
    .bind(&ids)
    .fetch_all(&mut *tx.tx)
    .await?;
    let states: Vec<(Uuid, MessageState)> = rows.iter().map(|row| (row.id, row.state())).collect();
    db::apply_op(&mut tx, &states, &op).await?;
    tx.commit().await?;
    let accounts: HashSet<Uuid> = rows.iter().map(|row| row.account_id).collect();
    for account_id in accounts {
        hub::notify_ops(&state.db, account_id).await;
    }
    Ok(Outcome::ok())
}

async fn set_preference(db: &PgPool, user_id: Uuid, key: &str, value: Option<&Value>) -> sqlx::Result<Outcome> {
    if key.is_empty() || key.len() > 500 {
        return Ok(Outcome::failed("That setting can't be saved."));
    }
    db::set_preference(db, user_id, key, value).await?;
    Ok(Outcome::ok())
}

async fn save_draft(db: &PgPool, user_id: Uuid, draft_id: &str, draft: &Draft) -> sqlx::Result<Outcome> {
    if draft_id.is_empty() || draft_id.len() > 200 {
        return Ok(Outcome::failed("That draft can't be saved."));
    }
    db::save_draft(db, user_id, draft_id, draft).await?;
    Ok(Outcome::ok())
}

/// The label is used at once; the account's worker makes it at the provider before it sends
/// the ops that use it.
async fn create_label(
    state: &AppState,
    user_id: Uuid,
    account_id: &str,
    label_id: &str,
    name: &str,
) -> anyhow::Result<Outcome> {
    let (Ok(account_id), Ok(label_id)) = (account_id.parse::<Uuid>(), label_id.parse::<Uuid>()) else {
        return Ok(Outcome::failed("No such account."));
    };
    let Some(account) = db::account(&state.db, account_id).await?.filter(|account| account.user_id == user_id) else {
        return Ok(Outcome::failed("No such account."));
    };
    let name = name.trim();
    if name.is_empty() {
        return Ok(Outcome::failed("Give the label a name."));
    }
    let labels = db::labels(&state.db, account_id).await?;
    if labels.iter().any(|label| !label.deleted && label.id != label_id && label.name.eq_ignore_ascii_case(name)) {
        return Ok(Outcome::failed("There's a label with that name already."));
    }
    db::create_label(&state.db, &account, label_id, name).await?;
    hub::notify_ops(&state.db, account_id).await;
    Ok(Outcome::ok())
}
