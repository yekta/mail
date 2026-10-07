//! What the server and the client core agree on: the mail they sync, the messages on the sync
//! socket and the JSON of the server's HTTP API.

pub mod api;
pub mod ops;
#[cfg(feature = "tls")]
pub mod tls;
pub mod types;
pub mod wire;

pub use ops::{MessageState, Op};
pub use types::*;

/// Bump when an old client or server could no longer understand the other.
pub const PROTOCOL_VERSION: u32 = 1;
/// Where the server sends the browser once a Google sign-in is done; the apps own this scheme.
pub const APP_REDIRECT: &str = "mailapp://auth";
/// How many messages one `Changes` batch holds at most.
pub const BATCH_SIZE: usize = 500;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
