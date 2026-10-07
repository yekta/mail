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
    /// The theme colour of its dot, `chart-1` to `chart-5`.
    pub color: String,
    pub deleted: bool,
    pub rev: i64,
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
    pub deleted: bool,
    pub rev: i64,
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
}
