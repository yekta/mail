//! Postgres. Every write to what clients sync runs in `UserTx`: it holds the user's advisory lock,
//! so that user's revs commit in the order they were taken and a cursor never skips one, and it
//! tells the user's sockets on commit.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, TimeZone, Utc};
use mail_protocol::{
    ACCOUNT_COLORS, Account, Address, Attachment, Draft, Identity, Label, Message, MessageState, Op, Preference,
    Provider, Recipients, SavedDraft, Unsubscribe,
};
use serde_json::{Value, json};
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub const SIGN_IN_LIFETIME: Duration = Duration::minutes(10);
pub const LINK_TICKET_LIFETIME: Duration = Duration::minutes(10);

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
    // An upload a send waiting for its time or a saved draft still has stays.
    sqlx::query(
        "DELETE FROM uploads WHERE created_at < now() - interval '7 days'
             AND NOT EXISTS (SELECT 1 FROM outgoing WHERE outgoing.user_id = uploads.user_id
                 AND outgoing.status IN ('pending', 'sending')
                 AND outgoing.draft->'attachments' @> jsonb_build_array(jsonb_build_object('upload', uploads.id::text)))
             AND NOT EXISTS (SELECT 1 FROM drafts WHERE drafts.user_id = uploads.user_id AND NOT drafts.deleted
                 AND drafts.draft->'attachments' @> jsonb_build_array(jsonb_build_object('upload', uploads.id::text)))",
    )
    .execute(db)
    .await?;
    sqlx::query("DELETE FROM reminders WHERE remind_at < now() - interval '7 days'").execute(db).await?;
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
    pub identities: Json<Vec<Identity>>,
    pub rev: i64,
    pub deleted: bool,
}

pub const ACCOUNT_COLUMNS: &str =
    "id, user_id, provider, address, login, credentials, sync_state, status, color, identities, rev, deleted";

impl AccountRow {
    pub fn provider(&self) -> Provider {
        Provider::parse(&self.provider).unwrap_or(Provider::Jmap)
    }

    /// The addresses that are the user's own: the account's and its identities'.
    pub fn own_addresses(&self) -> HashSet<String> {
        let identities = self.identities.iter().map(|identity| identity.email.to_lowercase());
        identities.chain([self.address.to_lowercase()]).collect()
    }

    pub fn wire(&self) -> Account {
        Account {
            id: self.id.to_string(),
            provider: self.provider(),
            address: self.address.clone(),
            status: self.status.clone(),
            color: self.color.clone(),
            identities: self.identities.0.clone(),
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

pub async fn live_account_ids_of(db: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar("SELECT id FROM accounts WHERE user_id = $1 AND NOT deleted AND status <> 'reauth'")
        .bind(user_id)
        .fetch_all(db)
        .await
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

pub async fn set_account_color(db: &PgPool, user_id: Uuid, account_id: Uuid, color: &str) -> sqlx::Result<bool> {
    let mut tx = UserTx::begin(db, user_id).await?;
    let changed = sqlx::query(
        "UPDATE accounts SET color = $3, rev = nextval('revs') WHERE id = $1 AND user_id = $2 AND NOT deleted",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(color)
    .execute(&mut *tx.tx)
    .await?;
    tx.commit().await?;
    Ok(changed.rows_affected() > 0)
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

/// Keeps what the provider says the account sends as; its rev changes only when they did.
pub async fn set_identities(db: &PgPool, account: &AccountRow, identities: &[Identity]) -> sqlx::Result<()> {
    if identities == account.identities.0.as_slice() {
        return Ok(());
    }
    let mut tx = UserTx::begin(db, account.user_id).await?;
    sqlx::query("UPDATE accounts SET identities = $2, rev = nextval('revs') WHERE id = $1 AND NOT deleted")
        .bind(account.id)
        .bind(Json(identities))
        .execute(&mut *tx.tx)
        .await?;
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
    /// None until a label made in the apps has been made at the provider.
    pub provider_id: Option<String>,
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
        .or_else(|| existing.iter().find(|label| label.provider_id.as_ref() == Some(provider_id)).map(|label| label.id))
        .unwrap_or_default();
        ids.insert(provider_id.clone(), id);
    }
    let gone = |label: &&LabelRow| label.provider_id.as_ref().is_some_and(|id| !ids.contains_key(id));
    for gone in existing.iter().filter(|label| !label.deleted).filter(gone) {
        sqlx::query("UPDATE labels SET deleted = true, rev = nextval('revs') WHERE id = $1")
            .bind(gone.id)
            .execute(&mut *tx.tx)
            .await?;
    }
    tx.commit().await?;
    Ok(ids)
}

/// A label made in the apps. The account's worker makes it at the provider.
pub async fn create_label(db: &PgPool, account: &AccountRow, id: Uuid, name: &str) -> sqlx::Result<()> {
    let mut tx = UserTx::begin(db, account.user_id).await?;
    sqlx::query(
        "INSERT INTO labels (id, user_id, account_id, name, rev) VALUES ($1, $2, $3, $4, nextval('revs'))
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(id)
    .bind(account.user_id)
    .bind(account.id)
    .bind(name)
    .execute(&mut *tx.tx)
    .await?;
    tx.commit().await
}

pub async fn set_label_provider_id(db: &PgPool, label_id: Uuid, provider_id: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE labels SET provider_id = $2 WHERE id = $1")
        .bind(label_id)
        .bind(provider_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn delete_label(db: &PgPool, account: &AccountRow, label_id: Uuid) -> sqlx::Result<()> {
    let mut tx = UserTx::begin(db, account.user_id).await?;
    sqlx::query("UPDATE labels SET deleted = true, rev = nextval('revs') WHERE id = $1")
        .bind(label_id)
        .execute(&mut *tx.tx)
        .await?;
    tx.commit().await
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
    pub bulk: bool,
    pub unsubscribe: Option<Json<Unsubscribe>>,
    pub rev: i64,
    pub deleted: bool,
}

pub const MESSAGE_COLUMNS: &str = "id, account_id, provider_id, thread_id, from_name, from_email, recipients, subject, \
     snippet, date, unread, starred, labels, attachments, message_id, in_reply_to, \"references\", snoozed_until, bulk, \
     unsubscribe, rev, deleted";

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
            bulk: self.bulk,
            unsubscribe: self.unsubscribe.map(|unsubscribe| unsubscribe.0),
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
    pub bulk: bool,
    pub unsubscribe: Option<Unsubscribe>,
}

/// A message as an upsert left it, and whether it is new to the server.
#[derive(sqlx::FromRow)]
pub struct Written {
    pub id: Uuid,
    pub thread_id: String,
    pub from_email: String,
    pub labels: Vec<String>,
    pub unread: bool,
    pub starred: bool,
    pub inserted: bool,
}

impl Written {
    pub fn state(&self) -> MessageState {
        MessageState { labels: self.labels.clone(), unread: self.unread, starred: self.starred, snoozed_until: None }
    }
}

const UPSERT_MESSAGES: &str = "INSERT INTO messages (id, user_id, account_id, provider_id, thread_id, from_name, from_email,
         recipients, subject, snippet, date, unread, starred, labels, attachments, message_id, in_reply_to, \"references\",
         bulk, unsubscribe, search, rev)
     SELECT gen_random_uuid(), $1, $2, m.provider_id, m.thread_id, m.from_name, m.from_email, m.recipients, m.subject,
         m.snippet, timestamptz 'epoch' + m.date * interval '1 millisecond', m.unread, m.starred,
         ARRAY(SELECT jsonb_array_elements_text(m.labels)), m.attachments, m.message_id, m.in_reply_to,
         ARRAY(SELECT jsonb_array_elements_text(m.\"references\")), m.bulk, m.unsubscribe,
         message_search(m.from_name, m.from_email, m.recipients, m.subject, m.snippet), nextval('revs')
     FROM jsonb_to_recordset($3) AS m(provider_id TEXT, thread_id TEXT, from_name TEXT, from_email TEXT,
         recipients JSONB, subject TEXT, snippet TEXT, date BIGINT, unread BOOLEAN, starred BOOLEAN, labels JSONB,
         attachments JSONB, message_id TEXT, in_reply_to TEXT, \"references\" JSONB, bulk BOOLEAN, unsubscribe JSONB)
     ON CONFLICT (account_id, provider_id) DO UPDATE SET
         thread_id = excluded.thread_id, from_name = excluded.from_name, from_email = excluded.from_email,
         recipients = excluded.recipients, subject = excluded.subject, snippet = excluded.snippet,
         date = excluded.date, unread = excluded.unread, starred = excluded.starred, labels = excluded.labels,
         attachments = CASE WHEN excluded.attachments = '[]'::jsonb THEN messages.attachments ELSE excluded.attachments END,
         message_id = excluded.message_id, in_reply_to = excluded.in_reply_to,
         \"references\" = excluded.\"references\", bulk = excluded.bulk, unsubscribe = excluded.unsubscribe,
         search = CASE WHEN EXISTS (SELECT 1 FROM bodies WHERE bodies.message_id = messages.id)
             THEN messages.search ELSE excluded.search END,
         deleted = false, rev = nextval('revs')
     WHERE NOT EXISTS (SELECT 1 FROM provider_ops WHERE provider_ops.message_id = messages.id)
         AND (messages.deleted OR messages.unread <> excluded.unread OR messages.starred <> excluded.starred
             OR messages.labels <> excluded.labels OR messages.subject <> excluded.subject
             OR messages.snippet <> excluded.snippet OR messages.thread_id <> excluded.thread_id
             OR messages.date <> excluded.date OR messages.bulk <> excluded.bulk
             OR messages.unsubscribe IS DISTINCT FROM excluded.unsubscribe)
     RETURNING id, thread_id, from_email, labels, unread, starred, (xmax = 0) AS inserted";

/// Writes what the provider says, a batch per statement. A message with changes the provider
/// doesn't have yet is left alone, so its stale state doesn't flicker over them; one that didn't
/// change keeps its rev. Mail new to the server goes through the user's rules.
pub async fn upsert_messages(db: &PgPool, account: &AccountRow, messages: &[RemoteMessage]) -> sqlx::Result<()> {
    let mut seen = HashSet::new();
    let unique: Vec<&RemoteMessage> =
        messages.iter().rev().filter(|message| seen.insert(message.provider_id.as_str())).collect();
    let mut queued = false;
    for chunk in unique.chunks(500) {
        let rows: Vec<Value> = chunk
            .iter()
            .map(|message| {
                json!({
                    "provider_id": message.provider_id, "thread_id": message.thread_id,
                    "from_name": message.from.name, "from_email": message.from.email,
                    "recipients": message.recipients, "subject": message.subject, "snippet": message.snippet,
                    "date": message.date, "unread": message.unread, "starred": message.starred,
                    "labels": message.labels, "attachments": message.attachments, "message_id": message.message_id,
                    "in_reply_to": message.in_reply_to, "references": message.references, "bulk": message.bulk,
                    "unsubscribe": message.unsubscribe,
                })
            })
            .collect();
        let mut tx = UserTx::begin(db, account.user_id).await?;
        let written: Vec<Written> = sqlx::query_as(UPSERT_MESSAGES)
            .bind(account.user_id)
            .bind(account.id)
            .bind(Json(rows))
            .fetch_all(&mut *tx.tx)
            .await?;
        apply_reminders(&mut tx, account.id).await?;
        let new: Vec<Written> = written.into_iter().filter(|row| row.inserted).collect();
        queued |= crate::rules::apply(&mut tx, account, &new).await?;
        tx.commit().await?;
    }
    if queued {
        crate::hub::notify_ops(db, account.id).await;
    }
    Ok(())
}

/// Changes the messages as `op` does to the state they are in, and queues it for their providers.
pub async fn apply_op(tx: &mut UserTx, messages: &[(Uuid, MessageState)], op: &Op) -> sqlx::Result<()> {
    if messages.is_empty() {
        return Ok(());
    }
    let changed: Vec<Value> = messages
        .iter()
        .map(|(id, state)| {
            let mut state = state.clone();
            op.apply(&mut state);
            json!({ "id": id, "labels": state.labels, "unread": state.unread, "starred": state.starred,
                "snoozed_until": state.snoozed_until })
        })
        .collect();
    sqlx::query(
        "UPDATE messages SET labels = ARRAY(SELECT jsonb_array_elements_text(changed.labels)),
             unread = changed.unread, starred = changed.starred,
             snoozed_until = timestamptz 'epoch' + changed.snoozed_until * interval '1 millisecond',
             rev = nextval('revs')
         FROM jsonb_to_recordset($1)
             AS changed(id UUID, labels JSONB, unread BOOLEAN, starred BOOLEAN, snoozed_until BIGINT)
         WHERE messages.id = changed.id",
    )
    .bind(Json(changed))
    .execute(&mut *tx.tx)
    .await?;
    let ids: Vec<Uuid> = messages.iter().map(|(id, _)| *id).collect();
    sqlx::query(
        "INSERT INTO provider_ops (account_id, message_id, op) SELECT account_id, id, $2 FROM messages WHERE id = ANY($1)",
    )
    .bind(&ids)
    .bind(json!(without_ids(op)))
    .execute(&mut *tx.tx)
    .await?;
    Ok(())
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
        _ => {}
    }
    op
}

/// Snoozes a sent message until `at`, now if it was synced already, else once it is.
pub async fn remind(db: &PgPool, account: &AccountRow, provider_id: &str, at: DateTime<Utc>) -> sqlx::Result<()> {
    let mut tx = UserTx::begin(db, account.user_id).await?;
    sqlx::query(
        "INSERT INTO reminders (account_id, provider_id, remind_at) VALUES ($1, $2, $3)
         ON CONFLICT (account_id, provider_id) DO UPDATE SET remind_at = excluded.remind_at",
    )
    .bind(account.id)
    .bind(provider_id)
    .bind(at)
    .execute(&mut *tx.tx)
    .await?;
    apply_reminders(&mut tx, account.id).await?;
    tx.commit().await
}

async fn apply_reminders(tx: &mut UserTx, account_id: Uuid) -> sqlx::Result<()> {
    sqlx::query(
        "WITH due AS (
             DELETE FROM reminders USING messages
             WHERE reminders.account_id = $1 AND messages.account_id = $1
                 AND messages.provider_id = reminders.provider_id AND NOT messages.deleted
             RETURNING messages.id, reminders.remind_at)
         UPDATE messages SET snoozed_until = due.remind_at, rev = nextval('revs') FROM due WHERE messages.id = due.id",
    )
    .bind(account_id)
    .execute(&mut *tx.tx)
    .await?;
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
        "UPDATE messages SET search = message_search(from_name, from_email, recipients, subject, snippet || ' ' || $2)
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

/// Rebuilds the search of the next `batch` messages synced before it was weighted. False once
/// there are none left.
pub async fn backfill_search(db: &PgPool, batch: i64) -> sqlx::Result<bool> {
    let Some(after): Option<Uuid> = sqlx::query_scalar("SELECT after FROM search_backfill").fetch_optional(db).await?
    else {
        return Ok(false);
    };
    let last: Option<Uuid> = sqlx::query_scalar(
        "WITH batch AS (SELECT id FROM messages WHERE id > $1 ORDER BY id LIMIT $2),
         rebuilt AS (
             UPDATE messages SET search = message_search(from_name, from_email, recipients, subject, snippet || ' '
                 || coalesce((SELECT left(coalesce(text, regexp_replace(html, '<[^>]*>', ' ', 'g')), 20000)
                     FROM bodies WHERE bodies.message_id = messages.id), ''))
             FROM batch WHERE messages.id = batch.id RETURNING messages.id)
         SELECT id FROM rebuilt ORDER BY id DESC LIMIT 1",
    )
    .bind(after)
    .bind(batch)
    .fetch_optional(db)
    .await?;
    let Some(last) = last else {
        sqlx::query("DELETE FROM search_backfill").execute(db).await?;
        return Ok(false);
    };
    sqlx::query("UPDATE search_backfill SET after = $1").bind(last).execute(db).await?;
    Ok(true)
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

// ---------- preferences and drafts ----------

#[derive(sqlx::FromRow)]
pub struct PreferenceRow {
    pub key: String,
    pub value: Value,
    pub rev: i64,
    pub deleted: bool,
}

impl PreferenceRow {
    pub fn wire(self) -> Preference {
        Preference { key: self.key, value: self.value, deleted: self.deleted, rev: self.rev }
    }
}

/// Sets a preference, or removes it when `value` is None.
pub async fn set_preference(db: &PgPool, user_id: Uuid, key: &str, value: Option<&Value>) -> sqlx::Result<()> {
    let mut tx = UserTx::begin(db, user_id).await?;
    sqlx::query(
        "INSERT INTO preferences (user_id, key, value, deleted, rev) VALUES ($1, $2, $3, $4, nextval('revs'))
         ON CONFLICT (user_id, key) DO UPDATE SET value = excluded.value, deleted = excluded.deleted, rev = excluded.rev",
    )
    .bind(user_id)
    .bind(key)
    .bind(value.cloned().unwrap_or(Value::Null))
    .bind(value.is_none())
    .execute(&mut *tx.tx)
    .await?;
    tx.commit().await
}

#[derive(sqlx::FromRow)]
pub struct DraftRow {
    pub id: String,
    pub draft: Json<Draft>,
    pub updated: DateTime<Utc>,
    pub rev: i64,
    pub deleted: bool,
}

impl DraftRow {
    pub fn wire(self) -> SavedDraft {
        SavedDraft {
            id: self.id,
            draft: self.draft.0,
            updated: millis(self.updated),
            deleted: self.deleted,
            rev: self.rev,
        }
    }
}

pub async fn save_draft(db: &PgPool, user_id: Uuid, id: &str, draft: &Draft) -> sqlx::Result<()> {
    let mut tx = UserTx::begin(db, user_id).await?;
    sqlx::query(
        "INSERT INTO drafts (id, user_id, draft, rev) VALUES ($1, $2, $3, nextval('revs'))
         ON CONFLICT (user_id, id) DO UPDATE SET draft = excluded.draft, updated = now(), deleted = false, rev = excluded.rev",
    )
    .bind(id)
    .bind(user_id)
    .bind(Json(draft))
    .execute(&mut *tx.tx)
    .await?;
    tx.commit().await
}

/// Leaves a tombstone with an empty draft.
pub async fn delete_draft(db: &PgPool, user_id: Uuid, id: &str) -> sqlx::Result<()> {
    let mut tx = UserTx::begin(db, user_id).await?;
    sqlx::query(
        "UPDATE drafts SET draft = $3, deleted = true, updated = now(), rev = nextval('revs')
         WHERE user_id = $1 AND id = $2 AND NOT deleted",
    )
    .bind(user_id)
    .bind(id)
    .bind(Json(Draft::default()))
    .execute(&mut *tx.tx)
    .await?;
    tx.commit().await
}

// ---------- uploads ----------

#[derive(sqlx::FromRow)]
pub struct UploadRow {
    pub id: Uuid,
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub async fn create_upload(db: &PgPool, user_id: Uuid, name: &str, mime: &str, bytes: &[u8]) -> sqlx::Result<Uuid> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO uploads (id, user_id, name, mime, bytes) VALUES ($1, $2, $3, $4, $5)")
        .bind(id)
        .bind(user_id)
        .bind(name)
        .bind(mime)
        .bind(bytes)
        .execute(db)
        .await?;
    Ok(id)
}

/// The user's own uploads among `ids`; someone else's are left out.
pub async fn uploads(db: &PgPool, user_id: Uuid, ids: &[Uuid]) -> sqlx::Result<Vec<UploadRow>> {
    sqlx::query_as("SELECT id, name, mime, bytes FROM uploads WHERE user_id = $1 AND id = ANY($2)")
        .bind(user_id)
        .bind(ids)
        .fetch_all(db)
        .await
}

pub async fn owns_uploads(db: &PgPool, user_id: Uuid, ids: &[Uuid]) -> sqlx::Result<bool> {
    let found: i64 = sqlx::query_scalar("SELECT count(*) FROM uploads WHERE user_id = $1 AND id = ANY($2)")
        .bind(user_id)
        .bind(ids)
        .fetch_one(db)
        .await?;
    Ok(found as usize == ids.iter().collect::<HashSet<_>>().len())
}

pub async fn delete_uploads(db: &PgPool, user_id: Uuid, ids: &[Uuid]) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM uploads WHERE user_id = $1 AND id = ANY($2)").bind(user_id).bind(ids).execute(db).await?;
    Ok(())
}
