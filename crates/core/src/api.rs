//! The JSON the apps and the core exchange: commands in, events out. Each command's answer is
//! written beside it.
//!
//! Mailbox ids: `inbox`, `starred`, `snoozed`, `sent`, `drafts`, `archive`, `spam`, `trash` for
//! every account together; `<account>/<mailbox>` for one account; `<account>/label/<label>` for a
//! label. With Split Inbox on, an inbox's splits are `inbox:important`, `inbox:other` and
//! `inbox:<split id>` (and `<account>/inbox:other`, ...); a thread is in one split only, and an
//! inbox's own id shows Important.
//!
//! The synced preferences (`Preferences`, `SetPreference`), by key:
//! - `split_inbox`: `true` splits the inbox into Important, the custom splits, and Other (mail
//!   that is `bulk`).
//! - `split:<id>`: a custom split, `{"name": "Team", "from": ["ann@x.com", "@x.com"],
//!   "label": "<label id>" | null, "order": 0}`. A thread is in it when a message's sender matches
//!   `from` (an address, or a domain written `@x.com`) or a message has `label`.
//! - `signature:<account id>`: text put under what is written from that account. Without one,
//!   the identity's own signature from the provider is used.
//! - `snippet:<id>`: `{"name": "Thanks", "text": "Thanks {first_name}!"}`.
//! - `blocked:<address or @domain>`: `true`. The server trashes new mail from it.
//! - `muted:<thread>`: `true`. The server archives new mail in it.
//! - `notify:<account id>`: `false` stops notifications for that account.
//! - `remote_images`: `false` leaves the images a message loads from the web out until the user
//!   asks for them, so the sender doesn't learn the mail was opened. By default they are shown.

use mail_protocol::{Address, Attachment, Draft, Identity};
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

/// Where the app is, kept on this device so the next start shows the same screen. Every field
/// has a default, so a state written by an older app still reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    pub mailbox: String,
    pub filter: Option<Filter>,
    /// The thread open on screen.
    pub thread: Option<String>,
    /// The row the keyboard is on.
    pub selected: Option<String>,
    /// How many rows the list had loaded, to load as many again.
    pub rows: usize,
    /// How far the list was scrolled, in points.
    pub list_offset: f64,
    pub search: String,
    /// The compose sheet as the app keeps it, with what was typed.
    pub compose: Option<Value>,
    pub sidebar: bool,
    /// The open thread's messages unfolded by hand, and the one the keyboard is on.
    pub unfolded: Vec<String>,
    pub focused: Option<String>,
    /// Remote images were allowed in the open thread.
    pub images: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            mailbox: "inbox".into(),
            filter: None,
            thread: None,
            selected: None,
            rows: 0,
            list_offset: 0.0,
            search: String::new(),
            compose: None,
            sidebar: false,
            unfolded: Vec::new(),
            focused: None,
            images: false,
        }
    }
}

/// The answer to `Boot`: the `Status`, the `Mailboxes`, the `ThreadPage` of the saved mailbox,
/// the saved thread's `ThreadView` when it is still there, the local matches of the saved
/// search, the `Preferences` and the saved state itself.
#[derive(Debug, Clone, Serialize)]
pub struct Boot {
    pub status: Value,
    pub mailboxes: Value,
    pub page: ThreadPage,
    pub thread: Option<Value>,
    pub search: Option<Vec<ThreadRow>>,
    pub preferences: Preferences,
    pub ui: UiState,
}

#[derive(Debug, Deserialize)]
pub struct Envelope {
    pub id: u64,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Needs `until`. A thread that isn't in the inbox (one the user sent) comes back to it then:
    /// that is "remind me". A reply ends the snooze early.
    Snooze,
    /// Needs `label`.
    AddLabel,
    /// Needs `label`.
    RemoveLabel,
    /// Adds `label` and takes the thread out of the inbox.
    Move,
    /// Archives the thread and keeps later mail in it out of the inbox.
    Mute,
    Unmute,
    /// Leaves the list of the thread's newest message that has a way to. Answers `url` when the
    /// list only takes a visit to a web page, for the app to open.
    Unsubscribe,
    /// Blocks the sender of the thread's newest message not from the user, and trashes the thread.
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    Unread,
    Starred,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Status,
    /// A `Boot`: everything the app's first frame needs, in the state it was closed in.
    Boot,
    /// Keeps where the app is, to come back to it at the next start. `{}`.
    SaveUi {
        ui: UiState,
    },
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
    /// `{unified: [Mailbox], accounts: [AccountView]}`.
    Mailboxes,
    /// A `ThreadPage`.
    Threads {
        mailbox: String,
        #[serde(default)]
        offset: usize,
        #[serde(default = "default_limit")]
        limit: usize,
        #[serde(default)]
        filter: Option<Filter>,
    },
    /// Fetches the bodies of these threads ahead of their opening, as the list scrolls past them
    /// or the keyboard moves near them. `{}`.
    Prefetch {
        threads: Vec<String>,
    },
    /// A `ThreadView`. Marks the thread read. `images` shows the remote images of this thread
    /// even with `remote_images` off.
    OpenThread {
        thread: String,
        #[serde(default)]
        images: bool,
    },
    /// An `ActReply`.
    Act {
        action: Action,
        threads: Vec<String>,
        /// When a snooze ends, in Unix milliseconds.
        #[serde(default)]
        until: Option<i64>,
        /// A label's id, for `add_label`, `remove_label` and `move`.
        #[serde(default)]
        label: Option<String>,
    },
    /// Takes back the last `Act` or `ArchiveAll` that answered `undo: true`. `{message}`, or an
    /// error when there is nothing to undo.
    Undo,
    /// Archives every thread a mailbox shows (older than `before`, when given; only the unread or
    /// starred ones with `filter`): "get me to zero". A split's id archives that split only. An
    /// `ActReply`.
    ArchiveAll {
        mailbox: String,
        #[serde(default)]
        before: Option<i64>,
        #[serde(default)]
        filter: Option<Filter>,
    },
    /// `{id}`: the new label's id, usable at once.
    CreateLabel {
        account: String,
        name: String,
    },
    /// The times a snooze, a reminder or a send-later can be for. Empty `text` gives the usual
    /// choices; otherwise what the text says ("tomorrow 9am", "in 2 hours", "fri", "dec 3").
    /// `{choices: [TimeChoice]}`.
    ParseTime {
        text: String,
    },
    /// Addresses the user wrote to or heard from, best first. `{contacts: [Address]}`.
    Contacts {
        query: String,
        #[serde(default = "default_contacts")]
        limit: usize,
    },
    /// A `Person`: someone, and the latest threads with them.
    Person {
        email: String,
    },
    /// A `Preferences`: every synced preference.
    Preferences,
    /// `value` null removes the key. `{}`.
    SetPreference {
        key: String,
        #[serde(default)]
        value: Option<Value>,
    },
    /// A new message from an account (the first one when none): `{draft, from}`.
    NewDraft {
        #[serde(default)]
        account: Option<String>,
    },
    /// `{draft, from}`. The draft keeps the text quoted in `quote`.
    ReplyDraft {
        thread: String,
        kind: ReplyKind,
    },
    /// Keeps a draft, on this device at once and on the user's other devices through the server.
    /// `{id}`; a new one is made when `id` is none.
    SaveDraft {
        #[serde(default)]
        id: Option<String>,
        draft: Draft,
    },
    /// `{id, draft, from}`.
    OpenDraft {
        id: String,
    },
    /// `{}`.
    DeleteDraft {
        id: String,
    },
    /// Sends after `delay` seconds (the undo window), or at `send_at` (Unix milliseconds). Adds
    /// the signature, folds the quote in and makes the HTML. Uploads the attachments that have a
    /// `path` first. Deletes the saved draft `draft_id`. `remind_at` brings the thread back to the
    /// inbox then if nobody answered. `{op_id, send_at}`.
    Send {
        draft: Draft,
        #[serde(default)]
        delay: u64,
        #[serde(default)]
        send_at: Option<i64>,
        #[serde(default)]
        remind_at: Option<i64>,
        #[serde(default)]
        draft_id: Option<String>,
    },
    /// `{draft, id}`: the draft as it was before sending, to edit again. It is kept as the saved
    /// draft `id` (the send's `draft_id` when it had one), so it stays in Drafts.
    CancelSend {
        op_id: String,
    },
    /// Downloads an attachment, or finds it already downloaded. `{path}`: a file on this device.
    OpenAttachment {
        message: String,
        /// Its place in the message's `attachments`.
        index: usize,
    },
    /// `{html}`: the whole thread, every message open, as a page to print.
    PrintThread {
        thread: String,
    },
    /// `{request, rows}`. Takes Gmail's operators: `from:`, `to:`, `subject:`, `has:attachment`,
    /// `is:unread`, `is:starred`, `in:<mailbox>`, `label:<name>`, `before:`/`after:` (YYYY/MM/DD),
    /// `newer_than:`/`older_than:` (2d, 3w, 1m, 1y), `"exact words"`, `-word` and `OR`.
    Search {
        query: String,
    },
}

fn default_limit() -> usize {
    100
}

fn default_contacts() -> usize {
    8
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
    /// their counts may have; `preferences` that a synced preference did.
    Changed {
        mailboxes: bool,
        threads: Vec<String>,
        preferences: bool,
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
    /// `draft` is the draft as it was before sending, kept again in Drafts as the saved draft
    /// `draft_id`: open that one rather than saving a copy.
    SendFailed {
        op_id: String,
        error: String,
        draft: Box<Draft>,
        #[serde(skip_serializing_if = "Option::is_none")]
        draft_id: Option<String>,
    },
    /// The server's matches for a search, merged with the local ones.
    SearchResults {
        request: u64,
        rows: Vec<ThreadRow>,
    },
    /// Mail that just arrived in an inbox, unread and not from the user, for a notification.
    /// Not sent for the mail of a first sync, nor for accounts with `notify:<account>` false.
    NewMail {
        messages: Vec<NewMail>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewMail {
    pub thread: String,
    pub account_id: String,
    /// The sender's name.
    pub from: String,
    pub subject: String,
    pub snippet: String,
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
    /// The row is a saved draft, not a thread: open it with `OpenDraft`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
}

/// A list of threads. `splits` are the tabs over an inbox when Split Inbox is on, empty
/// otherwise; the one shown is the one whose `mailbox` was asked for.
#[derive(Debug, Clone, Serialize)]
pub struct ThreadPage {
    pub rows: Vec<ThreadRow>,
    pub total: usize,
    pub splits: Vec<SplitTab>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SplitTab {
    pub mailbox: String,
    pub name: String,
    pub unread: u32,
    pub total: u32,
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
    pub identities: Vec<Identity>,
    /// Its mailboxes, then its labels (`<account>/label/<label>`).
    pub mailboxes: Vec<Mailbox>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LabelChip {
    pub id: String,
    pub name: String,
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
    pub muted: bool,
    /// The thread's custom labels.
    pub labels: Vec<LabelChip>,
    /// Whether `Act` `unsubscribe` can do something.
    pub unsubscribe: bool,
    /// The saved reply draft of this thread, to open with `OpenDraft`.
    pub draft_id: Option<String>,
    /// The address of the person the thread is with, for the contact pane.
    pub person: Option<String>,
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
    /// The server couldn't fetch the body; opening the thread again later asks again.
    pub failed: bool,
    /// Remote images were left out; `OpenThread` with `images` shows them.
    pub blocked_images: bool,
    /// Open one with `OpenAttachment` and its place in this list.
    pub attachments: Vec<Attachment>,
}

/// What an `Act` did. `message` is for a toast ("Archived."); `undo` says `Undo` can take it
/// back; `url` is a page to open to finish unsubscribing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ActReply {
    pub message: Option<String>,
    pub undo: bool,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimeChoice {
    pub id: String,
    /// "Tomorrow".
    pub name: String,
    /// "Tue 8:00 AM".
    pub label: String,
    /// Unix milliseconds.
    pub until: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Person {
    pub name: String,
    pub email: String,
    pub initials: String,
    pub threads: Vec<ThreadRow>,
}

/// The answer to `Preferences`. A list, not a map: the apps' JSON decoders rewrite map keys.
#[derive(Debug, Clone, Serialize)]
pub struct Preferences {
    pub values: Vec<PreferenceValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreferenceValue {
    pub key: String,
    pub value: Value,
}

/// The answer to `NewDraft`, `ReplyDraft` and `OpenDraft`: the draft and the address it is
/// written from.
#[derive(Debug, Clone, Serialize)]
pub struct DraftReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub draft: Draft,
    pub from: String,
}

/// The answer to `Contacts`.
#[derive(Debug, Clone, Serialize)]
pub struct Contacts {
    pub contacts: Vec<Address>,
}
