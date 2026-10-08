use serde::{Deserialize, Serialize};

/// The system mailboxes, as they appear in a message's labels. Anything else is a label's id.
pub mod role {
    pub const INBOX: &str = "inbox";
    pub const SENT: &str = "sent";
    pub const DRAFTS: &str = "drafts";
    pub const TRASH: &str = "trash";
    pub const SPAM: &str = "spam";
    pub const ALL: [&str; 5] = [INBOX, SENT, DRAFTS, TRASH, SPAM];
}

/// The theme colours an account can have, as the apps' tokens name them.
pub const ACCOUNT_COLORS: [&str; 10] = [
    "account-1",
    "account-2",
    "account-3",
    "account-4",
    "account-5",
    "account-6",
    "account-7",
    "account-8",
    "account-9",
    "account-10",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Gmail,
    Jmap,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Gmail => "gmail",
            Provider::Jmap => "jmap",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "gmail" => Some(Provider::Gmail),
            "jmap" => Some(Provider::Jmap),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub provider: Provider,
    pub address: String,
    /// `syncing`, `ready`, `reauth` (the provider wants the user to sign in again) or `error`.
    pub status: String,
    /// The theme colour of its dot, `account-1` to `account-10`.
    pub color: String,
    /// The addresses it can send as, the account's own first.
    #[serde(default)]
    pub identities: Vec<Identity>,
    pub deleted: bool,
    pub rev: i64,
}

/// An address an account sends as, with its name and the signature its provider keeps for it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Identity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
    /// Plain text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub deleted: bool,
    pub rev: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(name: Option<&str>, email: &str) -> Self {
        let name = name.map(str::trim).filter(|name| !name.is_empty() && *name != email).map(String::from);
        Self { name, email: email.trim().to_lowercase() }
    }

    /// The name if there is one, otherwise the part of the address before the @.
    pub fn display(&self) -> &str {
        match &self.name {
            Some(name) => name,
            None => self.email.split('@').next().unwrap_or(&self.email),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recipients {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<Address>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<Address>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<Address>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reply_to: Vec<Address>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub size: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub account_id: String,
    /// The provider's thread, unique within the account.
    pub thread_id: String,
    pub from: Address,
    pub recipients: Recipients,
    pub subject: String,
    pub snippet: String,
    /// Unix milliseconds.
    pub date: i64,
    pub unread: bool,
    pub starred: bool,
    pub labels: Vec<String>,
    pub attachments: Vec<Attachment>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub snoozed_until: Option<i64>,
    /// Sent to a list or by a machine (a newsletter, a notification), not by a person.
    #[serde(default)]
    pub bulk: bool,
    /// How to leave the list it came from, from its List-Unsubscribe headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsubscribe: Option<Unsubscribe>,
    pub deleted: bool,
    pub rev: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Unsubscribe {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailto: Option<String>,
    /// The URL takes RFC 8058's one-click POST.
    #[serde(default)]
    pub one_click: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Body {
    pub html: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub account_id: String,
    pub to: Vec<Address>,
    #[serde(default)]
    pub cc: Vec<Address>,
    #[serde(default)]
    pub bcc: Vec<Address>,
    pub subject: String,
    pub text: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    /// The provider thread a reply belongs to.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// One of the account's identities to send as; none for its first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Address>,
    /// `text` as HTML. The core makes it when sending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    /// The text quoted under a reply, kept apart while writing. The core folds it into `text` and
    /// `html` when sending, so the server never sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<DraftAttachment>,
    /// A message whose attachments go along, for a forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward_attachments_of: Option<String>,
}

/// A file going out with a draft. `path` is where it is on the device that wrote the draft;
/// `upload` the id the server gave it once uploaded. The server only sends uploaded ones.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DraftAttachment {
    pub name: String,
    pub mime: String,
    pub size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// A draft kept by the server, so every device of the user has it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedDraft {
    /// Chosen by the device that started it.
    pub id: String,
    pub draft: Draft,
    /// Unix milliseconds.
    pub updated: i64,
    pub deleted: bool,
    pub rev: i64,
}

/// One synced setting of the user. The keys, and what their values are, are the core's
/// (`crates/core/src/api.rs`); the server reads `muted:` and `blocked:` ones itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preference {
    pub key: String,
    pub value: serde_json::Value,
    pub deleted: bool,
    pub rev: i64,
}
