//! The server's real router on a fresh database per test. Google is played by a small server
//! that answers the token endpoint and enough of Gmail's API; Stalwart, when STALWART_URL is set,
//! is real.

mod attachments;
mod changes;
mod rules;
mod search;
mod sign_in;
mod stalwart;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Path};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use mail_protocol::Provider;
use reqwest::Url;
use reqwest::header::LOCATION;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use crate::config::Config;
use crate::db::{self, AccountRow, NewAccount};
use crate::{AppState, Inner, google, router, seal};

const GOOGLE_CLIENT_ID: &str = "google-client";

pub struct Server {
    pub base: String,
    pub state: AppState,
    /// Follows no redirects, to look at where the browser would be sent.
    http: reqwest::Client,
    google: Arc<Mutex<FakeGoogle>>,
}

#[derive(Default)]
struct FakeGoogle {
    email: String,
    nonce: String,
    /// The raw messages Gmail was asked to send.
    sent: Vec<String>,
    /// The bodies of batchModify, as Gmail was asked to change labels.
    modified: Vec<Value>,
    /// How many sends Gmail refuses next, saying the user's quota is spent.
    refused_sends: usize,
}

/// What every message at the fake Gmail is: a note with a PDF attached.
pub const RAW_MESSAGE: &str = "From: Cy <cy@example.com>\r\nTo: ann@gmail.com\r\nSubject: Plan\r\n\
    Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
    --b\r\nContent-Type: text/plain\r\n\r\nHere it is.\r\n\
    --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\n\r\n%PDF-1.4\r\n\
    --b--\r\n";

pub fn config(base: &str) -> Config {
    Config {
        public_url: base.to_string(),
        database_url: String::new(),
        port: 0,
        google_client_id: GOOGLE_CLIENT_ID.into(),
        google_client_secret: "google-secret".into(),
        google_authorize_url: google::AUTHORIZE_URL.into(),
        google_token_url: String::new(),
        gmail_api_url: String::new(),
        secret_key: "test-secret".into(),
        dev_login: true,
        initial_sync_limit: 200,
        gmail_pubsub_topic: None,
        gmail_hook_token: Some("hook-token".into()),
        poll_interval: std::time::Duration::from_millis(300),
        workers: false,
    }
}

impl Server {
    pub async fn start(db: PgPool) -> Self {
        Self::start_with(db, |_| {}).await
    }

    pub async fn start_with(db: PgPool, configure: impl FnOnce(&mut Config)) -> Self {
        mail_protocol::tls::install();
        let google = Arc::new(Mutex::new(FakeGoogle::default()));
        let google_base = fake_google(google.clone()).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut config = config(&base);
        config.google_token_url = format!("{google_base}/token");
        config.gmail_api_url = format!("{google_base}/gmail/v1/users/me");
        configure(&mut config);
        // A pool of the server's own: a test's pool keeps the connections it opened counted against
        // one limit shared by every test, and tests starting together would wait on each other.
        let server_db = PgPoolOptions::new().max_connections(5).connect_lazy_with((*db.connect_options()).clone());
        let state = Arc::new(Inner {
            sealer: seal::Sealer::new(&config.secret_key),
            config,
            background: crate::background_pool(&db),
            db: server_db,
            http: reqwest::Client::new(),
            hub: Default::default(),
            workers: Default::default(),
            stopping: tokio::sync::watch::channel(false).0,
        });
        crate::start_background(&state).await;
        let app = router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        Self { base, state, http, google }
    }

    /// Goes through Google as `email` and returns the `mailapp://auth` URL the app is sent to.
    pub async fn google_sign_in(&self, verifier: &str, ticket: Option<&str>, email: &str) -> Url {
        let challenge = crate::sha256_hex(verifier.as_bytes());
        let mut start = format!("{}/auth/google/start?challenge={challenge}&state=app-state", self.base);
        if let Some(ticket) = ticket {
            start.push_str(&format!("&ticket={ticket}"));
        }
        let response = self.http.get(&start).send().await.unwrap();
        assert!(response.status().is_redirection(), "{}", response.text().await.unwrap());
        let google = Url::parse(response.headers()[LOCATION].to_str().unwrap()).unwrap();
        let query: HashMap<String, String> = google.query_pairs().into_owned().collect();
        assert_eq!(query["scope"], google::SCOPES);
        assert_eq!(query["access_type"], "offline");
        {
            let mut google = self.google.lock().unwrap();
            (google.email, google.nonce) = (email.into(), query["nonce"].clone());
        }
        let callback = format!("{}/auth/google/callback?code=google-code&state={}", self.base, query["state"]);
        let response = self.http.get(callback).send().await.unwrap();
        assert!(response.status().is_redirection(), "{}", response.text().await.unwrap());
        Url::parse(response.headers()[LOCATION].to_str().unwrap()).unwrap()
    }

    pub async fn post(&self, path: &str, token: Option<&str>, body: Value) -> reqwest::Response {
        let mut request = self.http.post(format!("{}{path}", self.base)).json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    pub async fn get(&self, path: &str, token: Option<&str>) -> reqwest::Response {
        let mut request = self.http.get(format!("{}{path}", self.base));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    pub async fn user_of(&self, token: &str) -> Option<uuid::Uuid> {
        crate::api::user_of_token(&self.state, token).await.unwrap()
    }

    /// A user signed in with the dev login, with a Gmail account the fake Google plays.
    pub async fn gmail_user(&self, email: &str) -> (String, Uuid, AccountRow) {
        let response = self.post("/api/dev/login", None, json!({ "email": email })).await;
        let token = response.json::<Value>().await.unwrap()["token"].as_str().unwrap().to_string();
        let user_id = self.user_of(&token).await.unwrap();
        let credentials = self.state.sealer.seal(json!({ "refresh_token": "refresh" }).to_string().as_bytes());
        let account = NewAccount { user_id, provider: Provider::Gmail, address: email, login: "", credentials };
        let account_id = db::upsert_account(&self.state.db, account).await.unwrap();
        (token, user_id, db::account(&self.state.db, account_id).await.unwrap().unwrap())
    }

    pub fn sent(&self) -> Vec<String> {
        self.google.lock().unwrap().sent.clone()
    }

    pub fn refuse_sends(&self, count: usize) {
        self.google.lock().unwrap().refused_sends = count;
    }

    pub fn modified(&self) -> Vec<Value> {
        self.google.lock().unwrap().modified.clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.state.stopping.send_replace(true);
        self.state.workers.stop_all();
        let db = self.state.db.clone();
        tokio::spawn(async move { db.close().await });
    }
}

async fn fake_google(google: Arc<Mutex<FakeGoogle>>) -> String {
    let changes = google.clone();
    let modify = move |Json(body): Json<Value>| {
        changes.lock().unwrap().modified.push(body);
        async { Json(json!({})) }
    };
    // A label named "Taken" exists already; "Busy" finds Gmail unavailable.
    let create_label = |Json(body): Json<Value>| async move {
        match body["name"].as_str().unwrap_or_default() {
            "Taken" => (axum::http::StatusCode::CONFLICT, Json(json!({}))),
            "Busy" => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(json!({}))),
            name => (axum::http::StatusCode::OK, Json(json!({ "id": format!("Label_{name}") }))),
        }
    };
    let outbox = google.clone();
    // The multipart upload: the metadata, then the message.
    let send = move |body: String| {
        let message = body.split_once("Content-Type: message/rfc822\r\n\r\n").unwrap_or_default().1;
        let message = message.rsplit_once("\r\n--").unwrap_or_default().0;
        let mut google = outbox.lock().unwrap();
        let answer = if google.refused_sends > 0 {
            google.refused_sends -= 1;
            let reason =
                json!({ "error": { "status": "PERMISSION_DENIED", "errors": [{ "reason": "rateLimitExceeded" }] } });
            (axum::http::StatusCode::FORBIDDEN, Json(reason))
        } else {
            google.sent.push(message.to_string());
            let id = format!("sent-{}", google.sent.len());
            (axum::http::StatusCode::OK, Json(json!({ "id": id, "threadId": "sent-thread" })))
        };
        async move { answer }
    };
    let message =
        |Path(id): Path<String>| async move { Json(json!({ "id": id, "raw": URL_SAFE_NO_PAD.encode(RAW_MESSAGE) })) };
    let send_as = || async {
        Json(json!({ "sendAs": [
            { "sendAsEmail": "ann@gmail.com", "displayName": "Ann Lee", "isPrimary": true, "isDefault": true },
            { "sendAsEmail": "ann@work.com", "displayName": "Ann at work", "verificationStatus": "accepted" },
            { "sendAsEmail": "ann@unverified.com", "verificationStatus": "pending" },
        ]}))
    };
    let token = move |Form(form): Form<HashMap<String, String>>| {
        let google = google.lock().unwrap();
        let answer = match form.get("grant_type").map(String::as_str) {
            Some("refresh_token") => json!({ "access_token": "access", "expires_in": 3600 }),
            _ => {
                let claims = json!({
                    "iss": "https://accounts.google.com",
                    "aud": GOOGLE_CLIENT_ID,
                    "exp": chrono::Utc::now().timestamp() + 60,
                    "nonce": google.nonce,
                    "email": google.email,
                    "email_verified": true,
                });
                json!({
                    "id_token": google::id_token(&claims),
                    "access_token": "access",
                    "refresh_token": format!("refresh-{}", google.email),
                    "scope": google::SCOPES,
                })
            }
        };
        async move { Json(answer) }
    };
    let gmail = Router::new()
        .route("/profile", get(|| async { Json(json!({ "historyId": "1" })) }))
        .route("/labels", get(|| async { Json(json!({ "labels": [] })) }).post(create_label))
        .route("/messages/batchModify", post(modify))
        .route("/messages", get(|| async { Json(json!({})) }))
        .route("/history", get(|| async { Json(json!({ "historyId": "1" })) }))
        .route("/messages/{id}", get(message))
        .route("/settings/sendAs", get(send_as));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .nest("/gmail/v1/users/me", gmail)
        .route("/upload/gmail/v1/users/me/messages/send", post(send))
        .route("/token", post(token));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

/// The code and state in the URL the app is sent back to.
pub fn code_of(url: &Url) -> String {
    assert_eq!((url.scheme(), url.host_str()), ("mailapp", Some("auth")));
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(query["state"], "app-state");
    query["code"].clone()
}
