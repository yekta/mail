//! A client's change: written here at once, with a new rev, and queued for the account's worker
//! to send to the provider. An op sent twice is applied once.

use std::collections::HashSet;

use mail_protocol::Op;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{self, MESSAGE_COLUMNS, MessageRow, UserTx};
use crate::{AppState, hub};

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
        Op::Send { draft, send_at } => return queue_send(&state.db, user_id, op_id, &draft, send_at).await,
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
    draft: &mail_protocol::Draft,
    send_at: i64,
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
    sqlx::query(
        "INSERT INTO outgoing (id, user_id, account_id, draft, send_at) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(op_id)
    .bind(user_id)
    .bind(account_id)
    .bind(json!(draft))
    .bind(db::from_millis(send_at))
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
    let stored = json!(without_ids(&op));
    let mut accounts = HashSet::new();
    for row in &rows {
        let mut changed = row.state();
        op.apply(&mut changed);
        sqlx::query(
            "UPDATE messages SET labels = $2, unread = $3, starred = $4, snoozed_until = $5, rev = nextval('revs')
             WHERE id = $1",
        )
        .bind(row.id)
        .bind(&changed.labels)
        .bind(changed.unread)
        .bind(changed.starred)
        .bind(changed.snoozed_until.map(db::from_millis))
        .execute(&mut *tx.tx)
        .await?;
        sqlx::query("INSERT INTO provider_ops (account_id, message_id, op) VALUES ($1, $2, $3)")
            .bind(row.account_id)
            .bind(row.id)
            .bind(&stored)
            .execute(&mut *tx.tx)
            .await?;
        accounts.insert(row.account_id);
    }
    tx.commit().await?;
    for account_id in accounts {
        hub::notify_ops(&state.db, account_id).await;
    }
    Ok(Outcome::ok())
}

/// The op as the worker keeps it: one row per message, so the ids are left out.
fn without_ids(op: &Op) -> Op {
    let mut op = op.clone();
    match &mut op {
        Op::SetUnread { ids, .. }
        | Op::SetStarred { ids, .. }
        | Op::Archive { ids }
        | Op::MoveToInbox { ids }
        | Op::Trash { ids }
        | Op::Spam { ids }
        | Op::AddLabel { ids, .. }
        | Op::RemoveLabel { ids, .. }
        | Op::Snooze { ids, .. } => ids.clear(),
        Op::Send { .. } | Op::CancelSend { .. } | Op::RemoveAccount { .. } => {}
    }
    op
}
