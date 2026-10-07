//! The messages on the sync socket, one JSON text frame each.

use serde::{Deserialize, Serialize};

use crate::ops::Op;
use crate::types::{Account, Body, Label, Message};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// The first message. `cursor` is the last `Changes` cursor the client stored, 0 for none.
    Hello {
        token: String,
        cursor: i64,
        protocol: u32,
    },
    Mutate {
        op_id: String,
        op: Op,
    },
    Body {
        request: u64,
        message_id: String,
    },
    Search {
        request: u64,
        query: String,
    },
    Ping,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        protocol: u32,
    },
    /// What changed after the client's cursor. `more` means another batch follows at once.
    Changes {
        accounts: Vec<Account>,
        labels: Vec<Label>,
        messages: Vec<Message>,
        cursor: i64,
        more: bool,
    },
    /// The answer to a `Mutate`. A `Send` is answered once it was sent, failed or was cancelled.
    Applied {
        op_id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Body {
        request: u64,
        message_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Body>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    SearchResults {
        request: u64,
        message_ids: Vec<String>,
    },
    Pong,
    /// The socket is closing: a bad token or a protocol the server doesn't speak.
    Refused {
        reason: String,
    },
}
