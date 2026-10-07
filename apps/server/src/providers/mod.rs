//! The mail providers an account can be at. Each turns its own mail into `RemoteMessage`s with
//! labels as roles and label ids, applies the changes clients made, fetches raw MIME and sends.
//! A connection is cheap to clone, so bodies can be fetched while the worker syncs.

pub mod gmail;
pub mod jmap;

use std::collections::HashMap;
use std::fmt;

use mail_protocol::{Address, Draft, Op, Provider};
use uuid::Uuid;

use crate::AppState;
use crate::db::AccountRow;

/// The provider no longer accepts the credentials: the user has to sign in again.
#[derive(Debug)]
pub struct Reauth(pub String);

impl fmt::Display for Reauth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the provider wants the account signed in again: {}", self.0)
    }
}

impl std::error::Error for Reauth {}

#[derive(Clone)]
pub enum Connection {
    Gmail(gmail::Gmail),
    Jmap(jmap::Jmap),
}

/// One kind of change for many messages, as the worker sends it.
pub struct Batch {
    pub op: Op,
    pub provider_ids: Vec<String>,
}

/// What a sync step did: whether older mail is still waiting to be fetched.
pub struct Synced {
    pub backfilling: bool,
}

impl Connection {
    pub async fn open(state: &AppState, account: &AccountRow) -> anyhow::Result<Self> {
        let Some(secret) = state.sealer.open(&account.credentials) else {
            return Err(Reauth("the credentials can't be read".into()).into());
        };
        let secret = String::from_utf8(secret).unwrap_or_default();
        Ok(match account.provider() {
            Provider::Gmail => Connection::Gmail(gmail::Gmail::open(state, &secret).await?),
            Provider::Jmap => {
                let login: jmap::Login = serde_json::from_str(&account.login)?;
                Connection::Jmap(jmap::Jmap::open(&state.http, &login.url, &login.username, &secret).await?)
            }
        })
    }

    pub async fn sync(&self, state: &AppState, account: &AccountRow) -> anyhow::Result<Synced> {
        match self {
            Connection::Gmail(gmail) => gmail.sync(state, account).await,
            Connection::Jmap(jmap) => jmap.sync(state, account).await,
        }
    }

    pub async fn apply(&self, batch: &Batch, labels: &HashMap<Uuid, String>) -> anyhow::Result<()> {
        match self {
            Connection::Gmail(gmail) => gmail.apply(batch, labels).await,
            Connection::Jmap(jmap) => jmap.apply(batch, labels).await,
        }
    }

    pub async fn raw(&self, provider_id: &str) -> anyhow::Result<Vec<u8>> {
        match self {
            Connection::Gmail(gmail) => gmail.raw(provider_id).await,
            Connection::Jmap(jmap) => jmap.raw(provider_id).await,
        }
    }

    pub async fn send(&self, draft: &Draft, from: &Address) -> anyhow::Result<()> {
        match self {
            Connection::Gmail(gmail) => gmail.send(draft, from).await,
            Connection::Jmap(jmap) => jmap.send(draft, from).await,
        }
    }

    /// Asks the provider to push changes, where it can. Gmail's watch lasts a week.
    pub async fn watch(&self, state: &AppState) -> anyhow::Result<()> {
        match self {
            Connection::Gmail(gmail) => gmail.watch(state).await,
            Connection::Jmap(_) => Ok(()),
        }
    }
}

/// Gmail's snippets come HTML-escaped.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|number| number.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn unescapes_gmail_snippets() {
        assert_eq!(
            super::unescape("Tom &amp; Jerry&#39;s &lt;b&gt; &#x263A; &bogus; & more"),
            "Tom & Jerry's <b> ☺ &bogus; & more"
        );
    }
}
