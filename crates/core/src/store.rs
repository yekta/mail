//! The client's SQLite copy of its mail. Messages are as the server sent them, with the changes
//! the user made since on top: `bases` keeps the server's state of a message while an op on it
//! waits in the outbox, so a later change from the server is rebased under the op.
//!
//! `threads`, `thread_labels` (with a split inbox's splits), `thread_people` and `counts` are kept
//! from the messages as they are written, so a list, a person's threads and every mailbox's
//! counts are each one indexed read.
//!
//! The copy is a cache: when `SCHEMA_VERSION` changes it is made again, empty, and synced from
//! the start, keeping the server and the session.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, TimeZone};
use mail_protocol::{
    Account, Address, Attachment, Body, Draft, Identity, Label, Message, MessageState, Op, Preference, Recipients,
    SavedDraft, role,
};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction, params, params_from_iter};
use serde::Deserialize;
use serde_json::Value;

use crate::api::{AccountView, Filter, LabelChip, Mailbox, NewMail, Person, SplitTab, ThreadPage, ThreadRow};
use crate::render::{dates, drafts, rows};
use crate::search::{self, Condition, Query};

const SCHEMA_VERSION: &str = "3";

const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE accounts (id TEXT PRIMARY KEY, provider TEXT NOT NULL, address TEXT NOT NULL, status TEXT NOT NULL,
    color TEXT NOT NULL, identities TEXT NOT NULL);
CREATE TABLE labels (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, name TEXT NOT NULL);
CREATE TABLE messages (
    id TEXT NOT NULL UNIQUE, account_id TEXT NOT NULL, thread TEXT NOT NULL, thread_id TEXT NOT NULL,
    from_name TEXT, from_email TEXT NOT NULL, recipients TEXT NOT NULL, subject TEXT NOT NULL,
    snippet TEXT NOT NULL, date INTEGER NOT NULL, unread INTEGER NOT NULL, starred INTEGER NOT NULL,
    labels TEXT NOT NULL, attachments TEXT NOT NULL, message_id TEXT, in_reply_to TEXT,
    refs TEXT NOT NULL, snoozed_until INTEGER, bulk INTEGER NOT NULL, unsubscribe TEXT,
    search_id INTEGER NOT NULL UNIQUE);
CREATE INDEX messages_by_thread ON messages(thread, date);
CREATE INDEX messages_by_account ON messages(account_id);
CREATE TABLE threads (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, last_date INTEGER NOT NULL,
    subject TEXT NOT NULL, snippet TEXT NOT NULL, senders TEXT NOT NULL, count INTEGER NOT NULL,
    unread INTEGER NOT NULL, starred INTEGER NOT NULL, attachments INTEGER NOT NULL, snoozed INTEGER NOT NULL);
CREATE INDEX threads_by_account ON threads(account_id);
CREATE TABLE thread_labels (thread TEXT NOT NULL, label TEXT NOT NULL, account_id TEXT NOT NULL,
    last_date INTEGER NOT NULL, unread INTEGER NOT NULL, starred INTEGER NOT NULL, PRIMARY KEY (thread, label));
CREATE INDEX thread_labels_by_date ON thread_labels(label, last_date DESC);
CREATE INDEX thread_labels_by_account ON thread_labels(label, account_id, last_date DESC);
CREATE INDEX thread_labels_unread ON thread_labels(label, last_date DESC) WHERE unread > 0;
CREATE INDEX thread_labels_starred ON thread_labels(label, last_date DESC) WHERE starred > 0;
CREATE TABLE counts (label TEXT NOT NULL, account_id TEXT NOT NULL, total INTEGER NOT NULL, unread INTEGER NOT NULL,
    starred INTEGER NOT NULL, PRIMARY KEY (label, account_id));
CREATE TABLE thread_people (thread TEXT NOT NULL, email TEXT NOT NULL, last_date INTEGER NOT NULL,
    PRIMARY KEY (thread, email));
CREATE INDEX thread_people_by_email ON thread_people(email, last_date DESC);
CREATE TABLE contacts (email TEXT PRIMARY KEY, name TEXT, sent INTEGER NOT NULL, received INTEGER NOT NULL,
    last INTEGER NOT NULL);
CREATE INDEX contacts_by_rank ON contacts((sent * 4 + received) DESC, last DESC);
CREATE TABLE contact_words (word TEXT NOT NULL, email TEXT NOT NULL, PRIMARY KEY (word, email)) WITHOUT ROWID;
CREATE TABLE preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE saved_drafts (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, thread TEXT, draft TEXT NOT NULL,
    updated INTEGER NOT NULL);
CREATE INDEX saved_drafts_by_thread ON saved_drafts(thread, updated DESC);
CREATE TABLE originals (op_id TEXT PRIMARY KEY, draft TEXT NOT NULL, draft_id TEXT);
CREATE TABLE bodies (message_id TEXT PRIMARY KEY, html TEXT, text TEXT);
CREATE TABLE outbox (op_id TEXT PRIMARY KEY, op TEXT NOT NULL, created INTEGER NOT NULL);
CREATE TABLE bases (message_id TEXT PRIMARY KEY, state TEXT NOT NULL);
CREATE VIRTUAL TABLE search USING fts5(subject, sender, recipients, snippet, body,
    tokenize = 'unicode61 remove_diacritics 2');
";

const MESSAGE_COLUMNS: &str = "id, account_id, thread_id, from_name, from_email, recipients, subject, snippet, date, \
     unread, starred, labels, attachments, message_id, in_reply_to, refs, snoozed_until, bulk, unsubscribe";

const ROW_COLUMNS: &str = "t.id, t.account_id, a.color, t.senders, t.subject, t.snippet, t.last_date, t.unread, \
     t.starred, t.attachments, t.snoozed";

/// The unified mailboxes, in the sidebar's order.
pub const MAILBOXES: [(&str, &str, &str); 9] = [
    ("inbox", "Inbox", "inbox"),
    (UNREAD, "Unread", "mail"),
    ("starred", "Starred", "star"),
    ("snoozed", "Snoozed", "clock"),
    ("sent", "Sent", "send"),
    ("drafts", "Drafts", "file"),
    ("archive", "Archive", "archive"),
    ("spam", "Spam", "shield-alert"),
    ("trash", "Trash", "trash"),
];

/// The inbox's unread threads, with the ones read while it is on screen (see `thread_page`).
const UNREAD: &str = "unread";

/// How many threads a search finds at most.
const SEARCH_LIMIT: usize = 200;

pub struct Store {
    db: Connection,
    path: PathBuf,
}

pub struct LocalAccount {
    pub id: String,
    pub provider: String,
    pub address: String,
    pub status: String,
    pub color: String,
    pub identities: Vec<Identity>,
}

/// What a write changed, for the apps to reload.
#[derive(Debug, Default)]
pub struct Touched {
    pub threads: HashSet<String>,
    pub mailboxes: bool,
    pub preferences: bool,
}

impl Touched {
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty() && !self.mailboxes && !self.preferences
    }

    pub fn merge(&mut self, other: Touched) {
        self.threads.extend(other.threads);
        self.mailboxes |= other.mailboxes;
        self.preferences |= other.preferences;
    }
}

/// One batch of changes from the server. `cursor` is stored with it when given.
#[derive(Default)]
pub struct Batch<'a> {
    pub accounts: &'a [Account],
    pub labels: &'a [Label],
    pub messages: &'a [Message],
    pub preferences: &'a [Preference],
    pub drafts: &'a [SavedDraft],
    pub cursor: Option<i64>,
}

/// A message that arrived in an inbox unread, not from the user.
pub struct Arrived {
    pub date: i64,
    pub mail: NewMail,
}

#[derive(Default)]
pub struct Applied {
    pub touched: Touched,
    pub arrived: Vec<Arrived>,
}

/// A custom split of the inbox, from its `split:<id>` preference.
#[derive(Debug, Clone, Deserialize)]
pub struct Split {
    #[serde(skip)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub from: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub order: i64,
}

impl Split {
    /// Whether a message (its sender and labels) puts its thread in this split. The user's own
    /// mail is no sender of it.
    fn matches(&self, sender: &str, labels: &[String], me: &HashSet<String>) -> bool {
        let from = !me.contains(sender)
            && self.from.iter().map(|pattern| pattern.trim().to_lowercase()).any(|pattern| {
                match pattern.starts_with('@') {
                    true => sender.ends_with(&pattern),
                    false => sender == pattern,
                }
            });
        from || self.label.as_ref().is_some_and(|label| labels.contains(label))
    }
}

pub fn is_split_key(key: &str) -> bool {
    key == "split_inbox" || key.starts_with("split:")
}

/// What recomputing a thread needs to know besides its messages.
struct Context {
    me: HashSet<String>,
    /// None while Split Inbox is off.
    splits: Option<Vec<Split>>,
}

pub fn thread_key(account_id: &str, thread_id: &str) -> String {
    format!("{account_id}:{thread_id}")
}

/// `inbox`, `<account>/inbox` or `<account>/label/<label>` as its label and account.
pub fn parse_mailbox(mailbox: &str) -> Option<(String, Option<String>)> {
    let parts: Vec<&str> = mailbox.splitn(3, '/').collect();
    match parts.as_slice() {
        [label] => Some((label.to_string(), None)),
        [account, label] => Some((label.to_string(), Some(account.to_string()))),
        [account, "label", label] => Some((label.to_string(), Some(account.to_string()))),
        _ => None,
    }
}

/// The label and account a mailbox's threads are kept under. With Split Inbox on, the inbox
/// shows its Important split; with it off, a split shows the whole inbox.
fn resolve(mailbox: &str, split: bool) -> Option<(String, Option<String>)> {
    let (label, account) = parse_mailbox(mailbox)?;
    let label = match split {
        false if label.starts_with("inbox:") => role::INBOX.to_string(),
        true if label == role::INBOX => "inbox:important".to_string(),
        _ => label,
    };
    Some((label, account))
}

/// Unread is the inbox, narrowed to its unread threads and those of `kept`: the label to read,
/// and `kept` as JSON when it is Unread.
fn unread_of(label: String, kept: &[String]) -> (String, Option<String>) {
    if label != UNREAD {
        return (label, None);
    }
    (role::INBOX.to_string(), Some(serde_json::json!(kept).to_string()))
}

fn filter_sql(filter: Option<Filter>) -> &'static str {
    match filter {
        None => "",
        Some(Filter::Unread) => "AND l.unread > 0",
        Some(Filter::Starred) => "AND l.starred > 0",
    }
}

/// Unread's condition, with its kept threads as the JSON parameter `?index`.
fn unread_sql(index: usize) -> String {
    format!("AND (l.unread > 0 OR l.thread IN (SELECT value FROM json_each(?{index})))")
}

fn message_from_row(row: &rusqlite::Row) -> rusqlite::Result<Message> {
    let json = |index: usize| -> rusqlite::Result<String> { row.get(index) };
    let unsubscribe: Option<String> = row.get(18)?;
    Ok(Message {
        id: row.get(0)?,
        account_id: row.get(1)?,
        thread_id: row.get(2)?,
        from: Address { name: row.get(3)?, email: row.get(4)? },
        recipients: serde_json::from_str::<Recipients>(&json(5)?).unwrap_or_default(),
        subject: row.get(6)?,
        snippet: row.get(7)?,
        date: row.get(8)?,
        unread: row.get(9)?,
        starred: row.get(10)?,
        labels: serde_json::from_str(&json(11)?).unwrap_or_default(),
        attachments: serde_json::from_str::<Vec<Attachment>>(&json(12)?).unwrap_or_default(),
        message_id: row.get(13)?,
        in_reply_to: row.get(14)?,
        references: serde_json::from_str(&json(15)?).unwrap_or_default(),
        snoozed_until: row.get(16)?,
        bulk: row.get(17)?,
        unsubscribe: unsubscribe.and_then(|text| serde_json::from_str(&text).ok()),
        deleted: false,
        rev: 0,
    })
}

pub fn state_of(message: &Message) -> MessageState {
    MessageState {
        labels: message.labels.clone(),
        unread: message.unread,
        starred: message.starred,
        snoozed_until: message.snoozed_until,
    }
}

/// The mailboxes a message shows in: its labels, and the ones that come from its state.
pub fn shown_in(message: &Message) -> Vec<String> {
    let has = |label: &str| message.labels.iter().any(|existing| existing == label);
    if has(role::TRASH) {
        return vec![role::TRASH.into()];
    }
    if has(role::SPAM) {
        return vec![role::SPAM.into()];
    }
    let mut labels = message.labels.clone();
    if message.starred {
        labels.push("starred".into());
    }
    if message.snoozed_until.is_some() {
        labels.push("snoozed".into());
    }
    let only_sent = message.labels.iter().all(|label| label == role::SENT) && has(role::SENT);
    if !has(role::INBOX) && !has(role::DRAFTS) && !only_sent && message.snoozed_until.is_none() {
        labels.push("archive".into());
    }
    labels
}

fn connect(path: &Path, flags: OpenFlags) -> Result<Connection> {
    let db = Connection::open_with_flags(path, flags)?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "NORMAL")?;
    db.pragma_update(None, "cache_size", -32_000)?;
    db.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(db)
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db = connect(path, OpenFlags::default())?;
        let meta = |key: &str| -> Option<String> {
            db.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0)).optional().ok().flatten()
        };
        if meta("schema").as_deref() == Some(SCHEMA_VERSION) {
            return Ok(Self { db, path: path.to_path_buf() });
        }
        let kept: Vec<(&str, String)> = ["server", "server_chosen", "token", "ui"]
            .into_iter()
            .filter_map(|key| meta(key).map(|value| (key, value)))
            .collect();
        // What the user did that the server doesn't have yet survives the rebuild.
        let outbox: Vec<(String, String, i64)> = db
            .prepare("SELECT op_id, op, created FROM outbox")
            .and_then(|mut statement| {
                statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.collect()
            })
            .unwrap_or_default();
        let originals: Vec<(String, String, Option<String>)> = db
            .prepare("SELECT op_id, draft, draft_id FROM originals")
            .or_else(|_| db.prepare("SELECT op_id, draft, NULL FROM originals"))
            .and_then(|mut statement| {
                statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?.collect()
            })
            .unwrap_or_default();
        drop(db);
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_owned();
            file.push(suffix);
            let _ = std::fs::remove_file(file);
        }
        let db = connect(path, OpenFlags::default())?;
        db.execute_batch(SCHEMA)?;
        db.execute("INSERT INTO meta (key, value) VALUES ('schema', ?1)", [SCHEMA_VERSION])?;
        for (key, value) in kept {
            db.execute("INSERT INTO meta (key, value) VALUES (?1, ?2)", [key, value.as_str()])?;
        }
        for (op_id, op, created) in outbox {
            db.execute("INSERT INTO outbox (op_id, op, created) VALUES (?1, ?2, ?3)", params![op_id, op, created])?;
        }
        for (op_id, draft, draft_id) in originals {
            db.execute(
                "INSERT INTO originals (op_id, draft, draft_id) VALUES (?1, ?2, ?3)",
                params![op_id, draft, draft_id],
            )?;
        }
        let mut store = Self { db, path: path.to_path_buf() };
        // Ops on messages show again as their messages arrive; the others show now.
        let tx = store.db.transaction()?;
        for (_, op) in pending_ops(&tx)? {
            local_effect(&tx, &op)?;
        }
        tx.commit()?;
        Ok(store)
    }

    /// A second, read-only connection, for reads that may take a while away from the loop.
    pub fn reader(&self) -> Result<Reader> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
        Ok(Reader { db: connect(&self.path, flags)? })
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.db.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0)).optional().ok().flatten()
    }

    pub fn set_meta(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => self.db.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", [key, value])?,
            None => self.db.execute("DELETE FROM meta WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    pub fn cursor(&self) -> i64 {
        self.meta("cursor").and_then(|cursor| cursor.parse().ok()).unwrap_or(0)
    }

    /// Forgets all mail, for signing out. The server URL stays.
    pub fn clear(&self) -> Result<()> {
        self.db.execute_batch(
            "DELETE FROM accounts; DELETE FROM labels; DELETE FROM messages; DELETE FROM threads;
             DELETE FROM thread_labels; DELETE FROM counts; DELETE FROM thread_people; DELETE FROM contacts;
             DELETE FROM contact_words; DELETE FROM preferences; DELETE FROM saved_drafts; DELETE FROM originals;
             DELETE FROM bodies; DELETE FROM outbox; DELETE FROM bases; DELETE FROM search;
             DELETE FROM meta WHERE key IN ('token', 'cursor', 'caught_up');",
        )?;
        Ok(())
    }

    /// The user's own addresses: the accounts' and their identities'.
    pub fn me(&self) -> HashSet<String> {
        me_in(&self.db).unwrap_or_default()
    }

    pub fn account_address(&self, id: &str) -> Option<String> {
        self.db
            .query_row("SELECT address FROM accounts WHERE id = ?1", [id], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    // ---------- applying what the server sends ----------

    /// Applies one batch of changes, and its cursor when given, all at once.
    pub fn apply_changes(&mut self, batch: Batch) -> Result<Applied> {
        let tx = self.db.transaction()?;
        let mut applied = Applied::default();
        let touched = &mut applied.touched;
        let pending = pending_ops(&tx)?;
        for account in batch.accounts {
            touched.mailboxes = true;
            if account.deleted {
                remove_account(&tx, &account.id, &mut touched.threads)?;
                continue;
            }
            // A colour chosen here and not yet confirmed by the server wins.
            let color = pending
                .iter()
                .find_map(|(_, op)| match op {
                    Op::SetAccountColor { account_id, color } if *account_id == account.id => Some(color.as_str()),
                    _ => None,
                })
                .unwrap_or(&account.color);
            tx.execute(
                "INSERT INTO accounts (id, provider, address, status, color, identities) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (id) DO UPDATE SET provider = excluded.provider, address = excluded.address,
                     status = excluded.status, color = excluded.color, identities = excluded.identities",
                params![
                    account.id,
                    account.provider.as_str(),
                    account.address,
                    account.status,
                    color,
                    serde_json::to_string(&account.identities)?
                ],
            )?;
        }
        for label in batch.labels {
            touched.mailboxes = true;
            match label.deleted {
                true => tx.execute("DELETE FROM labels WHERE id = ?1", [&label.id])?,
                false => tx.execute(
                    "INSERT INTO labels (id, account_id, name) VALUES (?1, ?2, ?3)
                     ON CONFLICT (id) DO UPDATE SET account_id = excluded.account_id, name = excluded.name",
                    params![label.id, label.account_id, label.name],
                )?,
            };
        }
        let mut resplit = false;
        for preference in batch.preferences {
            // What the user changed here and the server hasn't confirmed yet wins.
            let waiting =
                pending.iter().any(|(_, op)| matches!(op, Op::SetPreference { key, .. } if *key == preference.key));
            if waiting {
                continue;
            }
            let value = (!preference.deleted).then_some(&preference.value);
            set_preference(&tx, &preference.key, value)?;
            touched.preferences = true;
            resplit |= is_split_key(&preference.key);
        }
        for saved in batch.drafts {
            let waiting = pending.iter().any(|(_, op)| match op {
                Op::SaveDraft { draft_id, .. } | Op::DeleteDraft { draft_id } => *draft_id == saved.id,
                _ => false,
            });
            if waiting {
                continue;
            }
            let draft = (!saved.deleted).then_some(&saved.draft);
            touched.threads.extend(save_draft(&tx, &saved.id, draft, saved.updated)?);
            touched.mailboxes = true;
        }
        let context = context(&tx)?;
        let waiting = waiting_by_message(&pending);
        if !batch.messages.is_empty() {
            touched.mailboxes = true;
        }
        for message in batch.messages {
            if message.deleted {
                if let Some(thread) = delete_message(&tx, &message.id)? {
                    touched.threads.insert(thread);
                }
                continue;
            }
            let mut message = message.clone();
            if let Some(ops) = waiting.get(message.id.as_str()) {
                let server = state_of(&message);
                tx.execute(
                    "INSERT OR REPLACE INTO bases (message_id, state) VALUES (?1, ?2)",
                    params![message.id, serde_json::to_string(&server)?],
                )?;
                let mut state = server;
                for op in ops {
                    op.apply(&mut state);
                }
                set_state(&mut message, state);
            }
            let written = write_message(&tx, &message, &context.me)?;
            touched.threads.extend(written.moved_from);
            let thread = thread_key(&message.account_id, &message.thread_id);
            let arrived = written.new
                && message.unread
                && message.labels.iter().any(|label| label == role::INBOX)
                && !context.me.contains(&message.from.email);
            if arrived {
                applied.arrived.push(Arrived {
                    date: message.date,
                    mail: NewMail {
                        thread: thread.clone(),
                        account_id: message.account_id.clone(),
                        from: message.from.display().to_string(),
                        subject: message.subject.clone(),
                        snippet: message.snippet.clone(),
                    },
                });
            }
            touched.threads.insert(thread);
        }
        if let Some(cursor) = batch.cursor {
            tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('cursor', ?1)", [cursor.to_string()])?;
        }
        for thread in &touched.threads {
            recompute(&tx, thread, &context)?;
        }
        if resplit {
            touched.mailboxes = true;
            resplit_inbox(&tx, &context)?;
        }
        tx.commit()?;
        Ok(applied)
    }

    // ---------- ops made here ----------

    /// Shows an op at once and keeps it for the server.
    pub fn apply_local(&mut self, op_id: &str, op: &Op) -> Result<Touched> {
        let tx = self.db.transaction()?;
        // A newer value of a preference, a draft or an account's colour replaces one still waiting.
        let replaces = |earlier: &Op| match (earlier, op) {
            (Op::SetPreference { key, .. }, Op::SetPreference { key: new, .. }) => key == new,
            (Op::SetAccountColor { account_id, .. }, Op::SetAccountColor { account_id: new, .. }) => account_id == new,
            (Op::SaveDraft { draft_id, .. } | Op::DeleteDraft { draft_id }, Op::SaveDraft { draft_id: new, .. })
            | (Op::SaveDraft { draft_id, .. } | Op::DeleteDraft { draft_id }, Op::DeleteDraft { draft_id: new }) => {
                draft_id == new
            }
            _ => false,
        };
        if matches!(
            op,
            Op::SetPreference { .. } | Op::SetAccountColor { .. } | Op::SaveDraft { .. } | Op::DeleteDraft { .. }
        ) {
            for (earlier, _) in pending_ops(&tx)?.into_iter().filter(|(_, earlier)| replaces(earlier)) {
                tx.execute("DELETE FROM outbox WHERE op_id = ?1", [earlier])?;
            }
        }
        tx.execute(
            "INSERT INTO outbox (op_id, op, created) VALUES (?1, ?2, ?3)",
            params![op_id, serde_json::to_string(op)?, mail_protocol::now_ms()],
        )?;
        let mut touched = local_effect(&tx, op)?;
        let context = context(&tx)?;
        let mut threads = HashSet::new();
        for id in op.ids() {
            let Some(mut message) = message_in(&tx, id)? else { continue };
            tx.execute(
                "INSERT OR IGNORE INTO bases (message_id, state) VALUES (?1, ?2)",
                params![id, serde_json::to_string(&state_of(&message))?],
            )?;
            let mut state = state_of(&message);
            op.apply(&mut state);
            set_state(&mut message, state);
            write_message(&tx, &message, &context.me)?;
            threads.insert(thread_key(&message.account_id, &message.thread_id));
        }
        for thread in &threads {
            recompute(&tx, thread, &context)?;
        }
        if !threads.is_empty() {
            touched.mailboxes = true;
        }
        touched.threads.extend(threads);
        tx.commit()?;
        Ok(touched)
    }

    pub fn outbox(&self) -> Result<Vec<(String, Op)>> {
        pending_ops(&self.db)
    }

    pub fn outbox_op(&self, op_id: &str) -> Option<Op> {
        let text: Option<String> = self
            .db
            .query_row("SELECT op FROM outbox WHERE op_id = ?1", [op_id], |row| row.get(0))
            .optional()
            .ok()
            .flatten();
        text.and_then(|text| serde_json::from_str(&text).ok())
    }

    /// Notes the ids the server gave a send's files, in place of their paths. Returns the op, when
    /// it still waits.
    pub fn set_uploads(&self, op_id: &str, uploads: &[(usize, String)]) -> Result<Option<Op>> {
        let Some(mut op) = self.outbox_op(op_id) else { return Ok(None) };
        let Op::Send { draft, .. } = &mut op else { return Ok(Some(op)) };
        for (index, upload) in uploads {
            let Some(attachment) = draft.attachments.get_mut(*index) else { continue };
            attachment.upload = Some(upload.clone());
            attachment.path = None;
        }
        self.db.execute("UPDATE outbox SET op = ?2 WHERE op_id = ?1", params![op_id, serde_json::to_string(&op)?])?;
        Ok(Some(op))
    }

    /// Keeps a draft as the user wrote it, before the core added to it to send it, and the saved
    /// draft it came from.
    pub fn keep_original(&self, op_id: &str, draft: &Draft, draft_id: Option<&str>) -> Result<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO originals (op_id, draft, draft_id) VALUES (?1, ?2, ?3)",
            params![op_id, serde_json::to_string(draft)?, draft_id],
        )?;
        Ok(())
    }

    /// A send's draft as the user wrote it, and the saved draft it came from.
    pub fn original(&self, op_id: &str) -> Option<(Draft, Option<String>)> {
        let found: Option<(String, Option<String>)> = self
            .db
            .query_row("SELECT draft, draft_id FROM originals WHERE op_id = ?1", [op_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()
            .ok()
            .flatten();
        let (draft, draft_id) = found?;
        Some((serde_json::from_str(&draft).ok()?, draft_id))
    }

    /// The server answered an op. Accepted, it leaves the outbox; refused, its messages go back
    /// to the server's state with the other waiting ops on top. Returns the op and the threads
    /// that changed.
    pub fn settle(&mut self, op_id: &str, ok: bool) -> Result<(Option<Op>, Vec<String>)> {
        let tx = self.db.transaction()?;
        let op: Option<String> =
            tx.query_row("SELECT op FROM outbox WHERE op_id = ?1", [op_id], |row| row.get(0)).optional()?;
        let Some(op) = op.and_then(|op| serde_json::from_str::<Op>(&op).ok()) else {
            return Ok((None, Vec::new()));
        };
        tx.execute("DELETE FROM outbox WHERE op_id = ?1", [op_id])?;
        tx.execute("DELETE FROM originals WHERE op_id = ?1", [op_id])?;
        let pending = pending_ops(&tx)?;
        let waiting = waiting_by_message(&pending);
        let mut touched = HashSet::new();
        let context = context(&tx)?;
        for id in op.ids() {
            let still_waiting = waiting.get(id.as_str()).cloned().unwrap_or_default();
            let base: Option<String> =
                tx.query_row("SELECT state FROM bases WHERE message_id = ?1", [id], |row| row.get(0)).optional()?;
            if still_waiting.is_empty() {
                tx.execute("DELETE FROM bases WHERE message_id = ?1", [id])?;
            }
            if ok {
                continue;
            }
            let (Some(base), Some(mut message)) =
                (base.and_then(|base| serde_json::from_str::<MessageState>(&base).ok()), message_in(&tx, id)?)
            else {
                continue;
            };
            let mut state = base;
            for op in still_waiting {
                op.apply(&mut state);
            }
            set_state(&mut message, state);
            write_message(&tx, &message, &context.me)?;
            touched.insert(thread_key(&message.account_id, &message.thread_id));
        }
        for thread in &touched {
            recompute(&tx, thread, &context)?;
        }
        tx.commit()?;
        Ok((Some(op), touched.into_iter().collect()))
    }

    // ---------- reading ----------

    pub fn accounts(&self) -> Result<Vec<LocalAccount>> {
        let mut statement = self
            .db
            .prepare_cached("SELECT id, provider, address, status, color, identities FROM accounts ORDER BY rowid")?;
        let rows = statement.query_map([], |row| {
            Ok(LocalAccount {
                id: row.get(0)?,
                provider: row.get(1)?,
                address: row.get(2)?,
                status: row.get(3)?,
                color: row.get(4)?,
                identities: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mailboxes(&self) -> Result<(Vec<Mailbox>, Vec<AccountView>)> {
        let mut unread: HashMap<(String, String), u32> = HashMap::new();
        {
            let mut statement =
                self.db.prepare_cached("SELECT label, account_id, unread FROM counts WHERE unread > 0")?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u32>(2)?)))?;
            for row in rows {
                let (label, account, count) = row?;
                unread.insert((label, account), count);
            }
        }
        let counted = |label: &str| if label == UNREAD { role::INBOX.to_string() } else { label.to_string() };
        let total = |label: &str| {
            let label = counted(label);
            unread.iter().filter(|((name, _), _)| *name == label).map(|(_, count)| count).sum()
        };
        let unified = MAILBOXES
            .iter()
            .map(|(id, name, symbol)| Mailbox {
                id: id.to_string(),
                name: name.to_string(),
                symbol: symbol.to_string(),
                unread: total(id),
            })
            .collect();
        let mut accounts = Vec::new();
        for LocalAccount { id, provider, address, status, color, identities } in self.accounts()? {
            let mut mailboxes: Vec<Mailbox> = MAILBOXES
                .iter()
                .map(|(mailbox, name, symbol)| Mailbox {
                    id: format!("{id}/{mailbox}"),
                    name: name.to_string(),
                    symbol: symbol.to_string(),
                    unread: unread.get(&(counted(mailbox), id.clone())).copied().unwrap_or(0),
                })
                .collect();
            let mut statement = self
                .db
                .prepare_cached("SELECT id, name FROM labels WHERE account_id = ?1 ORDER BY name COLLATE NOCASE")?;
            let labels = statement.query_map([&id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            for label in labels {
                let (label_id, name) = label?;
                mailboxes.push(Mailbox {
                    id: format!("{id}/label/{label_id}"),
                    unread: unread.get(&(label_id.clone(), id.clone())).copied().unwrap_or(0),
                    name,
                    symbol: "tag".into(),
                });
            }
            accounts.push(AccountView { id, address, provider, color, status, identities, mailboxes });
        }
        Ok((unified, accounts))
    }

    /// A page of a mailbox (see `api.rs` for the ids), with its total and, over a split inbox,
    /// the splits. Unread keeps the threads of `kept` that are still in the inbox, read or not.
    pub fn thread_page<Tz: TimeZone>(
        &self,
        mailbox: &str,
        offset: usize,
        limit: usize,
        filter: Option<Filter>,
        kept: &[String],
        now: &DateTime<Tz>,
    ) -> Result<ThreadPage>
    where
        Tz::Offset: std::fmt::Display,
    {
        let splits = splits_in(&self.db)?;
        let Some((label, account)) = resolve(mailbox, splits.is_some()) else {
            return Ok(ThreadPage { rows: Vec::new(), total: 0, splits: Vec::new() });
        };
        let (label, unread) = unread_of(label, kept);
        let drafts = match label == role::DRAFTS && filter.is_none() {
            true => self.draft_rows(account.as_deref(), now)?,
            false => Vec::new(),
        };
        let (from, count) = match drafts.is_empty() {
            true => (offset, limit),
            false => (0, offset + limit),
        };
        let mut rows = self.list(&label, account.as_deref(), from, count, filter, unread.as_deref(), now)?;
        let total = self.count(&label, account.as_deref(), filter, unread.as_deref())? + drafts.len();
        if !drafts.is_empty() {
            rows.extend(drafts);
            rows.sort_by_key(|row| std::cmp::Reverse(row.timestamp));
            rows = rows.into_iter().skip(offset).take(limit).collect();
        }
        let tabs = match &splits {
            Some(splits) if unread.is_none() && (label == role::INBOX || label.starts_with("inbox:")) => {
                let mut tabs = vec![("important".to_string(), "Important".to_string())];
                tabs.extend(splits.iter().map(|split| (split.id.clone(), split.name.clone())));
                tabs.push(("other".into(), "Other".into()));
                let prefix = account.as_ref().map(|account| format!("{account}/")).unwrap_or_default();
                let mut list = Vec::new();
                for (id, name) in tabs {
                    let label = format!("inbox:{id}");
                    let total = self.count(&label, account.as_deref(), None, None)? as u32;
                    let unread = self.count(&label, account.as_deref(), Some(Filter::Unread), None)? as u32;
                    list.push(SplitTab { mailbox: format!("{prefix}{label}"), name, unread, total });
                }
                list
            }
            _ => Vec::new(),
        };
        Ok(ThreadPage { rows, total, splits: tabs })
    }

    /// `unread`: in Unread, the threads kept in it as JSON (see `unread_of`).
    #[allow(clippy::too_many_arguments)]
    fn list<Tz: TimeZone>(
        &self,
        label: &str,
        account: Option<&str>,
        offset: usize,
        limit: usize,
        filter: Option<Filter>,
        unread: Option<&str>,
        now: &DateTime<Tz>,
    ) -> Result<Vec<ThreadRow>>
    where
        Tz::Offset: std::fmt::Display,
    {
        let by_account = if account.is_some() { "AND l.account_id = ?2" } else { "AND ?2 IS NULL" };
        let by_filter = filter_sql(filter);
        let by_unread = if unread.is_some() { unread_sql(5) } else { "AND ?5 IS NULL".into() };
        let sql = format!(
            "SELECT {ROW_COLUMNS} FROM thread_labels l JOIN threads t ON t.id = l.thread JOIN accounts a ON a.id = t.account_id
             WHERE l.label = ?1 {by_account} {by_filter} {by_unread} ORDER BY l.last_date DESC LIMIT ?3 OFFSET ?4"
        );
        let mut statement = self.db.prepare_cached(&sql)?;
        let rows = statement.query_map(params![label, account, limit as i64, offset as i64, unread], |row| {
            let mut found = row_from(row, now)?;
            found.timestamp = row.get(6)?;
            Ok(found)
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// How many threads a mailbox has (unread or starred ones with a filter), from `counts`;
    /// Unread's are counted from its threads.
    fn count(&self, label: &str, account: Option<&str>, filter: Option<Filter>, unread: Option<&str>) -> Result<usize> {
        if let Some(kept) = unread {
            let by_account = if account.is_some() { "AND l.account_id = ?2" } else { "AND ?2 IS NULL" };
            let sql = format!(
                "SELECT COUNT(*) FROM thread_labels l WHERE l.label = ?1 {by_account} {} {}",
                filter_sql(filter),
                unread_sql(3)
            );
            let count: i64 =
                self.db.prepare_cached(&sql)?.query_row(params![label, account, kept], |row| row.get(0))?;
            return Ok(count.max(0) as usize);
        }
        let column = match filter {
            None => "total",
            Some(Filter::Unread) => "unread",
            Some(Filter::Starred) => "starred",
        };
        let by_account = if account.is_some() { "AND account_id = ?2" } else { "AND ?2 IS NULL" };
        let sql = format!("SELECT COALESCE(SUM({column}), 0) FROM counts WHERE label = ?1 {by_account}");
        let count: i64 = self.db.prepare_cached(&sql)?.query_row(params![label, account], |row| row.get(0))?;
        Ok(count.max(0) as usize)
    }

    /// The saved drafts, as rows of the Drafts mailbox.
    fn draft_rows<Tz: TimeZone>(&self, account: Option<&str>, now: &DateTime<Tz>) -> Result<Vec<ThreadRow>>
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut statement = self.db.prepare_cached(
            "SELECT d.id, d.account_id, a.color, d.draft, d.updated FROM saved_drafts d JOIN accounts a ON a.id = d.account_id
             WHERE ?1 IS NULL OR d.account_id = ?1 ORDER BY d.updated DESC",
        )?;
        let rows = statement.query_map([account], |row| {
            Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?))
        })?;
        let mut list = Vec::new();
        for row in rows {
            let (id, account_id, color, draft, updated) = row?;
            let Ok(draft) = serde_json::from_str::<Draft>(&draft) else { continue };
            let to: Vec<&str> = draft.to.iter().chain(&draft.cc).map(Address::display).collect();
            list.push(ThreadRow {
                id: format!("draft:{id}"),
                account_id,
                color,
                senders: if to.is_empty() { "(no recipients)".into() } else { to.join(", ") },
                subject: if draft.subject.trim().is_empty() { "(no subject)".into() } else { draft.subject.clone() },
                snippet: draft.text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect(),
                date: dates::short(updated, now),
                timestamp: updated,
                unread: false,
                starred: false,
                attachment: !draft.attachments.is_empty(),
                snoozed: false,
                draft_id: Some(id),
            });
        }
        Ok(list)
    }

    /// The rows of these threads, newest first.
    pub fn rows_of<Tz: TimeZone>(&self, threads: &[String], now: &DateTime<Tz>) -> Result<Vec<ThreadRow>>
    where
        Tz::Offset: std::fmt::Display,
    {
        rows_of(&self.db, threads, now)
    }

    pub fn thread_messages(&self, thread: &str) -> Result<Vec<Message>> {
        let mut statement = self
            .db
            .prepare_cached(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE thread = ?1 ORDER BY date"))?;
        let rows = statement.query_map([thread], message_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn message(&self, id: &str) -> Result<Option<Message>> {
        Ok(message_in(&self.db, id)?)
    }

    pub fn thread_of_message(&self, id: &str) -> Option<String> {
        self.db.query_row("SELECT thread FROM messages WHERE id = ?1", [id], |row| row.get(0)).optional().ok().flatten()
    }

    pub fn body(&self, message_id: &str) -> Option<Body> {
        self.db
            .query_row("SELECT html, text FROM bodies WHERE message_id = ?1", [message_id], |row| {
                Ok(Body { html: row.get(0)?, text: row.get(1)? })
            })
            .optional()
            .ok()
            .flatten()
    }

    /// Keeps a body and makes its words searchable. Returns the message's thread.
    pub fn save_body(&self, message_id: &str, body: &Body) -> Result<Option<String>> {
        self.db.execute(
            "INSERT OR REPLACE INTO bodies (message_id, html, text) VALUES (?1, ?2, ?3)",
            params![message_id, body.html, body.text],
        )?;
        let words: String = drafts::body_text(body).chars().take(20_000).collect();
        self.db.execute(
            "UPDATE search SET body = ?2 WHERE rowid = (SELECT search_id FROM messages WHERE id = ?1)",
            params![message_id, words],
        )?;
        Ok(self.thread_of_message(message_id))
    }

    /// The newest messages of the inbox that have no body yet.
    pub fn missing_bodies(&self, limit: usize) -> Result<Vec<String>> {
        let mut statement = self.db.prepare_cached(
            "SELECT m.id FROM (SELECT thread, last_date FROM thread_labels WHERE label = 'inbox'
                 ORDER BY last_date DESC LIMIT 500) l
             JOIN messages m ON m.thread = l.thread LEFT JOIN bodies b ON b.message_id = m.id
             WHERE b.message_id IS NULL ORDER BY l.last_date DESC, m.date DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The messages of a thread that show open (its newest and the unread ones) and have no body.
    pub fn bodies_to_fetch(&self, thread: &str) -> Result<Vec<String>> {
        let mut statement = self.db.prepare_cached(
            "SELECT m.id FROM messages m LEFT JOIN bodies b ON b.message_id = m.id
             WHERE m.thread = ?1 AND b.message_id IS NULL
                 AND (m.unread OR m.date = (SELECT max(date) FROM messages WHERE thread = ?1))",
        )?;
        let rows = statement.query_map([thread], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The threads whose messages match, newest first.
    pub fn search<Tz: TimeZone>(&self, text: &str, now: &DateTime<Tz>) -> Result<Vec<String>> {
        find(&self.db, &search::parse(text, now), SEARCH_LIMIT)
    }

    pub fn threads_of_messages(&self, ids: &[String]) -> Vec<String> {
        let mut threads: Vec<String> = Vec::new();
        for id in ids {
            if let Some(thread) = self.thread_of_message(id).filter(|thread| !threads.contains(thread)) {
                threads.push(thread);
            }
        }
        threads
    }

    /// The ids of the inbox messages in the threads a mailbox shows (only its unread or starred
    /// ones with a filter, and Unread's `kept`), older than `before` when given, newest first.
    pub fn inbox_messages(
        &self,
        mailbox: &str,
        before: Option<i64>,
        filter: Option<Filter>,
        kept: &[String],
    ) -> Result<Vec<String>> {
        let Some((label, account)) = resolve(mailbox, splits_in(&self.db)?.is_some()) else { return Ok(Vec::new()) };
        let (label, unread) = unread_of(label, kept);
        let by_account = if account.is_some() { "AND l.account_id = ?2" } else { "AND ?2 IS NULL" };
        let by_filter = filter_sql(filter);
        let by_unread = if unread.is_some() { unread_sql(4) } else { "AND ?4 IS NULL".into() };
        let sql = format!(
            "SELECT m.id FROM thread_labels l JOIN messages m ON m.thread = l.thread
             WHERE l.label = ?1 {by_account} {by_filter} {by_unread} AND l.last_date < ?3
                 AND EXISTS (SELECT 1 FROM json_each(m.labels) WHERE value = 'inbox')
             ORDER BY l.last_date DESC"
        );
        let mut statement = self.db.prepare_cached(&sql)?;
        let rows =
            statement.query_map(params![label, account, before.unwrap_or(i64::MAX), unread], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---------- labels, preferences, drafts ----------

    /// A label's account and name.
    pub fn label(&self, id: &str) -> Option<(String, String)> {
        self.db
            .query_row("SELECT account_id, name FROM labels WHERE id = ?1", [id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
            .ok()
            .flatten()
    }

    pub fn preference(&self, key: &str) -> Option<Value> {
        preference_in(&self.db, key).ok().flatten()
    }

    pub fn preferences(&self) -> Result<Vec<(String, Value)>> {
        let mut statement = self.db.prepare_cached("SELECT key, value FROM preferences ORDER BY key")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut list = Vec::new();
        for row in rows {
            let (key, value) = row?;
            list.push((key, serde_json::from_str(&value).unwrap_or(Value::Null)));
        }
        Ok(list)
    }

    pub fn saved_draft(&self, id: &str) -> Option<Draft> {
        let text: Option<String> = self
            .db
            .query_row("SELECT draft FROM saved_drafts WHERE id = ?1", [id], |row| row.get(0))
            .optional()
            .ok()
            .flatten();
        text.and_then(|text| serde_json::from_str(&text).ok())
    }

    /// The newest saved reply draft of a thread.
    pub fn reply_draft_of(&self, thread: &str) -> Option<String> {
        self.db
            .query_row("SELECT id FROM saved_drafts WHERE thread = ?1 ORDER BY updated DESC LIMIT 1", [thread], |row| {
                row.get(0)
            })
            .optional()
            .ok()
            .flatten()
    }

    // ---------- people ----------

    /// The people whose name or address starts with every word of `query`, those the user wrote
    /// to most first.
    pub fn contacts(&self, query: &str, limit: usize) -> Result<Vec<Address>> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
        let mut found = Vec::new();
        let Some(first) = words.first() else {
            let mut statement = self.db.prepare_cached(
                "SELECT email, name, sent FROM contacts ORDER BY (sent * 4 + received) DESC, last DESC LIMIT ?1",
            )?;
            let rows = statement.query_map([(limit * 4) as i64], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            for row in rows {
                let (email, name, sent): (String, Option<String>, i64) = row?;
                if sent > 0 || !is_robot(&email) {
                    found.push(Address { name, email });
                }
            }
            found.truncate(limit);
            return Ok(found);
        };
        let mut statement = self.db.prepare_cached(
            "SELECT c.email, c.name, c.sent FROM contacts c
             WHERE c.email IN (SELECT email FROM contact_words WHERE word >= ?1 AND word < ?2)
             ORDER BY (c.sent * 4 + c.received) DESC, c.last DESC",
        )?;
        let rows = statement.query_map(params![first, format!("{first}\u{10FFFF}")], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        for row in rows {
            let (email, name, sent): (String, Option<String>, i64) = row?;
            if sent == 0 && is_robot(&email) {
                continue;
            }
            let known = contact_words(&email, name.as_deref());
            if words[1..].iter().all(|word| known.iter().any(|known| known.starts_with(word.as_str()))) {
                found.push(Address { name, email });
                if found.len() == limit {
                    break;
                }
            }
        }
        Ok(found)
    }

    /// Someone, and the latest threads with them.
    pub fn person<Tz: TimeZone>(&self, email: &str, now: &DateTime<Tz>) -> Result<Person>
    where
        Tz::Offset: std::fmt::Display,
    {
        let email = email.trim().to_lowercase();
        let name: Option<String> = self
            .db
            .query_row("SELECT name FROM contacts WHERE email = ?1", [&email], |row| row.get(0))
            .optional()?
            .flatten();
        let address = Address::new(name.as_deref(), &email);
        let mut statement = self
            .db
            .prepare_cached("SELECT thread FROM thread_people WHERE email = ?1 ORDER BY last_date DESC LIMIT 30")?;
        let threads: Vec<String> = statement.query_map([&email], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(Person {
            name: address.display().to_string(),
            initials: rows::initials(&address),
            email,
            threads: rows_of(&self.db, &threads, now)?,
        })
    }

    pub fn labels_named(&self, ids: &[String]) -> Result<Vec<LabelChip>> {
        let mut statement = self.db.prepare_cached("SELECT name FROM labels WHERE id = ?1")?;
        let mut chips = Vec::new();
        for id in ids {
            if let Some(name) = statement.query_row([id], |row| row.get(0)).optional()? {
                chips.push(LabelChip { id: id.clone(), name });
            }
        }
        chips.sort_by_key(|chip| chip.name.to_lowercase());
        Ok(chips)
    }
}

/// The read-only connection: searches run on it, away from the loop.
pub struct Reader {
    db: Connection,
}

impl Reader {
    /// The threads that match, and their rows.
    pub fn search<Tz: TimeZone>(&self, text: &str, now: &DateTime<Tz>) -> Result<(Vec<String>, Vec<ThreadRow>)>
    where
        Tz::Offset: std::fmt::Display,
    {
        let threads = find(&self.db, &search::parse(text, now), SEARCH_LIMIT)?;
        let rows = rows_of(&self.db, &threads, now)?;
        Ok((threads, rows))
    }
}

/// Runs a search: the FTS match newest first (its rowids are in date order, so nothing is sorted),
/// the conditions on each message, until `limit` threads are found.
fn find(db: &Connection, query: &Query, limit: usize) -> Result<Vec<String>> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let key = if query.text.is_some() { "search.rowid" } else { "m.search_id" };
    let mut values: Vec<Sql> = Vec::new();
    let mut conditions: Vec<String> = Vec::new();
    fn bind(value: Sql, values: &mut Vec<Sql>) -> String {
        values.push(value);
        format!("?{}", values.len())
    }
    if let Some(text) = &query.text {
        conditions.push(format!("search MATCH {}", bind(text.clone().into(), &mut values)));
    }
    if !query.includes_trash() {
        conditions.push("m.labels NOT LIKE '%\"trash\"%' AND m.labels NOT LIKE '%\"spam\"%'".into());
    }
    for condition in &query.conditions {
        let sql = match condition {
            Condition::Unread(unread) => format!("m.unread = {}", *unread as i32),
            Condition::Starred(starred) => format!("m.starred = {}", *starred as i32),
            Condition::Attachment => "m.attachments <> '[]'".into(),
            Condition::In(mailbox) if mailbox == "anywhere" => continue,
            Condition::In(mailbox) => format!(
                "EXISTS (SELECT 1 FROM thread_labels l WHERE l.thread = m.thread AND l.label = {})",
                bind(mailbox.clone().into(), &mut values)
            ),
            Condition::Label(name) => {
                let mut statement = db.prepare_cached(
                    "SELECT id FROM labels WHERE lower(name) = ?1 OR lower(replace(name, ' ', '-')) = ?1",
                )?;
                let ids: Vec<String> =
                    statement.query_map([name], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
                if ids.is_empty() {
                    return Ok(Vec::new());
                }
                let any: Vec<String> = ids
                    .iter()
                    .map(|id| format!("m.labels LIKE {}", bind(format!("%\"{id}\"%").into(), &mut values)))
                    .collect();
                format!("({})", any.join(" OR "))
            }
            Condition::Before(ms) => format!("{key} < {}", bind((ms.saturating_mul(1000)).into(), &mut values)),
            Condition::After(ms) => format!("{key} >= {}", bind((ms.saturating_mul(1000)).into(), &mut values)),
            Condition::Without(text) => format!(
                "m.search_id NOT IN (SELECT rowid FROM search WHERE search MATCH {})",
                bind(text.clone().into(), &mut values)
            ),
        };
        conditions.push(sql);
    }
    if conditions.is_empty() {
        conditions.push("1".into());
    }
    let from = match query.text {
        Some(_) => "search JOIN messages m ON m.search_id = search.rowid",
        None => "messages m",
    };
    let sql = format!("SELECT m.thread FROM {from} WHERE {} ORDER BY {key} DESC", conditions.join(" AND "));
    let mut statement = db.prepare(&sql)?;
    let mut rows = statement.query(params_from_iter(values))?;
    let mut threads = Vec::new();
    let mut seen = HashSet::new();
    while let Some(row) = rows.next()? {
        let thread: String = row.get(0)?;
        if seen.insert(thread.clone()) {
            threads.push(thread);
            if threads.len() == limit {
                break;
            }
        }
    }
    Ok(threads)
}

fn rows_of<Tz: TimeZone>(db: &Connection, threads: &[String], now: &DateTime<Tz>) -> Result<Vec<ThreadRow>>
where
    Tz::Offset: std::fmt::Display,
{
    let mut statement = db.prepare_cached(&format!(
        "SELECT {ROW_COLUMNS} FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?1"
    ))?;
    let mut rows = Vec::new();
    for thread in threads {
        if let Some(row) = statement.query_row([thread], |row| row_from(row, now)).optional()? {
            rows.push(row);
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.timestamp));
    Ok(rows)
}

fn row_from<Tz: TimeZone>(row: &rusqlite::Row, now: &DateTime<Tz>) -> rusqlite::Result<ThreadRow>
where
    Tz::Offset: std::fmt::Display,
{
    let timestamp: i64 = row.get(6)?;
    Ok(ThreadRow {
        id: row.get(0)?,
        account_id: row.get(1)?,
        color: row.get(2)?,
        senders: row.get(3)?,
        subject: row.get(4)?,
        snippet: rows::snippet(&row.get::<_, String>(5)?),
        date: dates::short(timestamp, now),
        timestamp,
        unread: row.get::<_, i64>(7)? > 0,
        starred: row.get(8)?,
        attachment: row.get(9)?,
        snoozed: row.get(10)?,
        draft_id: None,
    })
}

/// Addresses that send but don't read: left out of suggestions unless the user wrote to them.
fn is_robot(email: &str) -> bool {
    ["noreply", "no-reply", "donotreply", "do-not-reply", "mailer-daemon"].iter().any(|robot| email.contains(robot))
}

/// The words a contact is found by: its address, the address's parts, its name's words.
fn contact_words(email: &str, name: Option<&str>) -> Vec<String> {
    let mut words = vec![email.to_string()];
    if let Some((local, domain)) = email.split_once('@') {
        words.push(local.to_string());
        words.push(domain.to_string());
        words.extend(local.split(['.', '_', '-', '+']).filter(|part| part.len() > 1).map(String::from));
    }
    if let Some(name) = name {
        words.extend(
            name.to_lowercase()
                .split(|c: char| c.is_whitespace() || ",.()\"'<>".contains(c))
                .filter(|word| !word.is_empty())
                .map(String::from),
        );
    }
    words.sort();
    words.dedup();
    words
}

/// Counts the people of a new message: whom the user wrote to, and who wrote.
fn remember_people(tx: &Transaction, message: &Message, me: &HashSet<String>) -> Result<()> {
    let from_me = me.contains(&message.from.email);
    let recipients = message.recipients.to.iter().chain(&message.recipients.cc).chain(&message.recipients.bcc);
    let mut seen = HashSet::new();
    for (index, address) in std::iter::once(&message.from).chain(recipients).enumerate() {
        if address.email.is_empty() || me.contains(&address.email) || !seen.insert(address.email.as_str()) {
            continue;
        }
        let sent = from_me as i64;
        let received = (index == 0) as i64;
        let known: Option<Option<String>> = tx
            .prepare_cached("SELECT name FROM contacts WHERE email = ?1")?
            .query_row([&address.email], |row| row.get(0))
            .optional()?;
        tx.prepare_cached(
            "INSERT INTO contacts (email, name, sent, received, last) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (email) DO UPDATE SET sent = sent + excluded.sent, received = received + excluded.received,
                 name = CASE WHEN excluded.name IS NOT NULL AND excluded.last >= last THEN excluded.name ELSE name END,
                 last = max(last, excluded.last)",
        )?
        .execute(params![address.email, address.name, sent, received, message.date])?;
        let renamed = address.name.is_some() && known.as_ref().is_some_and(|name| *name != address.name);
        if known.is_none() || renamed {
            let mut statement =
                tx.prepare_cached("INSERT OR IGNORE INTO contact_words (word, email) VALUES (?1, ?2)")?;
            for word in contact_words(&address.email, address.name.as_deref()) {
                statement.execute(params![word, address.email])?;
            }
        }
    }
    Ok(())
}

fn set_state(message: &mut Message, state: MessageState) {
    message.labels = state.labels;
    message.unread = state.unread;
    message.starred = state.starred;
    message.snoozed_until = state.snoozed_until;
}

fn pending_ops(db: &Connection) -> Result<Vec<(String, Op)>> {
    let mut statement = db.prepare_cached("SELECT op_id, op FROM outbox ORDER BY created, rowid")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut ops = Vec::new();
    for row in rows {
        let (id, op) = row?;
        if let Ok(op) = serde_json::from_str(&op) {
            ops.push((id, op));
        }
    }
    Ok(ops)
}

/// The ops still waiting on each message.
fn waiting_by_message(pending: &[(String, Op)]) -> HashMap<&str, Vec<&Op>> {
    let mut waiting: HashMap<&str, Vec<&Op>> = HashMap::new();
    for (_, op) in pending {
        for id in op.ids() {
            waiting.entry(id.as_str()).or_default().push(op);
        }
    }
    waiting
}

/// What an op that isn't on messages does here: a preference, a draft, a label, a colour.
fn local_effect(tx: &Transaction, op: &Op) -> Result<Touched> {
    let mut touched = Touched::default();
    match op {
        Op::SetAccountColor { account_id, color } => {
            tx.execute("UPDATE accounts SET color = ?2 WHERE id = ?1", params![account_id, color])?;
            touched.mailboxes = true;
        }
        Op::SetPreference { key, value } => {
            set_preference(tx, key, value.as_ref())?;
            touched.preferences = true;
            if is_split_key(key) {
                touched.mailboxes = true;
                resplit_inbox(tx, &context(tx)?)?;
            }
        }
        Op::SaveDraft { draft_id, draft } => {
            touched.threads.extend(save_draft(tx, draft_id, Some(draft), mail_protocol::now_ms())?);
            touched.mailboxes = true;
        }
        Op::DeleteDraft { draft_id } => {
            touched.threads.extend(save_draft(tx, draft_id, None, 0)?);
            touched.mailboxes = true;
        }
        Op::CreateLabel { account_id, label_id, name } => {
            tx.execute(
                "INSERT OR REPLACE INTO labels (id, account_id, name) VALUES (?1, ?2, ?3)",
                params![label_id, account_id, name],
            )?;
            touched.mailboxes = true;
        }
        _ => {}
    }
    Ok(touched)
}

fn me_in(db: &Connection) -> Result<HashSet<String>> {
    let mut statement = db.prepare_cached("SELECT address, identities FROM accounts")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut me = HashSet::new();
    for row in rows {
        let (address, identities) = row?;
        me.insert(address.to_lowercase());
        let identities: Vec<Identity> = serde_json::from_str(&identities).unwrap_or_default();
        me.extend(identities.into_iter().map(|identity| identity.email.to_lowercase()));
    }
    Ok(me)
}

fn preference_in(db: &Connection, key: &str) -> Result<Option<Value>> {
    let text: Option<String> = db
        .prepare_cached("SELECT value FROM preferences WHERE key = ?1")?
        .query_row([key], |row| row.get(0))
        .optional()?;
    Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
}

fn set_preference(tx: &Transaction, key: &str, value: Option<&Value>) -> Result<()> {
    match value {
        Some(value) => tx.execute(
            "INSERT OR REPLACE INTO preferences (key, value) VALUES (?1, ?2)",
            params![key, serde_json::to_string(value)?],
        )?,
        None => tx.execute("DELETE FROM preferences WHERE key = ?1", [key])?,
    };
    Ok(())
}

/// Keeps a saved draft, or deletes it when none. Returns the threads whose reply draft changed.
fn save_draft(tx: &Transaction, id: &str, draft: Option<&Draft>, updated: i64) -> Result<Vec<String>> {
    let old: Option<Option<String>> =
        tx.query_row("SELECT thread FROM saved_drafts WHERE id = ?1", [id], |row| row.get(0)).optional()?;
    let mut threads: Vec<String> = old.flatten().into_iter().collect();
    match draft {
        Some(draft) => {
            let thread = draft.thread_id.as_ref().map(|thread_id| thread_key(&draft.account_id, thread_id));
            tx.execute(
                "INSERT OR REPLACE INTO saved_drafts (id, account_id, thread, draft, updated) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, draft.account_id, thread, serde_json::to_string(draft)?, updated],
            )?;
            threads.extend(thread);
        }
        None => {
            tx.execute("DELETE FROM saved_drafts WHERE id = ?1", [id])?;
        }
    }
    Ok(threads)
}

fn splits_in(db: &Connection) -> Result<Option<Vec<Split>>> {
    if preference_in(db, "split_inbox")?.and_then(|on| on.as_bool()) != Some(true) {
        return Ok(None);
    }
    let mut statement =
        db.prepare_cached("SELECT key, value FROM preferences WHERE key >= 'split:' AND key < 'split;'")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut splits = Vec::new();
    for row in rows {
        let (key, value) = row?;
        let Ok(mut split) = serde_json::from_str::<Split>(&value) else { continue };
        split.id = key["split:".len()..].to_string();
        splits.push(split);
    }
    splits.sort_by(|a, b| (a.order, &a.id).cmp(&(b.order, &b.id)));
    Ok(Some(splits))
}

fn context(db: &Connection) -> Result<Context> {
    Ok(Context { me: me_in(db)?, splits: splits_in(db)? })
}

fn message_in(db: &Connection, id: &str) -> rusqlite::Result<Option<Message>> {
    db.prepare_cached(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"))?
        .query_row([id], message_from_row)
        .optional()
}

struct Written {
    /// The message wasn't here before.
    new: bool,
    /// The thread it was in before, if that changed.
    moved_from: Option<String>,
}

/// Writes a message, and its search entry when its words changed.
fn write_message(tx: &Transaction, message: &Message, me: &HashSet<String>) -> Result<Written> {
    let thread = thread_key(&message.account_id, &message.thread_id);
    let old: Option<(String, i64, String, String)> = tx
        .prepare_cached("SELECT thread, search_id, subject, snippet FROM messages WHERE id = ?1")?
        .query_row([&message.id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .optional()?;
    // Search ids follow the date, so the newest matches come first without sorting.
    let search_id = match &old {
        Some((_, search_id, ..)) => *search_id,
        None => {
            let base = message.date.max(0).saturating_mul(1000);
            let next: i64 = tx
                .prepare_cached(
                    "SELECT COALESCE(max(search_id) + 1, ?1) FROM messages WHERE search_id >= ?1 AND search_id < ?1 + 1000",
                )?
                .query_row([base], |row| row.get(0))?;
            // A thousand messages of the same millisecond: past them, order no longer matters.
            match next < base + 1000 {
                true => next,
                false => tx
                    .prepare_cached("SELECT COALESCE(max(search_id), 0) + 1 FROM messages")?
                    .query_row([], |row| row.get(0))?,
            }
        }
    };
    tx.prepare_cached(
        "INSERT INTO messages (id, account_id, thread, thread_id, from_name, from_email, recipients, subject, snippet,
             date, unread, starred, labels, attachments, message_id, in_reply_to, refs, snoozed_until, bulk, unsubscribe,
             search_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
         ON CONFLICT (id) DO UPDATE SET account_id = excluded.account_id, thread = excluded.thread,
             thread_id = excluded.thread_id, from_name = excluded.from_name, from_email = excluded.from_email,
             recipients = excluded.recipients, subject = excluded.subject, snippet = excluded.snippet, date = excluded.date,
             unread = excluded.unread, starred = excluded.starred, labels = excluded.labels,
             attachments = excluded.attachments, message_id = excluded.message_id, in_reply_to = excluded.in_reply_to,
             refs = excluded.refs, snoozed_until = excluded.snoozed_until, bulk = excluded.bulk,
             unsubscribe = excluded.unsubscribe",
    )?
    .execute(params![
        message.id,
        message.account_id,
        thread,
        message.thread_id,
        message.from.name,
        message.from.email,
        serde_json::to_string(&message.recipients)?,
        message.subject,
        message.snippet,
        message.date,
        message.unread,
        message.starred,
        serde_json::to_string(&message.labels)?,
        serde_json::to_string(&message.attachments)?,
        message.message_id,
        message.in_reply_to,
        serde_json::to_string(&message.references)?,
        message.snoozed_until,
        message.bulk,
        message.unsubscribe.as_ref().map(serde_json::to_string).transpose()?,
        search_id,
    ])?;
    let words_changed =
        old.as_ref().is_none_or(|(_, _, subject, snippet)| *subject != message.subject || *snippet != message.snippet);
    if words_changed {
        let person = |address: &Address| format!("{} {}", address.name.as_deref().unwrap_or_default(), address.email);
        let recipients: Vec<String> = message
            .recipients
            .to
            .iter()
            .chain(&message.recipients.cc)
            .chain(&message.recipients.bcc)
            .map(person)
            .collect();
        let values = params![search_id, message.subject, person(&message.from), recipients.join(" "), message.snippet];
        let updated = tx
            .prepare_cached(
                "UPDATE search SET subject = ?2, sender = ?3, recipients = ?4, snippet = ?5 WHERE rowid = ?1",
            )?
            .execute(values)?;
        if updated == 0 {
            tx.prepare_cached(
                "INSERT INTO search (rowid, subject, sender, recipients, snippet, body) VALUES (?1, ?2, ?3, ?4, ?5, '')",
            )?
            .execute(values)?;
        }
    }
    if old.is_none() {
        remember_people(tx, message, me)?;
    }
    Ok(Written { new: old.is_none(), moved_from: old.map(|(old, ..)| old).filter(|old| *old != thread) })
}

fn delete_message(tx: &Transaction, id: &str) -> Result<Option<String>> {
    let found: Option<(i64, String)> = tx
        .query_row("SELECT search_id, thread FROM messages WHERE id = ?1", [id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    let Some((search_id, thread)) = found else { return Ok(None) };
    tx.execute("DELETE FROM search WHERE rowid = ?1", [search_id])?;
    tx.execute("DELETE FROM messages WHERE id = ?1", [id])?;
    tx.execute("DELETE FROM bodies WHERE message_id = ?1", [id])?;
    tx.execute("DELETE FROM bases WHERE message_id = ?1", [id])?;
    Ok(Some(thread))
}

fn remove_account(tx: &Transaction, account_id: &str, touched: &mut HashSet<String>) -> Result<()> {
    tx.execute(
        "DELETE FROM search WHERE rowid IN (SELECT search_id FROM messages WHERE account_id = ?1)",
        [account_id],
    )?;
    tx.execute("DELETE FROM bodies WHERE message_id IN (SELECT id FROM messages WHERE account_id = ?1)", [account_id])?;
    tx.execute("DELETE FROM bases WHERE message_id IN (SELECT id FROM messages WHERE account_id = ?1)", [account_id])?;
    tx.execute("DELETE FROM messages WHERE account_id = ?1", [account_id])?;
    let mut statement = tx.prepare("SELECT id FROM threads WHERE account_id = ?1")?;
    let threads: Vec<String> = statement.query_map([account_id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    touched.extend(threads);
    tx.execute("DELETE FROM saved_drafts WHERE account_id = ?1", [account_id])?;
    tx.execute("DELETE FROM labels WHERE account_id = ?1", [account_id])?;
    tx.execute("DELETE FROM accounts WHERE id = ?1", [account_id])?;
    Ok(())
}

/// A thread's place in one mailbox.
#[derive(Debug, Clone, PartialEq)]
struct Placed {
    account_id: String,
    last_date: i64,
    /// Its unread messages there.
    unread: i64,
    starred: bool,
}

impl Placed {
    /// What the mailbox's counts see of it.
    fn counted(&self) -> (&str, bool, bool) {
        (&self.account_id, self.unread > 0, self.starred)
    }
}

fn add_to_counts(tx: &Transaction, label: &str, placed: &Placed, sign: i64) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO counts (label, account_id, total, unread, starred) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (label, account_id) DO UPDATE SET total = total + excluded.total,
             unread = unread + excluded.unread, starred = starred + excluded.starred",
    )?
    .execute(params![
        label,
        placed.account_id,
        sign,
        sign * (placed.unread > 0) as i64,
        sign * placed.starred as i64
    ])?;
    Ok(())
}

/// Each message of a thread as its split sees it: its sender, its labels, whether it is bulk.
type Sorted<'a> = (&'a str, &'a [String], bool);

/// The split of the inbox a thread is in: the first custom split that takes it, Other when all
/// its mail from others is bulk, Important otherwise.
fn split_of(messages: &[Sorted], splits: &[Split], me: &HashSet<String>) -> String {
    let taken = |split: &&Split| messages.iter().any(|(sender, labels, _)| split.matches(sender, labels, me));
    if let Some(split) = splits.iter().find(taken) {
        return format!("inbox:{}", split.id);
    }
    let mut others = messages.iter().filter(|(sender, ..)| !me.contains(*sender)).peekable();
    match others.peek().is_some() && others.all(|(_, _, bulk)| *bulk) {
        true => "inbox:other".into(),
        false => "inbox:important".into(),
    }
}

/// Rebuilds a thread's row, its mailboxes, their counts and its people from its messages,
/// writing only what changed.
fn recompute(tx: &Transaction, thread: &str, context: &Context) -> Result<()> {
    let messages: Vec<Message> = {
        let mut statement =
            tx.prepare_cached(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE thread = ?1 ORDER BY date"))?;
        statement.query_map([thread], message_from_row)?.collect::<rusqlite::Result<_>>()?
    };
    let old: HashMap<String, Placed> = {
        let mut statement = tx.prepare_cached(
            "SELECT label, account_id, last_date, unread, starred FROM thread_labels WHERE thread = ?1",
        )?;
        let rows = statement.query_map([thread], |row| {
            Ok((
                row.get(0)?,
                Placed { account_id: row.get(1)?, last_date: row.get(2)?, unread: row.get(3)?, starred: row.get(4)? },
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut placed: HashMap<String, Placed> = HashMap::new();
    let mut people: HashMap<String, i64> = HashMap::new();
    match (messages.first(), messages.last()) {
        (Some(first), Some(last)) => {
            let from: Vec<Address> = messages.iter().map(|message| message.from.clone()).collect();
            let subject = messages
                .iter()
                .map(|message| message.subject.as_str())
                .find(|subject| !subject.is_empty())
                .unwrap_or(&first.subject);
            let starred = messages.iter().any(|message| message.starred);
            tx.prepare_cached(
                "INSERT INTO threads (id, account_id, last_date, subject, snippet, senders, count, unread, starred,
                     attachments, snoozed)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (id) DO UPDATE SET account_id = excluded.account_id, last_date = excluded.last_date,
                     subject = excluded.subject, snippet = excluded.snippet, senders = excluded.senders,
                     count = excluded.count, unread = excluded.unread, starred = excluded.starred,
                     attachments = excluded.attachments, snoozed = excluded.snoozed",
            )?
            .execute(params![
                thread,
                last.account_id,
                last.date,
                subject,
                last.snippet,
                rows::senders(&from, &context.me),
                messages.len() as i64,
                messages.iter().filter(|message| message.unread).count() as i64,
                starred,
                messages.iter().any(|message| !message.attachments.is_empty()),
                messages.iter().any(|message| message.snoozed_until.is_some()),
            ])?;
            for message in &messages {
                for label in shown_in(message) {
                    let entry = placed.entry(label).or_insert(Placed {
                        account_id: last.account_id.clone(),
                        last_date: 0,
                        unread: 0,
                        starred,
                    });
                    entry.last_date = entry.last_date.max(message.date);
                    entry.unread += message.unread as i64;
                }
                let recipients = message.recipients.to.iter().chain(&message.recipients.cc);
                for address in std::iter::once(&message.from).chain(recipients) {
                    if context.me.contains(&address.email) {
                        continue;
                    }
                    let date = people.entry(address.email.clone()).or_insert(message.date);
                    *date = (*date).max(message.date);
                }
            }
            if let (Some(splits), Some(inbox)) = (&context.splits, placed.get(role::INBOX).cloned()) {
                let sorted: Vec<Sorted> = messages
                    .iter()
                    .map(|message| (message.from.email.as_str(), message.labels.as_slice(), message.bulk))
                    .collect();
                placed.insert(split_of(&sorted, splits, &context.me), inbox);
            }
        }
        _ => {
            tx.execute("DELETE FROM threads WHERE id = ?1", [thread])?;
        }
    }
    for (label, before) in &old {
        let after = placed.get(label);
        if after.is_some_and(|after| after.counted() == before.counted()) {
            continue;
        }
        add_to_counts(tx, label, before, -1)?;
        if after.is_none() {
            tx.prepare_cached("DELETE FROM thread_labels WHERE thread = ?1 AND label = ?2")?
                .execute([thread, label])?;
        }
    }
    for (label, after) in &placed {
        let before = old.get(label);
        if before == Some(after) {
            continue;
        }
        if before.is_none_or(|before| before.counted() != after.counted()) {
            add_to_counts(tx, label, after, 1)?;
        }
        tx.prepare_cached(
            "INSERT INTO thread_labels (thread, label, account_id, last_date, unread, starred) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (thread, label) DO UPDATE SET account_id = excluded.account_id, last_date = excluded.last_date,
                 unread = excluded.unread, starred = excluded.starred",
        )?
        .execute(params![thread, label, after.account_id, after.last_date, after.unread, after.starred])?;
    }
    let known: HashMap<String, i64> = tx
        .prepare_cached("SELECT email, last_date FROM thread_people WHERE thread = ?1")?
        .query_map([thread], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for email in known.keys().filter(|email| !people.contains_key(*email)) {
        tx.prepare_cached("DELETE FROM thread_people WHERE thread = ?1 AND email = ?2")?.execute([thread, email])?;
    }
    for (email, date) in people.iter().filter(|(email, date)| known.get(*email) != Some(date)) {
        tx.prepare_cached("INSERT OR REPLACE INTO thread_people (thread, email, last_date) VALUES (?1, ?2, ?3)")?
            .execute(params![thread, email, date])?;
    }
    Ok(())
}

/// Puts every inbox thread in its split again, after the splits changed. Only the splits' rows
/// and counts change, so this reads just what decides them.
fn resplit_inbox(tx: &Transaction, context: &Context) -> Result<()> {
    let inbox: Vec<(String, Placed)> = tx
        .prepare("SELECT thread, account_id, last_date, unread, starred FROM thread_labels WHERE label = 'inbox'")?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                Placed { account_id: row.get(1)?, last_date: row.get(2)?, unread: row.get(3)?, starred: row.get(4)? },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let current: HashMap<String, String> = tx
        .prepare("SELECT thread, label FROM thread_labels WHERE label >= 'inbox:' AND label < 'inbox;'")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (thread, placed) in inbox {
        let wanted = match &context.splits {
            Some(splits) => {
                let messages: Vec<(String, String, bool)> = tx
                    .prepare_cached("SELECT from_email, labels, bulk FROM messages WHERE thread = ?1")?
                    .query_map([&thread], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                let labels: Vec<Vec<String>> =
                    messages.iter().map(|(_, labels, _)| serde_json::from_str(labels).unwrap_or_default()).collect();
                let sorted: Vec<Sorted> = messages
                    .iter()
                    .zip(&labels)
                    .map(|((sender, _, bulk), labels)| (sender.as_str(), labels.as_slice(), *bulk))
                    .collect();
                Some(split_of(&sorted, splits, &context.me))
            }
            None => None,
        };
        let had = current.get(&thread);
        if had == wanted.as_ref() {
            continue;
        }
        if let Some(had) = had {
            add_to_counts(tx, had, &placed, -1)?;
            tx.prepare_cached("DELETE FROM thread_labels WHERE thread = ?1 AND label = ?2")?.execute([&thread, had])?;
        }
        if let Some(wanted) = wanted {
            add_to_counts(tx, &wanted, &placed, 1)?;
            tx.prepare_cached(
                "INSERT INTO thread_labels (thread, label, account_id, last_date, unread, starred)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![
                thread,
                wanted,
                placed.account_id,
                placed.last_date,
                placed.unread,
                placed.starred
            ])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
