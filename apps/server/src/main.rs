mod api;
mod changes;
mod config;
mod db;
#[cfg(test)]
mod e2e;
mod error;
mod google;
mod hub;
mod mime;
mod ops;
mod providers;
mod scheduler;
mod seal;
mod seed;
mod sign_in;
mod sync;
mod workers;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::HeaderValue;
use axum::http::header::{REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS};
use axum::routing::{get, post};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;

pub struct Inner {
    pub config: Config,
    pub db: sqlx::PgPool,
    pub http: reqwest::Client,
    pub sealer: seal::Sealer,
    pub hub: hub::Hub,
    pub workers: workers::Workers,
    /// Set when the process stops, so that the LISTEN connection lets go of the database.
    pub stopping: tokio::sync::watch::Sender<bool>,
}

pub type AppState = Arc<Inner>;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the OS provides randomness");
    bytes
}

pub fn random_token() -> String {
    hex::encode(random_bytes::<32>())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn router(state: AppState) -> Router {
    let header = |name, value| SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/auth/google/start", get(sign_in::start))
        .route("/auth/google/callback", get(sign_in::callback))
        .route("/api/auth/exchange", post(api::exchange))
        .route("/api/link-ticket", post(api::link_ticket))
        .route("/api/accounts/jmap", post(api::add_jmap))
        .route("/api/dev/login", post(api::dev_login))
        .route("/hooks/gmail", post(api::gmail_hook))
        .route("/sync", get(sync::socket))
        .layer(header(X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header(X_FRAME_OPTIONS, "DENY"))
        .layer(header(REFERRER_POLICY, "no-referrer"))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Starts what runs beside the router: the LISTEN connection, the workers and the scheduler.
async fn start_background(state: &AppState) {
    hub::listen(state.clone());
    state.workers.start_all(state).await;
    scheduler::start(state.clone());
    let cleanup = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            if let Err(err) = db::cleanup(&cleanup.db).await {
                tracing::warn!("cleanup failed: {err}");
            }
        }
    });
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).build().expect("HTTP client")
}

async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.expect("ctrl-c handler") };
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {},
        _ = term.recv() => {},
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    mail_protocol::tls::install();

    if std::env::args().nth(1).as_deref() == Some("seed") {
        if let Err(error) = seed::run().await {
            eprintln!("seeding failed: {error:#}");
            std::process::exit(1);
        }
        return;
    }

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("config error: {err}");
        std::process::exit(1);
    });
    if config.dev_login {
        tracing::warn!("DEV_LOGIN is on: anyone can get a session. Never set it in production.");
    }
    if !config.google_enabled() {
        tracing::warn!("GOOGLE_CLIENT_ID or GOOGLE_CLIENT_SECRET is missing: Gmail accounts can't be added.");
    }
    let db = PgPoolOptions::new().max_connections(20).connect(&config.database_url).await.expect("connect to Postgres");
    sqlx::migrate!().run(&db).await.expect("run migrations");
    let state = Arc::new(Inner {
        sealer: seal::Sealer::new(&config.secret_key),
        config,
        db,
        http: http_client(),
        hub: Default::default(),
        workers: Default::default(),
        stopping: tokio::sync::watch::channel(false).0,
    });
    start_background(&state).await;

    let address = format!("0.0.0.0:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&address).await.expect("bind port");
    tracing::info!("Mail server listening on {address} as {}", state.config.public_url);
    axum::serve(listener, router(state)).with_graceful_shutdown(shutdown_signal()).await.expect("server");
}
