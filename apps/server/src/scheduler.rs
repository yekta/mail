//! What happens at a time: snoozed mail comes back to the inbox, and mail waiting to be sent
//! (undo-send, send-later) goes out.

use std::time::Duration;

use mail_protocol::{Address, Draft, Op};
use serde_json::json;
use uuid::Uuid;

use crate::db::{self, MESSAGE_COLUMNS, MessageRow, UserTx};
use crate::{AppState, hub};

pub fn start(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
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
    let users: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT user_id FROM messages WHERE snoozed_until <= now() AND NOT deleted LIMIT 100",
    )
    .fetch_all(&state.db)
    .await?;
    for user_id in users {
        let mut tx = UserTx::begin(&state.db, user_id).await?;
        let rows: Vec<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages
             WHERE user_id = $1 AND snoozed_until <= now() AND NOT deleted FOR UPDATE SKIP LOCKED"
        )))
        .bind(user_id)
        .fetch_all(&mut *tx.tx)
        .await?;
        let op = Op::MoveToInbox { ids: Vec::new() };
        let mut accounts = Vec::new();
        for row in &rows {
            let mut state = row.state();
            op.apply(&mut state);
            sqlx::query("UPDATE messages SET labels = $2, snoozed_until = NULL, rev = nextval('revs') WHERE id = $1")
                .bind(row.id)
                .bind(&state.labels)
                .execute(&mut *tx.tx)
                .await?;
            sqlx::query("INSERT INTO provider_ops (account_id, message_id, op) VALUES ($1, $2, $3)")
                .bind(row.account_id)
                .bind(row.id)
                .bind(json!(op))
                .execute(&mut *tx.tx)
                .await?;
            accounts.push(row.account_id);
        }
        tx.commit().await?;
        accounts.dedup();
        for account_id in accounts {
            hub::notify_ops(&state.db, account_id).await;
        }
    }
    Ok(())
}

async fn send_due(state: &AppState) -> sqlx::Result<()> {
    let due: Vec<(String, Uuid, Uuid, sqlx::types::Json<Draft>)> = sqlx::query_as(
        "UPDATE outgoing SET status = 'sending' WHERE id IN (
             SELECT id FROM outgoing WHERE status = 'pending' AND send_at <= now()
             ORDER BY send_at LIMIT 20 FOR UPDATE SKIP LOCKED)
         RETURNING id, user_id, account_id, draft",
    )
    .fetch_all(&state.db)
    .await?;
    for (id, user_id, account_id, draft) in due {
        let state = state.clone();
        tokio::spawn(async move {
            let result = send(&state, account_id, &draft.0).await;
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

async fn send(state: &AppState, account_id: Uuid, draft: &Draft) -> anyhow::Result<()> {
    let Some(account) = db::account(&state.db, account_id).await? else {
        anyhow::bail!("The account was removed.");
    };
    let connection = state.workers.connect(state, &account).await?;
    connection.send(draft, &Address::new(None, &account.address)).await
}

fn mail_text(error: &anyhow::Error) -> String {
    let text = error.to_string();
    match text.is_empty() {
        true => "It couldn't be sent.".into(),
        false => text,
    }
}
