//! The mail providers an account can be at. Each turns its own mail into `RemoteMessage`s with
//! labels as roles and label ids, applies the changes clients made, fetches raw MIME and sends.
//! A connection is cheap to clone, so bodies can be fetched while the worker syncs.

pub mod gmail;
pub mod jmap;

use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;

use mail_protocol::{Address, Draft, Identity, Op, Provider};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::AppState;
use crate::db::AccountRow;
use crate::mime::File;

/// The provider no longer accepts the credentials: the user has to sign in again.
#[derive(Debug)]
pub struct Reauth(pub String);

impl fmt::Display for Reauth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the provider wants the account signed in again: {}", self.0)
    }
}

impl std::error::Error for Reauth {}

/// The provider turned the request down for good (a bad or conflicting request), as opposed to
/// failing for now; asking again won't help.
#[derive(Debug)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the provider refused: {}", self.0)
    }
}

impl std::error::Error for Refused {}

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
            Provider::Gmail => Connection::Gmail(gmail::Gmail::open(state, account, &secret).await?),
            Provider::Jmap => {
                let login: jmap::Login = serde_json::from_str(&account.login)?;
                Connection::Jmap(jmap::Jmap::open(&state.http, &login.url, &login.username, &secret).await?)
            }
        })
    }

    /// The same connection for the worker's own requests, which give way to those someone waits for.
    pub fn background(&self) -> Self {
        match self {
            Connection::Gmail(gmail) => Connection::Gmail(gmail.background()),
            Connection::Jmap(jmap) => Connection::Jmap(jmap.clone()),
        }
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

    /// Answers the sent message's provider id.
    pub async fn send(&self, draft: &Draft, from: &Address, files: &[File]) -> anyhow::Result<Option<String>> {
        match self {
            Connection::Gmail(gmail) => gmail.send(draft, from, files).await,
            Connection::Jmap(jmap) => jmap.send(draft, from, files).await,
        }
    }

    /// The addresses the account sends as, the default first.
    pub async fn identities(&self) -> anyhow::Result<Vec<Identity>> {
        match self {
            Connection::Gmail(gmail) => gmail.identities().await,
            Connection::Jmap(jmap) => jmap.identities().await,
        }
    }

    /// Makes a label at the provider and answers its id there.
    pub async fn create_label(&self, name: &str) -> anyhow::Result<String> {
        match self {
            Connection::Gmail(gmail) => gmail.create_label(name).await,
            Connection::Jmap(jmap) => jmap.create_label(name).await,
        }
    }

    /// Wakes the worker when the provider says something changed, over a stream of its own.
    /// Gmail pushes to the hook instead.
    pub async fn push(&self, wake: &Notify) -> Infallible {
        match self {
            Connection::Gmail(_) => std::future::pending().await,
            Connection::Jmap(jmap) => jmap.push(wake).await,
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

/// A snippet on one line, with its whitespace collapsed.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// HTML as plain text, as a signature is shown: tags dropped, a line per block, entities decoded.
pub fn html_text(html: &str) -> String {
    let mut text = String::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        text.push_str(&rest[..start].replace(['\r', '\n'], " "));
        let Some(end) = rest[start..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim_start_matches('/').to_ascii_lowercase();
        let name =
            tag.split(|character: char| character.is_whitespace() || character == '/').next().unwrap_or_default();
        let block = ["p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6"].contains(&name);
        if name == "br" || (block && !text.ends_with('\n')) {
            text.push('\n');
        }
        rest = &rest[start + end + 1..];
    }
    text.push_str(&rest.replace(['\r', '\n'], " "));
    let lines: Vec<String> = unescape(&text).lines().map(one_line).collect();
    let mut kept: Vec<String> = Vec::new();
    for line in lines {
        if line.is_empty() && kept.last().is_none_or(String::is_empty) {
            continue;
        }
        kept.push(line);
    }
    kept.join("\n").trim().to_string()
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
    fn turns_a_signature_into_text() {
        let html =
            "<div dir=\"ltr\">Ann Lee<div>CEO &amp; founder</div><br><div><a href=\"x\">ann.com</a>\n</div></div>";
        assert_eq!(super::html_text(html), "Ann Lee\nCEO & founder\n\nann.com");
    }

    #[test]
    fn unescapes_gmail_snippets() {
        assert_eq!(
            super::unescape("Tom &amp; Jerry&#39;s &lt;b&gt; &#x263A; &bogus; & more"),
            "Tom & Jerry's <b> ☺ &bogus; & more"
        );
    }
}
