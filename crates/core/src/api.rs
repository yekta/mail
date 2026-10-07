//! The JSON the apps and the core exchange: commands in, events out.

use mail_protocol::{Attachment, Draft};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::render::drafts::ReplyKind;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Where the core keeps its database.
    pub data_dir: String,
    /// The server to use until the user sets another.
    #[serde(default)]
    pub server_url: Option<String>,
    /// Made-up mail instead of the server's, for looking at the apps.
    #[serde(default)]
    pub demo: bool,
}

#[derive(Debug, Deserialize)]
pub struct Envelope {
    pub id: u64,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Archive,
    Trash,
    Read,
    Unread,
    Star,
    Unstar,
    Inbox,
    Spam,
    Snooze,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Status,
    SetServer {
        url: String,
    },
    /// Answers with the URL to open in a browser; the core keeps the secret behind it.
    SignInGoogle,
    /// The `mailapp://auth?...` URL the browser came back with.
    FinishSignIn {
        url: String,
    },
    AddJmapAccount {
        url: String,
        username: String,
        password: String,
    },
    DevLogin {
        email: String,
    },
    SignOut,
    RemoveAccount {
        account: String,
    },
    Mailboxes,
    Threads {
        mailbox: String,
        #[serde(default)]
        offset: usize,
        #[serde(default = "default_limit")]
        limit: usize,
    },
    OpenThread {
        thread: String,
        #[serde(default)]
        images: bool,
    },
    Act {
        action: Action,
        threads: Vec<String>,
        /// When a snooze ends, in Unix milliseconds.
        #[serde(default)]
        until: Option<i64>,
    },
    ReplyDraft {
        thread: String,
        kind: ReplyKind,
    },
    /// Sends after `delay` seconds (the undo window), or at `send_at` (Unix milliseconds).
    Send {
        draft: Draft,
        #[serde(default)]
        delay: u64,
        #[serde(default)]
        send_at: Option<i64>,
    },
    CancelSend {
        op_id: String,
    },
    Search {
        query: String,
    },
}

fn default_limit() -> usize {
    100
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Reply {
        id: u64,
        ok: bool,
        value: Value,
    },
    /// Local data changed. `threads` are the threads that changed; `mailboxes` that the lists or
    /// their counts may have.
    Changed {
        mailboxes: bool,
        threads: Vec<String>,
    },
    /// `connecting`, `online`, `offline` or `signed_out`.
    Connection {
        state: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Sent {
        op_id: String,
    },
    SendFailed {
        op_id: String,
        error: String,
        draft: Box<Draft>,
    },
    /// The server's matches for a search, merged with the local ones.
    SearchResults {
        request: u64,
        rows: Vec<ThreadRow>,
    },
    Error {
        message: String,
    },
}

/// One row of a thread list, ready to draw.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadRow {
    pub id: String,
    pub account_id: String,
    /// The account's theme colour, `chart-1` to `chart-5`.
    pub color: String,
    /// "Alice, me (3)".
    pub senders: String,
    pub subject: String,
    pub snippet: String,
    /// "9:41 AM", "Yesterday", "Mar 3".
    pub date: String,
    pub timestamp: i64,
    pub unread: bool,
    pub starred: bool,
    pub attachment: bool,
    pub snoozed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mailbox {
    pub id: String,
    pub name: String,
    /// The Lucide icon it is drawn with.
    pub symbol: String,
    pub unread: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub id: String,
    pub address: String,
    pub provider: String,
    pub color: String,
    pub status: String,
    pub mailboxes: Vec<Mailbox>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadView {
    pub id: String,
    pub account_id: String,
    pub color: String,
    pub subject: String,
    /// "Roger & me".
    pub participants: String,
    pub starred: bool,
    pub unread: bool,
    pub messages: Vec<MessageView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageView {
    pub id: String,
    pub from_name: String,
    pub from_email: String,
    pub initials: String,
    /// "to Ann, Bob".
    pub to: String,
    pub date: String,
    pub snippet: String,
    pub unread: bool,
    /// Older messages start folded to their snippet, as Newton showed them.
    pub folded: bool,
    /// The page to show, or none while the body is on its way.
    pub html: Option<String>,
    pub blocked_images: bool,
    pub attachments: Vec<Attachment>,
}
