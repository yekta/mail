//! Postgres. Every write to what clients sync runs in `UserTx`: it holds the user's advisory lock,
//! so that user's revs commit in the order they were taken and a cursor never skips one, and it
//! tells the user's sockets on commit.

use std::collections::HashMap;

use chrono::{DateTime, Duration, TimeZone, Utc};
use mail_protocol::{Account, Address, Attachment, Label, Message, MessageState, Provider, Recipients};
use serde_json::Value;
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub const SIGN_IN_LIFETIME: Duration = Duration::minutes(10);
pub const LINK_TICKET_LIFETIME: Duration = Duration::minutes(10);
const ACCOUNT_COLORS: [&str; 5] = ["chart-1", "chart-2", "chart-3", "chart-4", "chart-5"];

pub struct UserTx {
    pub tx: Transaction<'static, Postgres>,
    user_id: Uuid,
}

impl UserTx {
    pub async fn begin(db: &PgPool, user_id: Uuid) -> sqlx::Result<Self> {
        let mut tx = db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))").bind(user_id.to_string()).execute(&mut *tx).await?;
        Ok(Self { tx, user_id })
    }

    pub async fn commit(mut self) -> sqlx::Result<()> {
        sqlx::query("SELECT pg_notify('changes', $1)").bind(self.user_id.to_string()).execute(&mut *self.tx).await?;
        self.tx.commit().await
    }
}

pub fn millis(date: DateTime<Utc>) -> i64 {
    date.timestamp_millis()
}

pub fn from_millis(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default()
}

// ---------- users and sessions ----------

pub async fn create_user(db: &PgPool) -> sqlx::Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id) VALUES ($1)").bind(id).execute(db).await?;
    Ok(id)
}

pub async fn create_session(db: &PgPool, user_id: Uuid, token_hash: &str) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO sessions (token_hash, user_id) VALUES ($1, $2)")
        .bind(token_hash)
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn user_of_session(db: &PgPool, token_hash: &str) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar("SELECT user_id FROM sessions WHERE token_hash = $1").bind(token_hash).fetch_optional(db).await
}

pub async fn create_link_ticket(db: &PgPool, user_id: Uuid, ticket_hash: &str) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO link_tickets (ticket_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(ticket_hash)
        .bind(user_id)
        .bind(Utc::now() + LINK_TICKET_LIFETIME)
        .execute(db)
        .await?;
    Ok(())
}

/// Uses up a ticket and returns whose it was. A ticket works once.
pub async fn take_link_ticket(db: &PgPool, ticket_hash: &str) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar("DELETE FROM link_tickets WHERE ticket_hash = $1 AND expires_at > now() RETURNING user_id")
        .bind(ticket_hash)
        .fetch_optional(db)
        .await
}

// ---------- sign-ins ----------

#[derive(sqlx::FromRow)]
pub struct SignIn {
    pub id: String,
    pub challenge: String,
    pub app_state: String,
    pub google_verifier: String,
    pub nonce: String,
    pub link_user_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
}

pub struct NewSignIn<'a> {
    pub id: &'a str,
    pub challenge: &'a str,
    pub app_state: &'a str,
    pub google_verifier: &'a str,
    pub nonce: &'a str,
    pub link_user_id: Option<Uuid>,
}

const SIGN_IN_COLUMNS: &str = "id, challenge, app_state, google_verifier, nonce, link_user_id, user_id";

pub async fn create_sign_in(db: &PgPool, sign_in: &NewSignIn<'_>) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO sign_ins (id, challenge, app_state, google_verifier, nonce, link_user_id, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(sign_in.id)
    .bind(sign_in.challenge)
    .bind(sign_in.app_state)
    .bind(sign_in.google_verifier)
    .bind(sign_in.nonce)
    .bind(sign_in.link_user_id)
    .bind(Utc::now() + SIGN_IN_LIFETIME)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn pending_sign_in(db: &PgPool, id: &str) -> sqlx::Result<Option<SignIn>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SIGN_IN_COLUMNS} FROM sign_ins WHERE id = $1 AND code_hash IS NULL AND expires_at > now()"
    )))
    .bind(id)
    .fetch_optional(db)
    .await
}

pub async fn complete_sign_in(db: &PgPool, id: &str, user_id: Uuid, code_hash: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE sign_ins SET user_id = $2, code_hash = $3 WHERE id = $1")
        .bind(id)
        .bind(user_id)
        .bind(code_hash)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn take_sign_in(db: &PgPool, code_hash: &str) -> sqlx::Result<Option<SignIn>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "DELETE FROM sign_ins WHERE code_hash = $1 AND expires_at > now() RETURNING {SIGN_IN_COLUMNS}"
    )))
    .bind(code_hash)
    .fetch_optional(db)
    .await
}

pub async fn cleanup(db: &PgPool) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sign_ins WHERE expires_at < now()").execute(db).await?;
    sqlx::query("DELETE FROM link_tickets WHERE expires_at < now()").execute(db).await?;
    sqlx::query("DELETE FROM client_ops WHERE created_at < now() - interval '7 days'").execute(db).await?;
    sqlx::query("DELETE FROM outgoing WHERE status <> 'pending' AND created_at < now() - interval '7 days'")
        .execute(db)
        .await?;
    Ok(())
}

// ---------- accounts ----------

#[derive(sqlx::FromRow, Clone)]
pub struct AccountRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub address: String,
    pub login: String,
    pub credentials: Vec<u8>,
    pub sync_state: Value,
    pub status: String,
    pub color: String,
    pub rev: i64,
    pub deleted: bool,
}

const ACCOUNT_COLUMNS: &str =
    "id, user_id, provider, address, login, credentials, sync_state, status, color, rev, deleted";

impl AccountRow {
    pub fn provider(&self) -> Provider {
        Provider::parse(&self.provider).unwrap_or(Provider::Jmap)
    }

    pub fn wire(&self) -> Account {
        Account {
            id: self.id.to_string(),
            provider: self.provider(),
            address: self.address.clone(),
            status: self.status.clone(),
            color: self.color.clone(),
            deleted: self.deleted,
            rev: self.rev,
        }
    }
}

pub async fn account(db: &PgPool, id: Uuid) -> sqlx::Result<Option<AccountRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE id = $1 AND NOT deleted")))
        .bind(id)
        .fetch_optional(db)
        .await
}

pub async fn live_account_ids(db: &PgPool) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar("SELECT id FROM accounts WHERE NOT deleted AND status <> 'reauth'").fetch_all(db).await
}

pub async fn account_by_login(
    db: &PgPool,
    provider: Provider,
    address: &str,
    login: &str,
) -> sqlx::Result<Option<AccountRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE provider = $1 AND address = $2 AND login = $3 AND NOT deleted"
    )))
    .bind(provider.as_str())
    .bind(address)
    .bind(login)
    .fetch_optional(db)
    .await
}

pub async fn gmail_accounts_by_address(db: &PgPool, address: &str) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar("SELECT id FROM accounts WHERE provider = 'gmail' AND address = $1 AND NOT deleted")
        .bind(address)
        .fetch_all(db)
        .await
}

pub struct NewAccount<'a> {
    pub user_id: Uuid,
    pub provider: Provider,
    pub address: &'a str,
    pub login: &'a str,
    pub credentials: Vec<u8>,
}

/// Adds the account, or gives an existing one with the same login its new credentials.
pub async fn upsert_account(db: &PgPool, account: NewAccount<'_>) -> sqlx::Result<Uuid> {
    let mut tx = UserTx::begin(db, account.user_id).await?;
    let existing: Option<Uuid> = sqlx::query_scalar(
        "UPDATE accounts SET credentials = $5, status = 'syncing', rev = nextval('revs')
         WHERE user_id = $1 AND provider = $2 AND address = $3 AND login = $4 AND NOT deleted RETURNING id",
    )
    .bind(account.user_id)
    .bind(account.provider.as_str())
    .bind(account.address)
    .bind(account.login)
    .bind(&account.credentials)
    .fetch_optional(&mut *tx.tx)
    .await?;
    if let Some(id) = existing {
        tx.commit().await?;
        return Ok(id);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE user_id = $1")
        .bind(account.user_id)
        .fetch_one(&mut *tx.tx)
        .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO accounts (id, user_id, provider, address, login, credentials, color, rev)
         VALUES ($1, $2, $3, $4, $5, $6, $7, nextval('revs'))",
    )
    .bind(id)
    .bind(account.user_id)
    .bind(account.provider.as_str())
    .bind(account.address)
    .bind(account.login)
    .bind(&account.credentials)
    .bind(ACCOUNT_COLORS[count as usize % ACCOUNT_COLORS.len()])
    .execute(&mut *tx.tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn set_account_status(db: &PgPool, account: &AccountRow, status: &str) -> sqlx::Result<()> {
    let mut tx = UserTx::begin(db, account.user_id).await?;
    let changed = sqlx::query(
        "UPDATE accounts SET status = $2, rev = nextval('revs') WHERE id = $1 AND status <> $2 AND NOT deleted",
    )
    .bind(account.id)
    .bind(status)
    .execute(&mut *tx.tx)
    .await?;
    if changed.rows_affected() == 0 {
        return Ok(());
    }
    tx.commit().await
}

pub async fn save_sync_state(db: &PgPool, account_id: Uuid, state: &Value) -> sqlx::Result<()> {
    sqlx::query("UPDATE accounts SET sync_state = $2 WHERE id = $1").bind(account_id).bind(state).execute(db).await?;
    Ok(())
}

/// Marks the account and everything synced from it deleted, and forgets its credentials.
pub async fn remove_account(db: &PgPool, user_id: Uuid, account_id: Uuid) -> sqlx::Result<bool> {
    let mut tx = UserTx::begin(db, user_id).await?;
    let removed = sqlx::query(
        "UPDATE accounts SET deleted = true, credentials = '', rev = nextval('revs')
         WHERE id = $1 AND user_id = $2 AND NOT deleted",
    )
    .bind(account_id)
    .bind(user_id)
    .execute(&mut *tx.tx)
    .await?;
    if removed.rows_affected() == 0 {
        return Ok(false);
    }
    for table in ["labels", "messages"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE {table} SET deleted = true, rev = nextval('revs') WHERE account_id = $1 AND NOT deleted"
        )))
        .bind(account_id)
        .execute(&mut *tx.tx)
        .await?;
    }
    sqlx::query("DELETE FROM provider_ops WHERE account_id = $1").bind(account_id).execute(&mut *tx.tx).await?;
    sqlx::query("DELETE FROM bodies WHERE message_id IN (SELECT id FROM messages WHERE account_id = $1)")
        .bind(account_id)
        .execute(&mut *tx.tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

// ---------- labels ----------

#[derive(sqlx::FromRow)]
pub struct LabelRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub provider_id: String,
    pub name: String,
    pub rev: i64,
    pub deleted: bool,
}

impl LabelRow {
    pub fn wire(&self) -> Label {
        Label {
            id: self.id.to_string(),
            account_id: self.account_id.to_string(),
            name: self.name.clone(),
            deleted: self.deleted,
            rev: self.rev,
        }
    }
}

pub async fn labels(db: &PgPool, account_id: Uuid) -> sqlx::Result<Vec<LabelRow>> {
    sqlx::query_as("SELECT id, account_id, provider_id, name, rev, deleted FROM labels WHERE account_id = $1")
        .bind(account_id)
        .fetch_all(db)
        .await
}

/// Makes the account's custom labels these, and returns provider id → label id.
pub async fn sync_labels(
    db: &PgPool,
    account: &AccountRow,
    remote: &[(String, String)],
) -> sqlx::Result<HashMap<String, Uuid>> {
    let existing = labels(db, account.id).await?;
    let mut tx = UserTx::begin(db, account.user_id).await?;
    let mut ids = HashMap::new();
    for (provider_id, name) in remote {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO labels (id, user_id, account_id, provider_id, name, rev) VALUES ($1, $2, $3, $4, $5, nextval('revs'))
             ON CONFLICT (account_id, provider_id) DO UPDATE SET name = excluded.name, deleted = false, rev = nextval('revs')
             WHERE labels.name <> excluded.name OR labels.deleted
             RETURNING id",
        )
        .bind(Uuid::new_v4())
        .bind(account.user_id)
        .bind(account.id)
        .bind(provider_id)
        .bind(name)
        .fetch_optional(&mut *tx.tx)
        .await?
        .or_else(|| existing.iter().find(|label| &label.provider_id == provider_id).map(|label| label.id))
        .unwrap_or_default();
        ids.insert(provider_id.clone(), id);
    }
    for gone in existing.iter().filter(|label| !label.deleted && !ids.contains_key(&label.provider_id)) {
        sqlx::query("UPDATE labels SET deleted = true, rev = nextval('revs') WHERE id = $1")
            .bind(gone.id)
            .execute(&mut *tx.tx)
            .await?;
    }
    tx.commit().await?;
    Ok(ids)
}

// ---------- messages ----------

#[derive(sqlx::FromRow)]
pub struct MessageRow {
    pub id: Uuid,
    pub account_id: Uuid,
    pub provider_id: String,
    pub thread_id: String,
    pub from_name: Option<String>,
    pub from_email: String,
    pub recipients: Json<Recipients>,
    pub subject: String,
    pub snippet: String,
    pub date: DateTime<Utc>,
    pub unread: bool,
    pub starred: bool,
    pub labels: Vec<String>,
    pub attachments: Json<Vec<Attachment>>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub rev: i64,
    pub deleted: bool,
}

pub const MESSAGE_COLUMNS: &str = "id, account_id, provider_id, thread_id, from_name, from_email, recipients, subject, \
     snippet, date, unread, starred, labels, attachments, message_id, in_reply_to, \"references\", snoozed_until, rev, deleted";

impl MessageRow {
    pub fn wire(self) -> Message {
        Message {
            id: self.id.to_string(),
            account_id: self.account_id.to_string(),
            thread_id: self.thread_id,
            from: Address { name: self.from_name, email: self.from_email },
            recipients: self.recipients.0,
            subject: self.subject,
            snippet: self.snippet,
            date: millis(self.date),
            unread: self.unread,
            starred: self.starred,
            labels: self.labels,
            attachments: self.attachments.0,
            message_id: self.message_id,
            in_reply_to: self.in_reply_to,
            references: self.references,
            snoozed_until: self.snoozed_until.map(millis),
            deleted: self.deleted,
            rev: self.rev,
        }
    }

    pub fn state(&self) -> MessageState {
        MessageState {
            labels: self.labels.clone(),
            unread: self.unread,
            starred: self.starred,
            snoozed_until: self.snoozed_until.map(millis),
        }
    }
}

/// A message as a provider has it, its labels already turned into roles and label ids.
#[derive(Debug, Clone, Default)]
pub struct RemoteMessage {
    pub provider_id: String,
    pub thread_id: String,
    pub from: Address,
    pub recipients: Recipients,
    pub subject: String,
    pub snippet: String,
    pub date: i64,
    pub unread: bool,
    pub starred: bool,
    pub labels: Vec<String>,
    pub attachments: Vec<Attachment>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

/// Writes what the provider says. A message with changes the provider doesn't have yet is left
/// alone, so its stale state doesn't flicker over them; one that didn't change keeps its rev.
pub async fn upsert_messages(db: &PgPool, account: &AccountRow, messages: &[RemoteMessage]) -> sqlx::Result<()> {
    for chunk in messages.chunks(200) {
        let mut tx = UserTx::begin(db, account.user_id).await?;
        for message in chunk {
            let search = format!(
                "{} {} {} {}",
                message.subject,
                message.from.name.as_deref().unwrap_or_default(),
                message.from.email,
                message.snippet
            );
            sqlx::query(
                "INSERT INTO messages (id, user_id, account_id, provider_id, thread_id, from_name, from_email, recipients,
                     subject, snippet, date, unread, starred, labels, attachments, message_id, in_reply_to, \"references\",
                     search, rev)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18,
                     to_tsvector('simple', $19), nextval('revs'))
                 ON CONFLICT (account_id, provider_id) DO UPDATE SET
                     thread_id = excluded.thread_id, from_name = excluded.from_name, from_email = excluded.from_email,
                     recipients = excluded.recipients, subject = excluded.subject, snippet = excluded.snippet,
                     date = excluded.date, unread = excluded.unread, starred = excluded.starred, labels = excluded.labels,
                     attachments = CASE WHEN excluded.attachments = '[]'::jsonb THEN messages.attachments ELSE excluded.attachments END,
                     message_id = excluded.message_id, in_reply_to = excluded.in_reply_to,
                     \"references\" = excluded.\"references\", search = excluded.search, deleted = false,
                     rev = nextval('revs')
                 WHERE NOT EXISTS (SELECT 1 FROM provider_ops WHERE provider_ops.message_id = messages.id)
                     AND (messages.deleted OR messages.unread <> excluded.unread OR messages.starred <> excluded.starred
                         OR messages.labels <> excluded.labels OR messages.subject <> excluded.subject
                         OR messages.snippet <> excluded.snippet OR messages.thread_id <> excluded.thread_id
                         OR messages.date <> excluded.date)",
            )
            .bind(Uuid::new_v4())
            .bind(account.user_id)
            .bind(account.id)
            .bind(&message.provider_id)
            .bind(&message.thread_id)
            .bind(&message.from.name)
            .bind(&message.from.email)
            .bind(Json(&message.recipients))
            .bind(&message.subject)
            .bind(&message.snippet)
            .bind(from_millis(message.date))
            .bind(message.unread)
            .bind(message.starred)
            .bind(&message.labels)
            .bind(Json(&message.attachments))
            .bind(&message.message_id)
            .bind(&message.in_reply_to)
            .bind(&message.references)
            .bind(search)
            .execute(&mut *tx.tx)
            .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}

pub async fn delete_messages(db: &PgPool, account: &AccountRow, provider_ids: &[String]) -> sqlx::Result<()> {
    if provider_ids.is_empty() {
        return Ok(());
    }
    let mut tx = UserTx::begin(db, account.user_id).await?;
    sqlx::query(
        "UPDATE messages SET deleted = true, rev = nextval('revs')
         WHERE account_id = $1 AND provider_id = ANY($2) AND NOT deleted",
    )
    .bind(account.id)
    .bind(provider_ids)
    .execute(&mut *tx.tx)
    .await?;
    tx.commit().await
}

pub async fn message(db: &PgPool, user_id: Uuid, id: Uuid) -> sqlx::Result<Option<MessageRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = $1 AND user_id = $2"
    )))
    .bind(id)
    .bind(user_id)
    .fetch_optional(db)
    .await
}

pub async fn body(db: &PgPool, message_id: Uuid) -> sqlx::Result<Option<mail_protocol::Body>> {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT html, text FROM bodies WHERE message_id = $1")
            .bind(message_id)
            .fetch_optional(db)
            .await?;
    Ok(row.map(|(html, text)| mail_protocol::Body { html, text }))
}

/// Keeps a fetched body, makes its text searchable and records the attachments it has.
pub async fn save_body(
    db: &PgPool,
    message: &MessageRow,
    user_id: Uuid,
    body: &mail_protocol::Body,
    attachments: &[Attachment],
    plain: &str,
) -> sqlx::Result<()> {
    let mut tx = UserTx::begin(db, user_id).await?;
    sqlx::query(
        "INSERT INTO bodies (message_id, html, text) VALUES ($1, $2, $3)
         ON CONFLICT (message_id) DO UPDATE SET html = excluded.html, text = excluded.text, fetched_at = now()",
    )
    .bind(message.id)
    .bind(&body.html)
    .bind(&body.text)
    .execute(&mut *tx.tx)
    .await?;
    let words: String = plain.chars().take(20_000).collect();
    sqlx::query(
        "UPDATE messages SET search = to_tsvector('simple', subject || ' ' || coalesce(from_name, '') || ' ' || from_email
             || ' ' || snippet || ' ' || $2)
         WHERE id = $1",
    )
    .bind(message.id)
    .bind(words)
    .execute(&mut *tx.tx)
    .await?;
    if attachments != message.attachments.0.as_slice() {
        sqlx::query("UPDATE messages SET attachments = $2, rev = nextval('revs') WHERE id = $1")
            .bind(message.id)
            .bind(Json(attachments))
            .execute(&mut *tx.tx)
            .await?;
    }
    tx.commit().await
}

/// The newest inbox messages of the account that have no body yet.
pub async fn missing_bodies(db: &PgPool, account_id: Uuid, newest: i64) -> sqlx::Result<Vec<MessageRow>> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "WITH top AS (SELECT {MESSAGE_COLUMNS} FROM messages
             WHERE account_id = $1 AND NOT deleted AND 'inbox' = ANY(labels) ORDER BY date DESC LIMIT $2)
         SELECT top.* FROM top LEFT JOIN bodies ON bodies.message_id = top.id WHERE bodies.message_id IS NULL"
    )))
    .bind(account_id)
    .bind(newest)
    .fetch_all(db)
    .await
}

pub async fn search(db: &PgPool, user_id: Uuid, query: &str) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar(
        "SELECT id FROM messages WHERE user_id = $1 AND NOT deleted AND search @@ websearch_to_tsquery('simple', $2)
         ORDER BY date DESC LIMIT 200",
    )
    .bind(user_id)
    .bind(query)
    .fetch_all(db)
    .await
}
