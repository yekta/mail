//! Gmail through its REST API. The first sync lists the last 90 days (or INITIAL_SYNC_LIMIT
//! messages), then older mail is backfilled a page at a time; after that history.list says what
//! changed since the last historyId.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::{StreamExt, stream};
use mail_protocol::{Address, Attachment, Draft, Identity, Op, role};
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::{Batch, Reauth, Refused, Synced, html_text, one_line, unescape};
use crate::db::{self, AccountRow, RemoteMessage};
use crate::mime::File;
use crate::{AppState, mime};

pub const API_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const PAGE: usize = 500;
const BACKFILL_PAGE: usize = 100;
const CONCURRENT_GETS: usize = 8;
const HEADERS: [&str; 15] = [
    "From",
    "To",
    "Cc",
    "Bcc",
    "Reply-To",
    "Subject",
    "Message-ID",
    "In-Reply-To",
    "References",
    "Content-Type",
    "List-Unsubscribe",
    "List-Unsubscribe-Post",
    "List-Id",
    "Precedence",
    "Auto-Submitted",
];
/// Gmail's tabs for mail sent in bulk.
const BULK_CATEGORIES: [&str; 4] = ["CATEGORY_PROMOTIONS", "CATEGORY_SOCIAL", "CATEGORY_UPDATES", "CATEGORY_FORUMS"];

#[derive(Clone)]
pub struct Gmail {
    http: reqwest::Client,
    api: Arc<String>,
    oauth: Arc<OAuth>,
    token: Arc<Mutex<Option<(String, Instant)>>>,
}

struct OAuth {
    token_url: String,
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

#[derive(Deserialize)]
pub struct Credentials {
    pub refresh_token: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MessageRef {
    id: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    #[serde(default)]
    messages: Vec<MessageRef>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailMessage {
    id: String,
    thread_id: String,
    #[serde(default)]
    label_ids: Vec<String>,
    #[serde(default)]
    snippet: String,
    internal_date: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    headers: Vec<Header>,
}

#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

#[derive(Deserialize, Default, serde::Serialize)]
struct SyncState {
    history_id: Option<String>,
    backfill_token: Option<String>,
    #[serde(default)]
    backfilled: bool,
    /// The last 90 days were fetched again for what was added after they were first synced
    /// (`bulk`, `unsubscribe`), keeping the history id.
    #[serde(default)]
    refreshed: bool,
}

impl Gmail {
    pub async fn open(state: &AppState, secret: &str) -> anyhow::Result<Self> {
        let credentials: Credentials = serde_json::from_str(secret)?;
        let config = &state.config;
        let gmail = Self {
            http: state.http.clone(),
            api: Arc::new(config.gmail_api_url.clone()),
            oauth: Arc::new(OAuth {
                token_url: config.google_token_url.clone(),
                client_id: config.google_client_id.clone(),
                client_secret: config.google_client_secret.clone(),
                refresh_token: credentials.refresh_token,
            }),
            token: Arc::new(Mutex::new(None)),
        };
        gmail.access_token().await?;
        Ok(gmail)
    }

    async fn access_token(&self) -> anyhow::Result<String> {
        let mut token = self.token.lock().await;
        if let Some((access, expires)) = token.as_ref()
            && *expires > Instant::now()
        {
            return Ok(access.clone());
        }
        let response = self
            .http
            .post(&self.oauth.token_url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", self.oauth.refresh_token.as_str()),
                ("client_id", self.oauth.client_id.as_str()),
                ("client_secret", self.oauth.client_secret.as_str()),
            ])
            .send()
            .await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or_default();
        if status == StatusCode::BAD_REQUEST || status == StatusCode::UNAUTHORIZED {
            return Err(Reauth(body["error"].as_str().unwrap_or("refused").to_string()).into());
        }
        let Some(access) = body["access_token"].as_str() else {
            anyhow::bail!("Google answered the token refresh with {status}");
        };
        let lifetime = body["expires_in"].as_u64().unwrap_or(3600).saturating_sub(120);
        *token = Some((access.to_string(), Instant::now() + Duration::from_secs(lifetime)));
        Ok(access.to_string())
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> anyhow::Result<T> {
        let url = format!("{}{path}", self.api);
        let what = format!("{method} {path}");
        self.execute(&what, || {
            let request = self.http.request(method.clone(), &url).query(query);
            match body {
                Some(body) => request.json(body),
                None => request,
            }
        })
        .await
    }

    /// Sends what `build` makes with the access token, again after a refresh or a busy answer.
    async fn execute<T: DeserializeOwned>(
        &self,
        what: &str,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> anyhow::Result<T> {
        for attempt in 0..3 {
            let response = build().bearer_auth(self.access_token().await?).send().await?;
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                *self.token.lock().await = None;
                continue;
            }
            if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                tokio::time::sleep(Duration::from_millis(500 << attempt)).await;
                continue;
            }
            if status == StatusCode::NOT_FOUND {
                return Err(NotFound.into());
            }
            if status == StatusCode::BAD_REQUEST || status == StatusCode::CONFLICT {
                let text = response.text().await.unwrap_or_default();
                return Err(Refused(format!("Gmail answered {what} with {status}: {text}")).into());
            }
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                anyhow::bail!("Gmail answered {what} with {status}: {text}");
            }
            let text = response.text().await?;
            return Ok(serde_json::from_str(if text.is_empty() { "null" } else { &text })?);
        }
        anyhow::bail!("Gmail kept refusing {what}")
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> anyhow::Result<T> {
        self.request(Method::GET, path, query, None).await
    }

    pub async fn sync(&self, state: &AppState, account: &AccountRow) -> anyhow::Result<Synced> {
        let mut sync: SyncState = serde_json::from_value(account.sync_state.clone()).unwrap_or_default();
        let labels = self.sync_labels(state, account).await?;

        let Some(history_id) = sync.history_id.clone() else {
            let profile: Value = self.get("/profile", &[]).await?;
            let history_id = profile["historyId"].as_str().unwrap_or_default().to_string();
            let limit = state.config.initial_sync_limit;
            let ids = self.list("newer_than:90d", limit, None).await?.0;
            self.fetch_and_store(state, account, &labels, &ids).await?;
            sync.history_id = Some(history_id);
            sync.refreshed = true;
            db::save_sync_state(&state.db, account.id, &json!(sync)).await?;
            return Ok(Synced { backfilling: true });
        };

        match self.history(&history_id).await {
            Ok((changed, deleted, latest)) => {
                self.fetch_and_store(state, account, &labels, &changed).await?;
                db::delete_messages(&state.db, account, &deleted).await?;
                sync.history_id = Some(latest);
            }
            Err(error) if error.is::<NotFound>() => {
                tracing::info!("Gmail history of {} expired, syncing again", account.address);
                sync.history_id = None;
            }
            Err(error) => return Err(error),
        }

        if !sync.refreshed && sync.history_id.is_some() {
            let ids = self.list("newer_than:90d", state.config.initial_sync_limit, None).await?.0;
            self.fetch_and_store(state, account, &labels, &ids).await?;
            sync.refreshed = true;
        }
        if !sync.backfilled && sync.history_id.is_some() {
            let (ids, next) = self.list("older_than:90d", BACKFILL_PAGE, sync.backfill_token.clone()).await?;
            self.fetch_and_store(state, account, &labels, &ids).await?;
            sync.backfilled = next.is_none();
            sync.backfill_token = next;
        }
        db::save_sync_state(&state.db, account.id, &json!(sync)).await?;
        Ok(Synced { backfilling: !sync.backfilled })
    }

    async fn sync_labels(&self, state: &AppState, account: &AccountRow) -> anyhow::Result<HashMap<String, Uuid>> {
        let response: Value = self.get("/labels", &[]).await?;
        let custom: Vec<(String, String)> = response["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|label| label["type"] == "user")
            .filter_map(|label| Some((label["id"].as_str()?.to_string(), label["name"].as_str()?.to_string())))
            .collect();
        Ok(db::sync_labels(&state.db, account, &custom).await?)
    }

    /// Message ids matching `query`, newest first, up to `limit`, and where the listing stopped.
    async fn list(
        &self,
        query: &str,
        limit: usize,
        mut page: Option<String>,
    ) -> anyhow::Result<(Vec<String>, Option<String>)> {
        let mut ids = Vec::new();
        loop {
            let mut parameters =
                vec![("q", query.to_string()), ("maxResults", PAGE.min(limit - ids.len()).to_string())];
            if let Some(token) = &page {
                parameters.push(("pageToken", token.clone()));
            }
            let response: ListResponse = self.get("/messages", &parameters).await?;
            ids.extend(response.messages.into_iter().map(|message| message.id));
            page = response.next_page_token;
            if page.is_none() || ids.len() >= limit {
                return Ok((ids, page));
            }
        }
    }

    /// What changed after `history_id`: messages to fetch again, messages gone, the latest id.
    async fn history(&self, history_id: &str) -> anyhow::Result<(Vec<String>, Vec<String>, String)> {
        let mut changed = HashSet::new();
        let mut deleted = HashSet::new();
        let mut page: Option<String> = None;
        let mut latest = history_id.to_string();
        loop {
            let mut parameters = vec![("startHistoryId", history_id.to_string()), ("maxResults", "500".to_string())];
            if let Some(token) = &page {
                parameters.push(("pageToken", token.clone()));
            }
            let response: Value = self.get("/history", &parameters).await?;
            for entry in response["history"].as_array().into_iter().flatten() {
                for key in ["messagesAdded", "labelsAdded", "labelsRemoved"] {
                    for item in entry[key].as_array().into_iter().flatten() {
                        if let Some(id) = item["message"]["id"].as_str() {
                            changed.insert(id.to_string());
                        }
                    }
                }
                for item in entry["messagesDeleted"].as_array().into_iter().flatten() {
                    if let Some(id) = item["message"]["id"].as_str() {
                        deleted.insert(id.to_string());
                    }
                }
            }
            if let Some(id) = response["historyId"].as_str() {
                latest = id.to_string();
            }
            page = response["nextPageToken"].as_str().map(String::from);
            if page.is_none() {
                break;
            }
        }
        changed.retain(|id| !deleted.contains(id));
        Ok((changed.into_iter().collect(), deleted.into_iter().collect(), latest))
    }

    async fn fetch_and_store(
        &self,
        state: &AppState,
        account: &AccountRow,
        labels: &HashMap<String, Uuid>,
        ids: &[String],
    ) -> anyhow::Result<()> {
        for chunk in ids.chunks(200) {
            let fetched: Vec<(String, anyhow::Result<GmailMessage>)> = stream::iter(chunk.iter().cloned())
                .map(|id| async move {
                    let mut query = vec![("format", "metadata".to_string())];
                    query.extend(HEADERS.iter().map(|header| ("metadataHeaders", header.to_string())));
                    let message = self.get(&format!("/messages/{id}"), &query).await;
                    (id, message)
                })
                .buffer_unordered(CONCURRENT_GETS)
                .collect()
                .await;
            let mut messages = Vec::new();
            let mut gone = Vec::new();
            for (id, message) in fetched {
                match message {
                    Ok(message) => messages.push(remote(message, labels)),
                    Err(error) if error.is::<NotFound>() => gone.push(id),
                    Err(error) => return Err(error),
                }
            }
            db::upsert_messages(&state.db, account, &messages).await?;
            db::delete_messages(&state.db, account, &gone).await?;
        }
        Ok(())
    }

    pub async fn apply(&self, batch: &Batch, labels: &HashMap<Uuid, String>) -> anyhow::Result<()> {
        let label = |id: &String| match id.as_str() {
            role::INBOX => Some("INBOX".to_string()),
            role::TRASH => Some("TRASH".to_string()),
            role::SPAM => Some("SPAM".to_string()),
            _ => id.parse::<Uuid>().ok().and_then(|id| labels.get(&id).cloned()),
        };
        let (add, remove): (Vec<String>, Vec<String>) = match &batch.op {
            Op::SetUnread { unread: true, .. } => (vec!["UNREAD".into()], vec![]),
            Op::SetUnread { unread: false, .. } => (vec![], vec!["UNREAD".into()]),
            Op::SetStarred { starred: true, .. } => (vec!["STARRED".into()], vec![]),
            Op::SetStarred { starred: false, .. } => (vec![], vec!["STARRED".into()]),
            Op::Archive { .. } | Op::Snooze { .. } => (vec![], vec!["INBOX".into()]),
            Op::MoveToInbox { .. } => (vec!["INBOX".into()], vec!["TRASH".into(), "SPAM".into()]),
            Op::Trash { .. } => (vec!["TRASH".into()], vec!["INBOX".into(), "SPAM".into()]),
            Op::Spam { .. } => (vec!["SPAM".into()], vec!["INBOX".into(), "TRASH".into()]),
            Op::AddLabel { label: id, .. } => (label(id).into_iter().collect(), vec![]),
            Op::RemoveLabel { label: id, .. } => (vec![], label(id).into_iter().collect()),
            _ => return Ok(()),
        };
        if add.is_empty() && remove.is_empty() {
            return Ok(());
        }
        for ids in batch.provider_ids.chunks(1000) {
            let body = json!({ "ids": ids, "addLabelIds": add, "removeLabelIds": remove });
            let _: Value = self.request(Method::POST, "/messages/batchModify", &[], Some(&body)).await?;
        }
        Ok(())
    }

    pub async fn raw(&self, provider_id: &str) -> anyhow::Result<Vec<u8>> {
        let message: Value = self.get(&format!("/messages/{provider_id}"), &[("format", "raw".into())]).await?;
        let raw = message["raw"].as_str().unwrap_or_default().trim_end_matches('=');
        Ok(URL_SAFE_NO_PAD.decode(raw)?)
    }

    /// Goes through Gmail's upload URL, which takes messages up to 35 MB; the plain one takes 5.
    pub async fn send(&self, draft: &Draft, from: &Address, files: &[File]) -> anyhow::Result<Option<String>> {
        let domain = from.email.split('@').nth(1).unwrap_or("gmail.com");
        let raw = mime::build(draft, from, &mime::new_message_id(domain), true, files);
        let mut metadata = json!({});
        if let Some(thread_id) = &draft.thread_id {
            metadata["threadId"] = json!(thread_id);
        }
        let boundary = format!("part-{}", crate::random_token());
        let mut body = format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n\
             --{boundary}\r\nContent-Type: message/rfc822\r\n\r\n"
        )
        .into_bytes();
        body.extend(raw);
        body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());
        let url = format!("{}/messages/send", self.api.replacen("/gmail/v1/", "/upload/gmail/v1/", 1));
        let content_type = format!("multipart/related; boundary={boundary}");
        let sent: Value = self
            .execute("POST /messages/send", || {
                self.http
                    .post(&url)
                    .query(&[("uploadType", "multipart")])
                    .header(reqwest::header::CONTENT_TYPE, &content_type)
                    .body(body.clone())
            })
            .await?;
        Ok(sent["id"].as_str().map(String::from))
    }

    /// The verified send-as addresses, the default first, then the account's own.
    pub async fn identities(&self) -> anyhow::Result<Vec<Identity>> {
        let response: Value = self.get("/settings/sendAs", &[]).await?;
        let mut list: Vec<&Value> = response["sendAs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|send_as| send_as["isPrimary"] == true || send_as["verificationStatus"] == "accepted")
            .collect();
        list.sort_by_key(|send_as| (send_as["isDefault"] != true, send_as["isPrimary"] != true));
        Ok(list
            .into_iter()
            .filter_map(|send_as| {
                let address = Address::new(send_as["displayName"].as_str(), send_as["sendAsEmail"].as_str()?);
                let signature = send_as["signature"].as_str().map(html_text).filter(|text| !text.is_empty());
                Some(Identity { name: address.name, email: address.email, signature })
            })
            .collect())
    }

    pub async fn create_label(&self, name: &str) -> anyhow::Result<String> {
        let body = json!({ "name": name, "labelListVisibility": "labelShow", "messageListVisibility": "show" });
        let label: Value = self.request(Method::POST, "/labels", &[], Some(&body)).await?;
        label["id"].as_str().map(String::from).ok_or_else(|| anyhow::anyhow!("Gmail made the label without an id"))
    }

    pub async fn watch(&self, state: &AppState) -> anyhow::Result<()> {
        let Some(topic) = &state.config.gmail_pubsub_topic else { return Ok(()) };
        let body = json!({ "topicName": topic, "labelFilterBehavior": "include", "labelIds": ["INBOX"] });
        let _: Value = self.request(Method::POST, "/watch", &[], Some(&body)).await?;
        Ok(())
    }
}

#[derive(Debug)]
struct NotFound;

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Gmail has no such item")
    }
}

impl std::error::Error for NotFound {}

fn remote(message: GmailMessage, labels: &HashMap<String, Uuid>) -> RemoteMessage {
    let lines: Vec<(String, String)> = message
        .payload
        .map(|payload| payload.headers.into_iter().map(|header| (header.name, header.value)).collect())
        .unwrap_or_default();
    let headers = mime::headers(&lines);
    let mut unread = false;
    let mut starred = false;
    let mut normalized = Vec::new();
    for label in &message.label_ids {
        match label.as_str() {
            "INBOX" => normalized.push(role::INBOX.to_string()),
            "SENT" => normalized.push(role::SENT.to_string()),
            "DRAFT" => normalized.push(role::DRAFTS.to_string()),
            "TRASH" => normalized.push(role::TRASH.to_string()),
            "SPAM" => normalized.push(role::SPAM.to_string()),
            "UNREAD" => unread = true,
            "STARRED" => starred = true,
            other => normalized.extend(labels.get(other).map(Uuid::to_string)),
        }
    }
    let date = message.internal_date.and_then(|date| date.parse().ok()).or(headers.date).unwrap_or(0);
    let bulk = headers.bulk || message.label_ids.iter().any(|label| BULK_CATEGORIES.contains(&label.as_str()));
    let attachments = match headers.multipart_mixed {
        true => vec![Attachment { name: String::new(), mime: String::new(), size: 0 }],
        false => Vec::new(),
    };
    RemoteMessage {
        provider_id: message.id,
        thread_id: message.thread_id,
        from: headers.from,
        recipients: headers.recipients,
        subject: headers.subject,
        snippet: one_line(&unescape(&message.snippet)),
        date,
        unread,
        starred,
        labels: normalized,
        attachments,
        message_id: headers.message_id,
        in_reply_to: headers.in_reply_to,
        references: headers.references,
        bulk,
        unsubscribe: headers.unsubscribe,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_labels_and_flags() {
        let custom = Uuid::new_v4();
        let labels = HashMap::from([("Label_7".to_string(), custom)]);
        let message: GmailMessage = serde_json::from_value(json!({
            "id": "m1", "threadId": "t1", "snippet": "Hi &amp; bye",
            "labelIds": ["INBOX", "UNREAD", "STARRED", "Label_7", "CATEGORY_PERSONAL"],
            "internalDate": "1700000000000",
            "payload": {"headers": [{"name": "From", "value": "Ann <ann@example.com>"}, {"name": "Subject", "value": "Hello"}]}
        }))
        .unwrap();
        let remote = remote(message, &labels);
        assert_eq!(remote.labels, ["inbox".to_string(), custom.to_string()]);
        assert!(remote.unread && remote.starred);
        assert_eq!(remote.snippet, "Hi & bye");
        assert_eq!(remote.date, 1_700_000_000_000);
        assert_eq!(remote.from.email, "ann@example.com");
        assert!(!remote.bulk);
    }

    #[test]
    fn promotions_and_lists_are_bulk() {
        let message = |labels: Value, headers: Value| -> GmailMessage {
            serde_json::from_value(
                json!({ "id": "m", "threadId": "t", "labelIds": labels, "payload": { "headers": headers } }),
            )
            .unwrap()
        };
        let promotion = remote(message(json!(["INBOX", "CATEGORY_PROMOTIONS"]), json!([])), &HashMap::new());
        assert!(promotion.bulk && promotion.unsubscribe.is_none());
        let headers = json!([{ "name": "List-Unsubscribe", "value": "<mailto:leave@list.org>" }]);
        let list = remote(message(json!(["INBOX"]), headers), &HashMap::new());
        assert!(list.bulk);
        assert_eq!(list.unsubscribe.unwrap().mailto.as_deref(), Some("mailto:leave@list.org"));
    }
}
