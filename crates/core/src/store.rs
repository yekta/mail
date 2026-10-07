//! The client's SQLite copy of its mail. Messages are as the server sent them, with the changes
//! the user made since on top: `bases` keeps the server's state of a message while an op on it
//! waits in the outbox, so a later change from the server is rebased under the op. `threads`
//! and `thread_labels` are kept from the messages, so a list is one indexed read.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, TimeZone};
use mail_protocol::{Account, Address, Attachment, Body, Label, Message, MessageState, Op, Recipients, role};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::api::{AccountView, Mailbox, ThreadRow};
use crate::render::{dates, drafts, rows};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS accounts (id TEXT PRIMARY KEY, provider TEXT NOT NULL, address TEXT NOT NULL,
    status TEXT NOT NULL, color TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS labels (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, name TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS messages (
    id TEXT NOT NULL UNIQUE, account_id TEXT NOT NULL, thread TEXT NOT NULL, thread_id TEXT NOT NULL,
    from_name TEXT, from_email TEXT NOT NULL, recipients TEXT NOT NULL, subject TEXT NOT NULL,
    snippet TEXT NOT NULL, date INTEGER NOT NULL, unread INTEGER NOT NULL, starred INTEGER NOT NULL,
    labels TEXT NOT NULL, attachments TEXT NOT NULL, message_id TEXT, in_reply_to TEXT,
    refs TEXT NOT NULL, snoozed_until INTEGER);
CREATE INDEX IF NOT EXISTS messages_by_thread ON messages(thread, date);
CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, last_date INTEGER NOT NULL,
    subject TEXT NOT NULL, snippet TEXT NOT NULL, senders TEXT NOT NULL, count INTEGER NOT NULL,
    unread INTEGER NOT NULL, starred INTEGER NOT NULL, attachments INTEGER NOT NULL, snoozed INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS thread_labels (thread TEXT NOT NULL, label TEXT NOT NULL, account_id TEXT NOT NULL,
    last_date INTEGER NOT NULL, unread INTEGER NOT NULL, PRIMARY KEY (thread, label));
CREATE INDEX IF NOT EXISTS thread_labels_by_date ON thread_labels(label, last_date DESC);
CREATE INDEX IF NOT EXISTS thread_labels_by_account ON thread_labels(label, account_id, last_date DESC);
CREATE TABLE IF NOT EXISTS bodies (message_id TEXT PRIMARY KEY, html TEXT, text TEXT);
CREATE TABLE IF NOT EXISTS outbox (op_id TEXT PRIMARY KEY, op TEXT NOT NULL, created INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS bases (message_id TEXT PRIMARY KEY, state TEXT NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5(subject, people, snippet, body, tokenize = 'unicode61 remove_diacritics 2');
";

const MESSAGE_COLUMNS: &str = "id, account_id, thread_id, from_name, from_email, recipients, subject, snippet, date, \
     unread, starred, labels, attachments, message_id, in_reply_to, refs, snoozed_until";

/// The unified mailboxes, in the sidebar's order.
pub const MAILBOXES: [(&str, &str, &str); 8] = [
    ("inbox", "Inbox", "inbox"),
    ("starred", "Starred", "star"),
    ("snoozed", "Snoozed", "clock"),
    ("sent", "Sent", "send"),
    ("drafts", "Drafts", "file"),
    ("archive", "Archive", "archive"),
    ("spam", "Spam", "shield-alert"),
    ("trash", "Trash", "trash"),
];

pub struct Store {
    db: Connection,
}

pub struct LocalAccount {
    pub id: String,
    pub provider: String,
    pub address: String,
    pub status: String,
    pub color: String,
}

pub fn thread_key(account_id: &str, thread_id: &str) -> String {
    format!("{account_id}:{thread_id}")
}

fn message_from_row(row: &rusqlite::Row) -> rusqlite::Result<Message> {
    let json = |index: usize| -> rusqlite::Result<String> { row.get(index) };
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
        deleted: false,
        rev: 0,
    })
}

fn state_of(message: &Message) -> MessageState {
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

/// A search typed by the user as an FTS5 query: every word, as a prefix.
fn fts_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|word| word.chars().filter(|c| c.is_alphanumeric() || *c == '@' || *c == '.').collect::<String>())
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{}\"*", word.replace('"', "")))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Connection::open(path)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        db.execute_batch(SCHEMA)?;
        Ok(Self { db })
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
             DELETE FROM thread_labels; DELETE FROM bodies; DELETE FROM outbox; DELETE FROM bases;
             DELETE FROM search; DELETE FROM meta WHERE key IN ('token', 'cursor');",
        )?;
        Ok(())
    }

    /// The user's own addresses.
    pub fn me(&self) -> HashSet<String> {
        let mut statement = match self.db.prepare_cached("SELECT address FROM accounts") {
            Ok(statement) => statement,
            Err(_) => return HashSet::new(),
        };
        statement.query_map([], |row| row.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    pub fn account_address(&self, id: &str) -> Option<String> {
        self.db
            .query_row("SELECT address FROM accounts WHERE id = ?1", [id], |row| row.get(0))
            .optional()
            .ok()
            .flatten()
    }

    // ---------- applying what the server sends ----------

    /// Applies one batch of changes and stores its cursor, all at once. Returns the threads it
    /// changed.
    pub fn apply_changes(
        &mut self,
        accounts: &[Account],
        labels: &[Label],
        messages: &[Message],
        cursor: i64,
    ) -> Result<Vec<String>> {
        let tx = self.db.transaction()?;
        let mut touched = HashSet::new();
        for account in accounts {
            if account.deleted {
                remove_account(&tx, &account.id, &mut touched)?;
                continue;
            }
            tx.execute(
                "INSERT OR REPLACE INTO accounts (id, provider, address, status, color) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![account.id, account.provider.as_str(), account.address, account.status, account.color],
            )?;
        }
        for label in labels {
            match label.deleted {
                true => tx.execute("DELETE FROM labels WHERE id = ?1", [&label.id])?,
                false => tx.execute(
                    "INSERT OR REPLACE INTO labels (id, account_id, name) VALUES (?1, ?2, ?3)",
                    params![label.id, label.account_id, label.name],
                )?,
            };
        }
        let pending = pending_ops(&tx)?;
        for message in messages {
            if message.deleted {
                if let Some(thread) = delete_message(&tx, &message.id)? {
                    touched.insert(thread);
                }
                continue;
            }
            let mut message = message.clone();
            let base: Option<String> = tx
                .query_row("SELECT state FROM bases WHERE message_id = ?1", [&message.id], |row| row.get(0))
                .optional()?;
            if base.is_some() {
                let server = state_of(&message);
                tx.execute(
                    "UPDATE bases SET state = ?2 WHERE message_id = ?1",
                    params![message.id, serde_json::to_string(&server)?],
                )?;
                let mut state = server;
                for (_, op) in pending.iter().filter(|(_, op)| op.ids().contains(&message.id)) {
                    op.apply(&mut state);
                }
                set_state(&mut message, state);
            }
            if let Some(old) = write_message(&tx, &message)? {
                touched.insert(old);
            }
            touched.insert(thread_key(&message.account_id, &message.thread_id));
        }
        tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('cursor', ?1)", [cursor.to_string()])?;
        let me = me_in(&tx)?;
        for thread in &touched {
            recompute(&tx, thread, &me)?;
        }
        tx.commit()?;
        Ok(touched.into_iter().collect())
    }

    // ---------- ops made here ----------

    /// Shows an op at once and keeps it for the server. Returns the threads it changed.
    pub fn apply_local(&mut self, op_id: &str, op: &Op) -> Result<Vec<String>> {
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO outbox (op_id, op, created) VALUES (?1, ?2, ?3)",
            params![op_id, serde_json::to_string(op)?, mail_protocol::now_ms()],
        )?;
        let mut touched = HashSet::new();
        for id in op.ids() {
            let Some(mut message) = message_in(&tx, id)? else { continue };
            tx.execute(
                "INSERT OR IGNORE INTO bases (message_id, state) VALUES (?1, ?2)",
                params![id, serde_json::to_string(&state_of(&message))?],
            )?;
            let mut state = state_of(&message);
            op.apply(&mut state);
            set_state(&mut message, state);
            write_message(&tx, &message)?;
            touched.insert(thread_key(&message.account_id, &message.thread_id));
        }
        let me = me_in(&tx)?;
        for thread in &touched {
            recompute(&tx, thread, &me)?;
        }
        tx.commit()?;
        Ok(touched.into_iter().collect())
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
        let pending = pending_ops(&tx)?;
        let mut touched = HashSet::new();
        for id in op.ids() {
            let still_waiting: Vec<&Op> = pending.iter().map(|(_, op)| op).filter(|op| op.ids().contains(id)).collect();
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
            write_message(&tx, &message)?;
            touched.insert(thread_key(&message.account_id, &message.thread_id));
        }
        let me = me_in(&tx)?;
        for thread in &touched {
            recompute(&tx, thread, &me)?;
        }
        tx.commit()?;
        Ok((Some(op), touched.into_iter().collect()))
    }

    // ---------- reading ----------

    pub fn accounts(&self) -> Result<Vec<LocalAccount>> {
        let mut statement =
            self.db.prepare_cached("SELECT id, provider, address, status, color FROM accounts ORDER BY rowid")?;
        let rows = statement.query_map([], |row| {
            Ok(LocalAccount {
                id: row.get(0)?,
                provider: row.get(1)?,
                address: row.get(2)?,
                status: row.get(3)?,
                color: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn mailboxes(&self) -> Result<(Vec<Mailbox>, Vec<AccountView>)> {
        let mut unread: HashMap<(String, String), u32> = HashMap::new();
        {
            let mut statement = self.db.prepare_cached(
                "SELECT label, account_id, SUM(unread > 0) FROM thread_labels GROUP BY label, account_id",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u32>(2)?)))?;
            for row in rows {
                let (label, account, count) = row?;
                unread.insert((label, account), count);
            }
        }
        let total = |label: &str| unread.iter().filter(|((name, _), _)| name == label).map(|(_, count)| count).sum();
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
        for LocalAccount { id, provider, address, status, color } in self.accounts()? {
            let mut mailboxes: Vec<Mailbox> = MAILBOXES
                .iter()
                .map(|(mailbox, name, symbol)| Mailbox {
                    id: format!("{id}/{mailbox}"),
                    name: name.to_string(),
                    symbol: symbol.to_string(),
                    unread: unread.get(&(mailbox.to_string(), id.clone())).copied().unwrap_or(0),
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
            accounts.push(AccountView { id, address, provider, color, status, mailboxes });
        }
        Ok((unified, accounts))
    }

    /// A page of a mailbox: `inbox`, `<account>/inbox` or `<account>/label/<label>`.
    pub fn thread_rows<Tz: TimeZone>(
        &self,
        mailbox: &str,
        offset: usize,
        limit: usize,
        now: &DateTime<Tz>,
    ) -> Result<(Vec<ThreadRow>, usize)>
    where
        Tz::Offset: std::fmt::Display,
    {
        let parts: Vec<&str> = mailbox.splitn(3, '/').collect();
        let (label, account) = match parts.as_slice() {
            [label] => (label.to_string(), None),
            [account, label] => (label.to_string(), Some(account.to_string())),
            [account, "label", label] => (label.to_string(), Some(account.to_string())),
            _ => return Ok((Vec::new(), 0)),
        };
        let filter = if account.is_some() { "AND l.account_id = ?2" } else { "AND ?2 IS NULL" };
        let sql = format!(
            "SELECT t.id, t.account_id, a.color, t.senders, t.subject, t.snippet, l.last_date, t.unread, t.starred,
                 t.attachments, t.snoozed
             FROM thread_labels l JOIN threads t ON t.id = l.thread JOIN accounts a ON a.id = t.account_id
             WHERE l.label = ?1 {filter} ORDER BY l.last_date DESC LIMIT ?3 OFFSET ?4"
        );
        let mut statement = self.db.prepare_cached(&sql)?;
        let rows =
            statement.query_map(params![label, account, limit as i64, offset as i64], |row| row_from(row, now))?;
        let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let count_sql = format!("SELECT count(*) FROM thread_labels l WHERE l.label = ?1 {filter}");
        let total: i64 = self.db.query_row(&count_sql, params![label, account], |row| row.get(0))?;
        Ok((rows, total as usize))
    }

    /// The rows of these threads, newest first.
    pub fn rows_of<Tz: TimeZone>(&self, threads: &[String], now: &DateTime<Tz>) -> Result<Vec<ThreadRow>>
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut statement = self.db.prepare_cached(
            "SELECT t.id, t.account_id, a.color, t.senders, t.subject, t.snippet, t.last_date, t.unread, t.starred,
                 t.attachments, t.snoozed
             FROM threads t JOIN accounts a ON a.id = t.account_id WHERE t.id = ?1",
        )?;
        let mut rows = Vec::new();
        for thread in threads {
            if let Some(row) = statement.query_row([thread], |row| row_from(row, now)).optional()? {
                rows.push(row);
            }
        }
        rows.sort_by_key(|row| std::cmp::Reverse(row.timestamp));
        Ok(rows)
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
            "UPDATE search SET body = ?2 WHERE rowid = (SELECT rowid FROM messages WHERE id = ?1)",
            params![message_id, words],
        )?;
        Ok(self.thread_of_message(message_id))
    }

    /// The newest messages of the inbox that have no body yet.
    pub fn missing_bodies(&self, limit: usize) -> Result<Vec<String>> {
        let mut statement = self.db.prepare_cached(
            "SELECT m.id FROM thread_labels l JOIN messages m ON m.thread = l.thread
             LEFT JOIN bodies b ON b.message_id = m.id
             WHERE l.label = 'inbox' AND b.message_id IS NULL
             ORDER BY l.last_date DESC, m.date DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The threads whose messages match, newest first.
    pub fn search(&self, query: &str) -> Result<Vec<String>> {
        let Some(query) = fts_query(query) else { return Ok(Vec::new()) };
        let mut statement = self.db.prepare_cached(
            "SELECT DISTINCT m.thread FROM search JOIN messages m ON m.rowid = search.rowid
             WHERE search MATCH ?1 ORDER BY m.date DESC LIMIT 200",
        )?;
        let rows = statement.query_map([query], |row| row.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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
        snippet: row.get(5)?,
        date: dates::short(timestamp, now),
        timestamp,
        unread: row.get::<_, i64>(7)? > 0,
        starred: row.get(8)?,
        attachment: row.get(9)?,
        snoozed: row.get(10)?,
    })
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

fn me_in(db: &Connection) -> Result<HashSet<String>> {
    let mut statement = db.prepare_cached("SELECT address FROM accounts")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn message_in(db: &Connection, id: &str) -> rusqlite::Result<Option<Message>> {
    db.query_row(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"), [id], message_from_row).optional()
}

/// Writes a message and its search entry. Returns the thread it was in before, if that changed.
fn write_message(tx: &Transaction, message: &Message) -> Result<Option<String>> {
    let thread = thread_key(&message.account_id, &message.thread_id);
    let old: Option<String> =
        tx.query_row("SELECT thread FROM messages WHERE id = ?1", [&message.id], |row| row.get(0)).optional()?;
    tx.execute(
        "INSERT INTO messages (id, account_id, thread, thread_id, from_name, from_email, recipients, subject, snippet,
             date, unread, starred, labels, attachments, message_id, in_reply_to, refs, snoozed_until)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT (id) DO UPDATE SET account_id = excluded.account_id, thread = excluded.thread,
             thread_id = excluded.thread_id, from_name = excluded.from_name, from_email = excluded.from_email,
             recipients = excluded.recipients, subject = excluded.subject, snippet = excluded.snippet, date = excluded.date,
             unread = excluded.unread, starred = excluded.starred, labels = excluded.labels,
             attachments = excluded.attachments, message_id = excluded.message_id, in_reply_to = excluded.in_reply_to,
             refs = excluded.refs, snoozed_until = excluded.snoozed_until",
        params![
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
        ],
    )?;
    let rowid: i64 = tx.query_row("SELECT rowid FROM messages WHERE id = ?1", [&message.id], |row| row.get(0))?;
    let people: Vec<String> = std::iter::once(&message.from)
        .chain(&message.recipients.to)
        .chain(&message.recipients.cc)
        .map(|address| format!("{} {}", address.name.as_deref().unwrap_or_default(), address.email))
        .collect();
    let updated = tx.execute(
        "UPDATE search SET subject = ?2, people = ?3, snippet = ?4 WHERE rowid = ?1",
        params![rowid, message.subject, people.join(" "), message.snippet],
    )?;
    if updated == 0 {
        tx.execute(
            "INSERT INTO search (rowid, subject, people, snippet, body) VALUES (?1, ?2, ?3, ?4, '')",
            params![rowid, message.subject, people.join(" "), message.snippet],
        )?;
    }
    Ok(old.filter(|old| *old != thread))
}

fn delete_message(tx: &Transaction, id: &str) -> Result<Option<String>> {
    let found: Option<(i64, String)> = tx
        .query_row("SELECT rowid, thread FROM messages WHERE id = ?1", [id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    let Some((rowid, thread)) = found else { return Ok(None) };
    tx.execute("DELETE FROM search WHERE rowid = ?1", [rowid])?;
    tx.execute("DELETE FROM messages WHERE id = ?1", [id])?;
    tx.execute("DELETE FROM bodies WHERE message_id = ?1", [id])?;
    tx.execute("DELETE FROM bases WHERE message_id = ?1", [id])?;
    Ok(Some(thread))
}

fn remove_account(tx: &Transaction, account_id: &str, touched: &mut HashSet<String>) -> Result<()> {
    tx.execute("DELETE FROM search WHERE rowid IN (SELECT rowid FROM messages WHERE account_id = ?1)", [account_id])?;
    tx.execute("DELETE FROM bodies WHERE message_id IN (SELECT id FROM messages WHERE account_id = ?1)", [account_id])?;
    tx.execute("DELETE FROM messages WHERE account_id = ?1", [account_id])?;
    let mut statement = tx.prepare("SELECT id FROM threads WHERE account_id = ?1")?;
    let threads: Vec<String> = statement.query_map([account_id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    touched.extend(threads);
    tx.execute("DELETE FROM labels WHERE account_id = ?1", [account_id])?;
    tx.execute("DELETE FROM accounts WHERE id = ?1", [account_id])?;
    Ok(())
}

/// Rebuilds a thread's row and its mailboxes from its messages.
fn recompute(tx: &Transaction, thread: &str, me: &HashSet<String>) -> Result<()> {
    let messages: Vec<Message> = {
        let mut statement =
            tx.prepare_cached(&format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE thread = ?1 ORDER BY date"))?;
        statement.query_map([thread], message_from_row)?.collect::<rusqlite::Result<_>>()?
    };
    tx.execute("DELETE FROM thread_labels WHERE thread = ?1", [thread])?;
    let (Some(first), Some(last)) = (messages.first(), messages.last()) else {
        tx.execute("DELETE FROM threads WHERE id = ?1", [thread])?;
        return Ok(());
    };
    let from: Vec<Address> = messages.iter().map(|message| message.from.clone()).collect();
    let subject = messages
        .iter()
        .map(|message| message.subject.as_str())
        .find(|subject| !subject.is_empty())
        .unwrap_or(&first.subject);
    tx.execute(
        "INSERT OR REPLACE INTO threads (id, account_id, last_date, subject, snippet, senders, count, unread, starred,
             attachments, snoozed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            thread,
            last.account_id,
            last.date,
            subject,
            last.snippet,
            rows::senders(&from, me),
            messages.len() as i64,
            messages.iter().filter(|message| message.unread).count() as i64,
            messages.iter().any(|message| message.starred),
            messages.iter().any(|message| !message.attachments.is_empty()),
            messages.iter().any(|message| message.snoozed_until.is_some()),
        ],
    )?;
    let mut labels: HashMap<String, (i64, i64)> = HashMap::new();
    for message in &messages {
        for label in shown_in(message) {
            let entry = labels.entry(label).or_insert((0, 0));
            entry.0 = entry.0.max(message.date);
            entry.1 += message.unread as i64;
        }
    }
    for (label, (date, unread)) in labels {
        tx.execute(
            "INSERT INTO thread_labels (thread, label, account_id, last_date, unread) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![thread, label, last.account_id, date, unread],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use mail_protocol::Provider;

    use super::*;

    fn store() -> Store {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("mail.db")).unwrap();
        std::mem::forget(dir);
        store
    }

    fn account() -> Account {
        Account {
            id: "a".into(),
            provider: Provider::Jmap,
            address: "me@example.com".into(),
            status: "ready".into(),
            color: "chart-1".into(),
            deleted: false,
            rev: 1,
        }
    }

    fn message(id: &str, thread: &str, from: &str, date: i64) -> Message {
        Message {
            id: id.into(),
            account_id: "a".into(),
            thread_id: thread.into(),
            from: Address::new(Some(from), &format!("{}@x.com", from.to_lowercase())),
            recipients: Recipients::default(),
            subject: format!("About {thread}"),
            snippet: format!("Snippet {id}"),
            date,
            unread: true,
            starred: false,
            labels: vec!["inbox".into()],
            attachments: vec![],
            message_id: None,
            in_reply_to: None,
            references: vec![],
            snoozed_until: None,
            deleted: false,
            rev: 2,
        }
    }

    fn inbox(store: &Store) -> Vec<ThreadRow> {
        store.thread_rows("inbox", 0, 50, &Utc::now()).unwrap().0
    }

    #[test]
    fn threads_are_kept_from_their_messages() {
        let mut store = store();
        let mut mine = message("m2", "t1", "Me", 2_000);
        mine.from = Address::new(None, "me@example.com");
        mine.unread = false;
        let messages = [message("m1", "t1", "Alice", 1_000), mine, message("m3", "t2", "Bob", 1_500)];
        store.apply_changes(&[account()], &[], &messages, 10).unwrap();
        let rows = inbox(&store);
        assert_eq!(rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(), ["a:t1", "a:t2"]);
        assert_eq!(rows[0].senders, "Alice, me (2)");
        assert_eq!(rows[0].snippet, "Snippet m2");
        assert!(rows[0].unread);
        assert_eq!(store.cursor(), 10);
        let (unified, _) = store.mailboxes().unwrap();
        assert_eq!(unified[0].unread, 2);
    }

    #[test]
    fn a_local_op_survives_a_stale_server_change_and_settles() {
        let mut store = store();
        store.apply_changes(&[account()], &[], &[message("m1", "t1", "Alice", 1_000)], 1).unwrap();

        store.apply_local("op1", &Op::Archive { ids: vec!["m1".into()] }).unwrap();
        assert!(inbox(&store).is_empty());

        // The server hasn't applied it yet, but the sender's name changed: rebased, it stays archived.
        let mut renamed = message("m1", "t1", "Alicia", 1_000);
        renamed.unread = false;
        store.apply_changes(&[], &[], &[renamed], 2).unwrap();
        assert!(inbox(&store).is_empty());
        let archived = store.thread_rows("archive", 0, 50, &Utc::now()).unwrap().0;
        assert_eq!(archived[0].senders, "Alicia");
        assert!(!archived[0].unread);

        store.settle("op1", true).unwrap();
        assert!(store.outbox().unwrap().is_empty());
        assert!(inbox(&store).is_empty());
    }

    #[test]
    fn a_refused_op_goes_back_to_the_server_state_under_the_others() {
        let mut store = store();
        store.apply_changes(&[account()], &[], &[message("m1", "t1", "Alice", 1_000)], 1).unwrap();
        store.apply_local("archive", &Op::Archive { ids: vec!["m1".into()] }).unwrap();
        store.apply_local("read", &Op::SetUnread { ids: vec!["m1".into()], unread: false }).unwrap();

        let (op, threads) = store.settle("archive", false).unwrap();
        assert!(matches!(op, Some(Op::Archive { .. })));
        assert_eq!(threads, ["a:t1"]);
        let rows = inbox(&store);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].unread, "the read op still waits on top");
    }

    #[test]
    fn deleted_messages_and_accounts_leave_their_threads() {
        let mut store = store();
        store
            .apply_changes(
                &[account()],
                &[],
                &[message("m1", "t1", "Alice", 1_000), message("m2", "t2", "Bob", 2_000)],
                1,
            )
            .unwrap();
        let mut gone = message("m1", "t1", "Alice", 1_000);
        gone.deleted = true;
        store.apply_changes(&[], &[], &[gone], 2).unwrap();
        assert_eq!(inbox(&store).len(), 1);
        let mut removed = account();
        removed.deleted = true;
        store.apply_changes(&[removed], &[], &[], 3).unwrap();
        assert!(inbox(&store).is_empty());
        assert!(store.me().is_empty());
    }

    #[test]
    fn archive_trash_and_snooze_show_in_their_own_mailboxes() {
        let mut archived = message("m", "t", "A", 1);
        archived.labels = vec![];
        assert_eq!(shown_in(&archived), ["archive"]);
        let mut trashed = message("m", "t", "A", 1);
        trashed.labels = vec!["trash".into(), "inbox".into()];
        trashed.starred = true;
        assert_eq!(shown_in(&trashed), ["trash"]);
        let mut sent = message("m", "t", "A", 1);
        sent.labels = vec!["sent".into()];
        assert_eq!(shown_in(&sent), ["sent"]);
        let mut snoozed = message("m", "t", "A", 1);
        snoozed.labels = vec![];
        snoozed.snoozed_until = Some(5);
        assert_eq!(shown_in(&snoozed), ["snoozed"]);
    }

    #[test]
    fn search_finds_words_by_prefix_in_bodies_too() {
        let mut store = store();
        store
            .apply_changes(
                &[account()],
                &[],
                &[message("m1", "t1", "Alice", 1_000), message("m2", "t2", "Bob", 2_000)],
                1,
            )
            .unwrap();
        store.save_body("m2", &Body { html: None, text: Some("The telescope arrived".into()) }).unwrap();
        assert_eq!(store.search("teles").unwrap(), ["a:t2"]);
        assert_eq!(store.search("alice").unwrap(), ["a:t1"]);
        assert!(store.search("\"; DROP").unwrap().is_empty());
    }
}
