//! What changed for a user after a cursor, read in one snapshot. A batch holds at most
//! `BATCH_SIZE` messages; a full batch ends at its last message's rev, and the accounts and
//! labels it carries end there too, so the next batch starts where this one stopped.

use mail_protocol::wire::ServerMessage;
use mail_protocol::{Account, BATCH_SIZE, Label};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{AccountRow, LabelRow, MESSAGE_COLUMNS, MessageRow};

pub async fn read(db: &PgPool, user_id: Uuid, cursor: i64) -> sqlx::Result<ServerMessage> {
    let mut tx = db.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY").execute(&mut *tx).await?;

    let rows: Vec<MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages WHERE user_id = $1 AND rev > $2 ORDER BY rev LIMIT $3"
    )))
    .bind(user_id)
    .bind(cursor)
    .bind(BATCH_SIZE as i64)
    .fetch_all(&mut *tx)
    .await?;
    let full = rows.len() == BATCH_SIZE;
    let upper = match (full, rows.last()) {
        (true, Some(last)) => last.rev,
        _ => i64::MAX,
    };

    let accounts: Vec<AccountRow> = sqlx::query_as(
        "SELECT id, user_id, provider, address, login, credentials, sync_state, status, color, rev, deleted
         FROM accounts WHERE user_id = $1 AND rev > $2 AND rev <= $3 ORDER BY rev",
    )
    .bind(user_id)
    .bind(cursor)
    .bind(upper)
    .fetch_all(&mut *tx)
    .await?;
    let labels: Vec<LabelRow> = sqlx::query_as(
        "SELECT id, account_id, provider_id, name, rev, deleted
         FROM labels WHERE user_id = $1 AND rev > $2 AND rev <= $3 ORDER BY rev",
    )
    .bind(user_id)
    .bind(cursor)
    .bind(upper)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let highest = rows
        .iter()
        .map(|row| row.rev)
        .chain(accounts.iter().map(|row| row.rev))
        .chain(labels.iter().map(|row| row.rev))
        .max()
        .unwrap_or(cursor);
    let accounts: Vec<Account> = accounts.iter().map(AccountRow::wire).collect();
    let labels: Vec<Label> = labels.iter().map(LabelRow::wire).collect();
    Ok(ServerMessage::Changes {
        accounts,
        labels,
        messages: rows.into_iter().map(MessageRow::wire).collect(),
        cursor: if full { upper } else { highest.max(cursor) },
        more: full,
    })
}
