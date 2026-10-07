use std::env;
use std::time::Duration;

pub struct Config {
    pub public_url: String,
    pub database_url: String,
    pub port: u16,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub google_authorize_url: String,
    pub google_token_url: String,
    pub gmail_api_url: String,
    /// Seals provider credentials. Any text; the key is its SHA-256.
    pub secret_key: String,
    /// Lets anyone sign in as anyone. For tests and local work only.
    pub dev_login: bool,
    /// How many messages an account's first sync fetches at most; older mail is backfilled.
    pub initial_sync_limit: usize,
    /// The Pub/Sub topic Gmail pushes changes to; without it Gmail accounts are polled.
    pub gmail_pubsub_topic: Option<String>,
    /// The token Pub/Sub sends in `/hooks/gmail?token=`.
    pub gmail_hook_token: Option<String>,
    pub poll_interval: Duration,
    /// Runs the account workers. Off in tests that only look at the API.
    pub workers: bool,
}

fn required(name: &str) -> Result<String, String> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        _ => Err(format!("{name} is not set")),
    }
}

fn optional(name: &str) -> Option<String> {
    env::var(name).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let number = |name: &str, default: u64| -> Result<u64, String> {
            optional(name).map_or(Ok(default), |value| value.parse().map_err(|_| format!("{name} is not a number")))
        };
        Ok(Self {
            public_url: required("PUBLIC_URL")?.trim_end_matches('/').to_string(),
            database_url: required("DATABASE_URL")?,
            port: number("PORT", 3000)? as u16,
            google_client_id: optional("GOOGLE_CLIENT_ID").unwrap_or_default(),
            google_client_secret: optional("GOOGLE_CLIENT_SECRET").unwrap_or_default(),
            google_authorize_url: crate::google::AUTHORIZE_URL.to_string(),
            google_token_url: crate::google::TOKEN_URL.to_string(),
            gmail_api_url: crate::providers::gmail::API_URL.to_string(),
            secret_key: required("SECRET_KEY")?,
            dev_login: optional("DEV_LOGIN").is_some_and(|value| value == "1"),
            initial_sync_limit: number("INITIAL_SYNC_LIMIT", 2000)? as usize,
            gmail_pubsub_topic: optional("GMAIL_PUBSUB_TOPIC"),
            gmail_hook_token: optional("GMAIL_HOOK_TOKEN"),
            poll_interval: Duration::from_secs(number("POLL_SECONDS", 30)?),
            workers: true,
        })
    }

    pub fn google_enabled(&self) -> bool {
        !self.google_client_id.is_empty() && !self.google_client_secret.is_empty()
    }
}
