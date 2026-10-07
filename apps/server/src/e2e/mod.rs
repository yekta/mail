//! The server's real router on a fresh database per test. Google is played by a small server
//! that answers the token endpoint and enough of Gmail's API; Stalwart, when STALWART_URL is set,
//! is real.

mod changes;
mod sign_in;
mod stalwart;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::Form;
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::Url;
use reqwest::header::LOCATION;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::config::Config;
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
}

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
        config.gmail_api_url = format!("{google_base}/gmail");
        configure(&mut config);
        let state = Arc::new(Inner {
            sealer: seal::Sealer::new(&config.secret_key),
            config,
            background: crate::background_pool(&db),
            db,
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
        *self.google.lock().unwrap() = FakeGoogle { email: email.into(), nonce: query["nonce"].clone() };
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

    pub async fn user_of(&self, token: &str) -> Option<uuid::Uuid> {
        crate::api::user_of_token(&self.state, token).await.unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.state.stopping.send_replace(true);
        self.state.workers.stop_all();
    }
}

async fn fake_google(google: Arc<Mutex<FakeGoogle>>) -> String {
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
        .route("/gmail/profile", get(|| async { Json(json!({ "historyId": "1" })) }))
        .route("/gmail/labels", get(|| async { Json(json!({ "labels": [] })) }))
        .route("/gmail/messages", get(|| async { Json(json!({})) }))
        .route("/gmail/history", get(|| async { Json(json!({ "historyId": "1" })) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = gmail.route("/token", post(token));
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
