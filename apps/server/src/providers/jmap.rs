//! JMAP (RFC 8620, 8621): Stalwart, Fastmail and others. Mailboxes with a role are the system
//! mailboxes; the others are labels. Archiving moves a message from the inbox to the mailbox
//! with the archive role, which is made when there is none.

use std::collections::HashMap;
use std::sync::Arc;

use mail_protocol::{Address, Attachment, Draft, Identity, Op, Recipients, role};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::{Batch, Reauth, Refused, Synced, html_text, one_line};
use crate::db::{self, AccountRow, RemoteMessage};
use crate::mime::{File, ListHeaders};
use crate::{AppState, mime};

const CORE: &str = "urn:ietf:params:jmap:core";
const MAIL: &str = "urn:ietf:params:jmap:mail";
const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
const PAGE: usize = 250;
const EMAIL_PROPERTIES: [&str; 22] = [
    "id",
    "threadId",
    "mailboxIds",
    "keywords",
    "from",
    "to",
    "cc",
    "bcc",
    "replyTo",
    "subject",
    "preview",
    "receivedAt",
    "hasAttachment",
    "messageId",
    "inReplyTo",
    "references",
    "attachments",
    "header:List-Unsubscribe:asURLs",
    "header:List-Unsubscribe-Post:asText",
    "header:List-Id:asText",
    "header:Precedence:asText",
    "header:Auto-Submitted:asText",
];

/// Where an account logs in: kept in the account's `login` column.
#[derive(Serialize, Deserialize)]
pub struct Login {
    pub url: String,
    pub username: String,
}

#[derive(Clone)]
pub struct Jmap {
    http: reqwest::Client,
    session: Arc<Session>,
    /// The server says how many requests it takes at once; more are refused.
    permits: Arc<Semaphore>,
}

struct Session {
    authorization: String,
    api_url: String,
    upload_url: String,
    download_url: String,
    account_id: String,
    username: String,
}

#[derive(Deserialize, Serialize, Default)]
struct SyncState {
    email_state: Option<String>,
    mailbox_state: Option<String>,
    #[serde(default)]
    synced: usize,
    #[serde(default)]
    backfilled: bool,
    /// The newest pages were fetched again for what was added after they were first synced
    /// (`bulk`, `unsubscribe`), keeping the Email state.
    #[serde(default)]
    refreshed: bool,
}

/// The account's mailboxes: which one has each role, and the others as labels.
struct Mailboxes {
    by_id: HashMap<String, String>,
    labels: HashMap<String, Uuid>,
}

impl Jmap {
    /// Finds the session from the server's address. A username left empty sends the password
    /// as a bearer token, as Fastmail's API tokens are used.
    pub async fn open(http: &reqwest::Client, url: &str, username: &str, password: &str) -> anyhow::Result<Self> {
        let authorization = match username.is_empty() {
            true => format!("Bearer {password}"),
            false => {
                use base64::Engine;
                format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}")))
            }
        };
        let base = url.trim_end_matches('/');
        let origin = reqwest::Url::parse(base)?;
        let well_known = origin.join("/.well-known/jmap")?;
        let mut session = None;
        for candidate in [well_known.as_str(), base] {
            let response = http.get(candidate).header("Authorization", &authorization).send().await?;
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(Reauth("the username or password is wrong".into()).into());
            }
            if !response.status().is_success() {
                continue;
            }
            let Ok(found) = response.json::<Value>().await else { continue };
            if found["apiUrl"].is_string() {
                session = Some(found);
                break;
            }
        }
        let Some(session) = session else {
            anyhow::bail!("There's no JMAP server at {url}");
        };
        let text = |key: &str| session[key].as_str().unwrap_or_default().to_string();
        let account_id = session["primaryAccounts"][MAIL].as_str().unwrap_or_default().to_string();
        if account_id.is_empty() {
            anyhow::bail!("This JMAP server has no mail account for {username}");
        }
        let origin_text = origin.origin().ascii_serialization();
        let resolve = |value: String| if value.starts_with('/') { format!("{origin_text}{value}") } else { value };
        let concurrent = session["capabilities"][CORE]["maxConcurrentRequests"].as_u64().unwrap_or(4).clamp(1, 16);
        Ok(Self {
            http: http.clone(),
            permits: Arc::new(Semaphore::new(concurrent as usize)),
            session: Arc::new(Session {
                authorization,
                api_url: resolve(text("apiUrl")),
                upload_url: resolve(text("uploadUrl")),
                download_url: resolve(text("downloadUrl")),
                account_id,
                username: text("username"),
            }),
        })
    }

    /// The address mail is sent from: the first identity, else the login.
    pub async fn address(&self) -> anyhow::Result<Address> {
        let identities = self.identity_list().await.unwrap_or_default();
        if let Some(identity) = identities.first() {
            return Ok(Address::new(identity["name"].as_str(), identity["email"].as_str().unwrap_or_default()));
        }
        Ok(Address::new(None, &self.session.username))
    }

    async fn identity_list(&self) -> anyhow::Result<Vec<Value>> {
        let responses = self.call(&[CORE, MAIL, SUBMISSION], vec![("Identity/get", json!({}))]).await?;
        Ok(responses[0]["list"].as_array().cloned().unwrap_or_default())
    }

    /// The account's identities, the login's own first.
    pub async fn identities(&self) -> anyhow::Result<Vec<Identity>> {
        let mut identities: Vec<Identity> = self
            .identity_list()
            .await?
            .iter()
            .filter_map(|identity| {
                let address = Address::new(identity["name"].as_str(), identity["email"].as_str()?);
                let text = identity["textSignature"].as_str().map(str::trim).filter(|text| !text.is_empty());
                let signature = text
                    .map(String::from)
                    .or_else(|| identity["htmlSignature"].as_str().map(html_text))
                    .filter(|text| !text.is_empty());
                Some(Identity { name: address.name, email: address.email, signature })
            })
            .collect();
        identities.sort_by_key(|identity| !identity.email.eq_ignore_ascii_case(&self.session.username));
        Ok(identities)
    }

    pub async fn create_label(&self, name: &str) -> anyhow::Result<String> {
        let create = json!({ "create": { "l": { "name": name, "parentId": null } } });
        let answers = self.call(&[CORE, MAIL], vec![("Mailbox/set", create)]).await?;
        if let Some(id) = answers[0]["created"]["l"]["id"].as_str() {
            return Ok(id.to_string());
        }
        let failed = &answers[0]["notCreated"]["l"];
        let kind = failed["type"].as_str().unwrap_or("serverFail");
        if ["serverFail", "serverUnavailable", "serverPartialFail", "rateLimit"].contains(&kind) {
            anyhow::bail!("the JMAP server couldn't make the label now: {failed}");
        }
        Err(Refused(format!("the JMAP server won't make the label: {failed}")).into())
    }

    /// Runs method calls in one request. Each gets the account id; the answers come in order.
    pub async fn call(&self, using: &[&str], calls: Vec<(&str, Value)>) -> anyhow::Result<Vec<Value>> {
        let method_calls: Vec<Value> = calls
            .into_iter()
            .enumerate()
            .map(|(index, (name, mut arguments))| {
                arguments["accountId"] = json!(self.session.account_id);
                json!([name, arguments, index.to_string()])
            })
            .collect();
        let _permit = self.permits.acquire().await?;
        let response = self
            .http
            .post(&self.session.api_url)
            .header("Authorization", &self.session.authorization)
            .json(&json!({ "using": using, "methodCalls": method_calls }))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(Reauth("the password is no longer accepted".into()).into());
        }
        if !response.status().is_success() {
            let status = response.status();
            anyhow::bail!("the JMAP server answered {status}: {}", response.text().await.unwrap_or_default());
        }
        let body: Value = response.json().await?;
        let mut answers = Vec::new();
        for answer in body["methodResponses"].as_array().into_iter().flatten() {
            if answer[0] == "error" {
                let kind = answer[1]["type"].as_str().unwrap_or("error");
                return Err(MethodError(kind.to_string()).into());
            }
            answers.push(answer[1].clone());
        }
        Ok(answers)
    }

    pub async fn upload(&self, raw: Vec<u8>) -> anyhow::Result<String> {
        let url = self.session.upload_url.replace("{accountId}", &self.session.account_id);
        let _permit = self.permits.acquire().await?;
        let response = self
            .http
            .post(url)
            .header("Authorization", &self.session.authorization)
            .header("Content-Type", "message/rfc822")
            .body(raw)
            .send()
            .await?
            .error_for_status()?;
        let body: Value = response.json().await?;
        body["blobId"].as_str().map(String::from).ok_or_else(|| anyhow::anyhow!("the upload has no blob id"))
    }

    /// Imports uploaded messages; returns the new email ids in order.
    pub async fn import(&self, emails: Vec<Value>) -> anyhow::Result<Vec<String>> {
        let keys: Vec<String> = (0..emails.len()).map(|index| format!("e{index}")).collect();
        let map: Map<String, Value> = keys.iter().cloned().zip(emails).collect();
        let answers = self.call(&[CORE, MAIL], vec![("Email/import", json!({ "emails": map }))]).await?;
        if let Some(failed) = answers[0]["notCreated"].as_object().and_then(|failed| failed.values().next()) {
            anyhow::bail!("the JMAP server refused an import: {failed}");
        }
        Ok(keys.iter().filter_map(|key| answers[0]["created"][key]["id"].as_str().map(String::from)).collect())
    }

    async fn mailboxes(
        &self,
        state: &AppState,
        account: &AccountRow,
        sync: &mut SyncState,
    ) -> anyhow::Result<Mailboxes> {
        let answers = self.call(&[CORE, MAIL], vec![("Mailbox/get", json!({ "ids": null }))]).await?;
        let mut list = answers[0]["list"].as_array().cloned().unwrap_or_default();
        if !list.iter().any(|mailbox| mailbox["role"] == "archive") {
            let created = self
                .call(
                    &[CORE, MAIL],
                    vec![("Mailbox/set", json!({ "create": { "a": { "name": "Archive", "role": "archive" } } }))],
                )
                .await?;
            if let Some(id) = created[0]["created"]["a"]["id"].as_str() {
                list.push(json!({ "id": id, "name": "Archive", "role": "archive" }));
            }
        }
        sync.mailbox_state = answers[0]["state"].as_str().map(String::from);

        let mut by_id = HashMap::new();
        let mut custom = Vec::new();
        for mailbox in &list {
            let Some(id) = mailbox["id"].as_str() else { continue };
            let name = mailbox["name"].as_str().unwrap_or_default();
            let mapped = match mailbox["role"].as_str() {
                Some("inbox") => Some(role::INBOX),
                Some("sent") => Some(role::SENT),
                Some("drafts") => Some(role::DRAFTS),
                Some("trash") => Some(role::TRASH),
                Some("junk") => Some(role::SPAM),
                Some("archive") => Some("archive"),
                _ => None,
            };
            match mapped {
                Some(role) => {
                    by_id.insert(id.to_string(), role.to_string());
                }
                None => custom.push((id.to_string(), name.to_string())),
            }
        }
        let labels = db::sync_labels(&state.db, account, &custom).await?;
        Ok(Mailboxes { by_id, labels })
    }

    pub async fn sync(&self, state: &AppState, account: &AccountRow) -> anyhow::Result<Synced> {
        let mut sync: SyncState = serde_json::from_value(account.sync_state.clone()).unwrap_or_default();
        let mailboxes = self.mailboxes(state, account, &mut sync).await?;

        match sync.email_state.clone() {
            None => {
                let limit = state.config.initial_sync_limit;
                sync.refreshed = true;
                while sync.synced < limit {
                    let wanted = PAGE.min(limit - sync.synced);
                    let (count, email_state) = self.page(state, account, &mailboxes, sync.synced, wanted).await?;
                    sync.email_state = sync.email_state.or(email_state);
                    sync.synced += count;
                    if count < wanted {
                        sync.backfilled = true;
                        break;
                    }
                }
            }
            Some(since) => match self.changes(state, account, &mailboxes, &since).await {
                Ok(latest) => sync.email_state = Some(latest),
                Err(error) if error.downcast_ref::<MethodError>().is_some_and(|e| e.0 == "cannotCalculateChanges") => {
                    sync = SyncState { mailbox_state: sync.mailbox_state, ..Default::default() };
                }
                Err(error) => return Err(error),
            },
        }

        if !sync.refreshed && sync.email_state.is_some() {
            let limit = state.config.initial_sync_limit;
            let mut position = 0;
            while position < limit {
                let wanted = PAGE.min(limit - position);
                let (count, _) = self.page(state, account, &mailboxes, position, wanted).await?;
                position += count;
                if count < wanted {
                    break;
                }
            }
            sync.refreshed = true;
        }
        if !sync.backfilled && sync.email_state.is_some() {
            let (count, _) = self.page(state, account, &mailboxes, sync.synced, PAGE).await?;
            sync.synced += count;
            sync.backfilled = count < PAGE;
        }
        db::save_sync_state(&state.db, account.id, &json!(sync)).await?;
        Ok(Synced { backfilling: !sync.backfilled })
    }

    /// One page of the newest-first listing. Returns how many it had and the Email state.
    async fn page(
        &self,
        state: &AppState,
        account: &AccountRow,
        mailboxes: &Mailboxes,
        position: usize,
        limit: usize,
    ) -> anyhow::Result<(usize, Option<String>)> {
        let answers = self
            .call(
                &[CORE, MAIL],
                vec![
                    (
                        "Email/query",
                        json!({ "sort": [{ "property": "receivedAt", "isAscending": false }], "position": position, "limit": limit }),
                    ),
                    (
                        "Email/get",
                        json!({ "#ids": { "resultOf": "0", "name": "Email/query", "path": "/ids" }, "properties": EMAIL_PROPERTIES }),
                    ),
                ],
            )
            .await?;
        let list = answers[1]["list"].as_array().cloned().unwrap_or_default();
        let messages: Vec<RemoteMessage> = list.iter().map(|email| remote(email, mailboxes)).collect();
        db::upsert_messages(&state.db, account, &messages).await?;
        Ok((list.len(), answers[1]["state"].as_str().map(String::from)))
    }

    async fn changes(
        &self,
        state: &AppState,
        account: &AccountRow,
        mailboxes: &Mailboxes,
        since: &str,
    ) -> anyhow::Result<String> {
        let mut since = since.to_string();
        loop {
            let answers = self
                .call(
                    &[CORE, MAIL],
                    vec![
                        ("Email/changes", json!({ "sinceState": since, "maxChanges": 500 })),
                        (
                            "Email/get",
                            json!({ "#ids": { "resultOf": "0", "name": "Email/changes", "path": "/created" }, "properties": EMAIL_PROPERTIES }),
                        ),
                        (
                            "Email/get",
                            json!({ "#ids": { "resultOf": "0", "name": "Email/changes", "path": "/updated" }, "properties": EMAIL_PROPERTIES }),
                        ),
                    ],
                )
                .await?;
            let messages: Vec<RemoteMessage> = [&answers[1], &answers[2]]
                .iter()
                .flat_map(|answer| answer["list"].as_array().cloned().unwrap_or_default())
                .map(|email| remote(&email, mailboxes))
                .collect();
            db::upsert_messages(&state.db, account, &messages).await?;
            let destroyed: Vec<String> = answers[0]["destroyed"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|id| id.as_str().map(String::from))
                .collect();
            db::delete_messages(&state.db, account, &destroyed).await?;
            since = answers[0]["newState"].as_str().unwrap_or(&since).to_string();
            if answers[0]["hasMoreChanges"] != true {
                return Ok(since);
            }
        }
    }

    pub async fn apply(&self, batch: &Batch, labels: &HashMap<Uuid, String>) -> anyhow::Result<()> {
        let answers = self
            .call(&[CORE, MAIL], vec![("Mailbox/get", json!({ "ids": null, "properties": ["id", "role"] }))])
            .await?;
        let role_id = |wanted: &str| {
            answers[0]["list"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|mailbox| mailbox["role"] == wanted)
                .and_then(|mailbox| mailbox["id"].as_str())
                .map(String::from)
        };
        let label = |id: &String| match id.as_str() {
            role::INBOX => role_id("inbox"),
            role::TRASH => role_id("trash"),
            role::SPAM => role_id("junk"),
            _ => id.parse::<Uuid>().ok().and_then(|id| labels.get(&id).cloned()),
        };
        let only = |mailbox: Option<String>| -> Vec<(String, Value)> {
            mailbox.map(|id| vec![("mailboxIds".to_string(), json!({ id: true }))]).unwrap_or_default()
        };
        // Out of the inbox and into the archive, so the message is never in no mailbox.
        let archive = || {
            let mut patch = Vec::new();
            patch.extend(role_id("inbox").map(|id| (format!("mailboxIds/{id}"), Value::Null)));
            patch.extend(role_id("archive").map(|id| (format!("mailboxIds/{id}"), json!(true))));
            patch
        };
        let patch: Vec<(String, Value)> = match &batch.op {
            Op::SetUnread { unread, .. } => {
                vec![("keywords/$seen".into(), if *unread { Value::Null } else { json!(true) })]
            }
            Op::SetStarred { starred, .. } => {
                vec![("keywords/$flagged".into(), if *starred { json!(true) } else { Value::Null })]
            }
            Op::Archive { .. } | Op::Snooze { .. } => archive(),
            // Keeps Sent and the labels, so mail that was only sent comes back to the inbox whole.
            Op::MoveToInbox { .. } => {
                let mut patch: Vec<(String, Value)> = ["trash", "junk", "archive"]
                    .into_iter()
                    .filter_map(|wanted| role_id(wanted).map(|id| (format!("mailboxIds/{id}"), Value::Null)))
                    .collect();
                patch.extend(role_id("inbox").map(|id| (format!("mailboxIds/{id}"), json!(true))));
                patch
            }
            Op::Trash { .. } => only(role_id("trash")),
            Op::Spam { .. } => only(role_id("junk")),
            Op::AddLabel { label: id, .. } => {
                label(id).map(|id| vec![(format!("mailboxIds/{id}"), json!(true))]).unwrap_or_default()
            }
            Op::RemoveLabel { label: id, .. } if id == role::INBOX => archive(),
            Op::RemoveLabel { label: id, .. } => {
                label(id).map(|id| vec![(format!("mailboxIds/{id}"), Value::Null)]).unwrap_or_default()
            }
            _ => return Ok(()),
        };
        if patch.is_empty() {
            return Ok(());
        }
        let patch: Map<String, Value> = patch.into_iter().collect();
        for ids in batch.provider_ids.chunks(200) {
            let update: Map<String, Value> = ids.iter().map(|id| (id.clone(), Value::Object(patch.clone()))).collect();
            let answers = self.call(&[CORE, MAIL], vec![("Email/set", json!({ "update": update }))]).await?;
            if let Some(failed) = answers[0]["notUpdated"].as_object().filter(|failed| !failed.is_empty()) {
                tracing::warn!("the JMAP server didn't update {} messages: {:?}", failed.len(), failed.values().next());
            }
        }
        Ok(())
    }

    pub async fn raw(&self, provider_id: &str) -> anyhow::Result<Vec<u8>> {
        let answers = self
            .call(&[CORE, MAIL], vec![("Email/get", json!({ "ids": [provider_id], "properties": ["blobId"] }))])
            .await?;
        let Some(blob) = answers[0]["list"][0]["blobId"].as_str() else {
            anyhow::bail!("the JMAP server has no message {provider_id}");
        };
        let url = self
            .session
            .download_url
            .replace("{accountId}", &self.session.account_id)
            .replace("{blobId}", blob)
            .replace("{name}", "message.eml")
            .replace("{type}", "message/rfc822");
        let _permit = self.permits.acquire().await?;
        let response =
            self.http.get(url).header("Authorization", &self.session.authorization).send().await?.error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    }

    pub async fn send(&self, draft: &Draft, from: &Address, files: &[File]) -> anyhow::Result<Option<String>> {
        let identities = self.identity_list().await?;
        let identity = identities
            .iter()
            .find(|identity| identity["email"].as_str().is_some_and(|email| email.eq_ignore_ascii_case(&from.email)))
            .or(identities.first())
            .and_then(|identity| identity["id"].as_str())
            .ok_or_else(|| anyhow::anyhow!("this account has no identity to send from"))?
            .to_string();
        let domain = from.email.split('@').nth(1).unwrap_or("localhost");
        let raw = mime::build(draft, from, &mime::new_message_id(domain), false, files);
        let blob = self.upload(raw).await?;
        let mailboxes = self
            .call(&[CORE, MAIL], vec![("Mailbox/get", json!({ "ids": null, "properties": ["id", "role"] }))])
            .await?;
        let sent = mailboxes[0]["list"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|mailbox| mailbox["role"] == "sent")
            .and_then(|mailbox| mailbox["id"].as_str())
            .ok_or_else(|| anyhow::anyhow!("this account has no Sent mailbox"))?
            .to_string();
        let ids = self
            .import(vec![json!({ "blobId": blob, "mailboxIds": { sent: true }, "keywords": { "$seen": true } })])
            .await?;
        let Some(email_id) = ids.first() else {
            anyhow::bail!("the JMAP server didn't keep the message");
        };
        let recipients: Vec<Value> = draft
            .to
            .iter()
            .chain(&draft.cc)
            .chain(&draft.bcc)
            .map(|address| json!({ "email": address.email }))
            .collect();
        let submission = json!({
            "create": { "s": {
                "emailId": email_id,
                "identityId": identity,
                "envelope": { "mailFrom": { "email": from.email }, "rcptTo": recipients },
            }}
        });
        let answers = self.call(&[CORE, MAIL, SUBMISSION], vec![("EmailSubmission/set", submission)]).await?;
        if let Some(failed) = answers[0]["notCreated"]["s"].as_object() {
            anyhow::bail!(
                "the JMAP server refused to send it: {}",
                failed.get("description").or(failed.get("type")).unwrap_or(&Value::Null)
            );
        }
        Ok(Some(email_id.clone()))
    }
}

#[derive(Debug)]
pub struct MethodError(pub String);

impl std::fmt::Display for MethodError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the JMAP server answered {}", self.0)
    }
}

impl std::error::Error for MethodError {}

fn addresses(value: &Value) -> Vec<Address> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|address| Some(Address::new(address["name"].as_str(), address["email"].as_str()?)))
        .collect()
}

fn remote(email: &Value, mailboxes: &Mailboxes) -> RemoteMessage {
    let mut labels = Vec::new();
    for (id, _) in email["mailboxIds"].as_object().into_iter().flatten() {
        match mailboxes.by_id.get(id) {
            Some(role) if role == "archive" => {}
            Some(role) => labels.push(role.clone()),
            None => labels.extend(mailboxes.labels.get(id).map(Uuid::to_string)),
        }
    }
    let first =
        |key: &str| email[key].as_array().and_then(|list| list.first()).and_then(Value::as_str).map(String::from);
    let attachments = email["attachments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["disposition"] != "inline")
        .map(|part| Attachment {
            name: part["name"].as_str().unwrap_or("Attachment").to_string(),
            mime: part["type"].as_str().unwrap_or_default().to_string(),
            size: part["size"].as_i64().unwrap_or(0),
        })
        .collect();
    let text = |key: &str| email[key].as_str();
    let list = ListHeaders {
        unsubscribe: email["header:List-Unsubscribe:asURLs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|url| url.as_str().map(String::from))
            .collect(),
        unsubscribe_post: text("header:List-Unsubscribe-Post:asText"),
        list_id: text("header:List-Id:asText"),
        precedence: text("header:Precedence:asText"),
        auto_submitted: text("header:Auto-Submitted:asText"),
    };
    let date = email["receivedAt"]
        .as_str()
        .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
        .map(|date| date.timestamp_millis())
        .unwrap_or(0);
    RemoteMessage {
        provider_id: email["id"].as_str().unwrap_or_default().to_string(),
        thread_id: email["threadId"].as_str().unwrap_or_default().to_string(),
        from: addresses(&email["from"]).into_iter().next().unwrap_or_default(),
        recipients: Recipients {
            to: addresses(&email["to"]),
            cc: addresses(&email["cc"]),
            bcc: addresses(&email["bcc"]),
            reply_to: addresses(&email["replyTo"]),
        },
        subject: email["subject"].as_str().unwrap_or_default().to_string(),
        snippet: one_line(email["preview"].as_str().unwrap_or_default()),
        date,
        unread: email["keywords"]["$seen"] != true,
        starred: email["keywords"]["$flagged"] == true,
        labels,
        attachments,
        message_id: first("messageId"),
        in_reply_to: first("inReplyTo"),
        references: email["references"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|id| id.as_str().map(String::from))
            .collect(),
        bulk: list.bulk(),
        unsubscribe: list.unsubscribe(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_mailboxes_to_roles_and_labels() {
        let work = Uuid::new_v4();
        let mailboxes = Mailboxes {
            by_id: HashMap::from([("i".into(), "inbox".into()), ("a".into(), "archive".into())]),
            labels: HashMap::from([("w".into(), work)]),
        };
        let email = json!({
            "id": "e1", "threadId": "t1", "mailboxIds": {"i": true, "w": true},
            "keywords": {"$flagged": true}, "from": [{"name": "Ann", "email": "ann@example.com"}],
            "subject": "Hi", "preview": "Hello", "receivedAt": "2024-03-03T09:41:00Z",
            "attachments": [{"name": "a.pdf", "type": "application/pdf", "size": 10, "disposition": "attachment"}]
        });
        let remote = remote(&email, &mailboxes);
        let mut labels = remote.labels.clone();
        labels.sort();
        let mut expected = vec!["inbox".to_string(), work.to_string()];
        expected.sort();
        assert_eq!(labels, expected);
        assert!(remote.unread && remote.starred);
        assert_eq!(remote.attachments.len(), 1);

        let archived =
            super::remote(&json!({"id": "e2", "mailboxIds": {"a": true}, "keywords": {"$seen": true}}), &mailboxes);
        assert!(archived.labels.is_empty() && !archived.unread);
        assert!(!remote.bulk && remote.unsubscribe.is_none());

        let newsletter = super::remote(
            &json!({
                "id": "e3", "mailboxIds": {"i": true},
                "header:List-Unsubscribe:asURLs": ["mailto:leave@list.org", "https://list.org/leave"],
                "header:List-Unsubscribe-Post:asText": "List-Unsubscribe=One-Click"
            }),
            &mailboxes,
        );
        assert!(newsletter.bulk);
        let unsubscribe = newsletter.unsubscribe.unwrap();
        assert_eq!(unsubscribe.url.as_deref(), Some("https://list.org/leave"));
        assert!(unsubscribe.one_click);
    }
}
