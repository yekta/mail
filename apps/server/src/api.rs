//! What the apps call over HTTP. A signed-in app sends its session's `Bearer` token.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use mail_protocol::Provider;
use mail_protocol::api::{
    DevLoginRequest, ExchangeRequest, JmapAccountRequest, LinkTicket, TokenResponse, UploadResponse,
};
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::db::NewAccount;
use crate::error::{AppError, AppResult};
use crate::providers::Reauth;
use crate::providers::jmap::{Jmap, Login};
use crate::{AppState, db, mime, random_token, sha256_hex, workers};

/// The largest file `POST /api/uploads` takes.
pub const UPLOAD_LIMIT: usize = 25 * 1024 * 1024;

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get("authorization")?.to_str().ok()?.strip_prefix("Bearer ")
}

pub async fn user_of_token(state: &AppState, token: &str) -> sqlx::Result<Option<Uuid>> {
    db::user_of_session(&state.db, &sha256_hex(token.as_bytes())).await
}

async fn signed_in(state: &AppState, headers: &HeaderMap) -> AppResult<Option<Uuid>> {
    let Some(token) = bearer(headers) else { return Ok(None) };
    let user = user_of_token(state, token).await?;
    user.map(Some).ok_or_else(|| AppError::unauthorized("This session has ended. Sign in again."))
}

async fn user(state: &AppState, headers: &HeaderMap) -> AppResult<Uuid> {
    signed_in(state, headers).await?.ok_or_else(|| AppError::unauthorized("Sign in first."))
}

async fn new_session(state: &AppState, user_id: Uuid) -> AppResult<Json<TokenResponse>> {
    let token = random_token();
    db::create_session(&state.db, user_id, &sha256_hex(token.as_bytes())).await?;
    Ok(Json(TokenResponse { token }))
}

pub async fn exchange(
    State(state): State<AppState>,
    Json(request): Json<ExchangeRequest>,
) -> AppResult<Json<TokenResponse>> {
    let expired = || AppError::bad_request("This sign-in has expired. Sign in again.");
    let sign_in = db::take_sign_in(&state.db, &sha256_hex(request.code.as_bytes())).await?.ok_or_else(expired)?;
    if sha256_hex(request.verifier.as_bytes()) != sign_in.challenge {
        return Err(AppError::unauthorized("This sign-in was started by another app."));
    }
    let user_id = sign_in.user_id.ok_or_else(expired)?;
    new_session(&state, user_id).await
}

pub async fn link_ticket(State(state): State<AppState>, headers: HeaderMap) -> AppResult<Json<LinkTicket>> {
    let user_id = user(&state, &headers).await?;
    let ticket = random_token();
    db::create_link_ticket(&state.db, user_id, &sha256_hex(ticket.as_bytes())).await?;
    Ok(Json(LinkTicket { ticket }))
}

/// Adds a JMAP account once its server accepts the login. Signed in, it joins the user;
/// otherwise it signs in its user, or makes one.
pub async fn add_jmap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<JmapAccountRequest>,
) -> AppResult<Json<TokenResponse>> {
    let current = signed_in(&state, &headers).await?;
    let url = request.url.trim().trim_end_matches('/').to_string();
    let username = request.username.trim().to_string();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(AppError::bad_request("The server's address should start with https://"));
    }
    let jmap = match Jmap::open(&state.http, &url, &username, &request.password).await {
        Ok(jmap) => jmap,
        Err(error) if error.is::<Reauth>() => return Err(AppError::unauthorized("The email or password is wrong.")),
        Err(error) if error.is::<reqwest::Error>() => {
            return Err(AppError::bad_request(format!("{url} can't be reached. Check the address.")));
        }
        Err(error) => return Err(AppError::bad_request(format!("{error}"))),
    };
    let address = jmap.address().await.map_err(|error| AppError::bad_request(error.to_string()))?.email;
    let login = serde_json::to_string(&Login { url, username })?;
    let existing = db::account_by_login(&state.db, Provider::Jmap, &address, &login).await?;
    let user_id = match (current, existing) {
        (Some(user_id), _) => user_id,
        (None, Some(account)) => account.user_id,
        (None, None) => db::create_user(&state.db).await?,
    };
    let credentials = state.sealer.seal(request.password.as_bytes());
    let account = NewAccount { user_id, provider: Provider::Jmap, address: &address, login: &login, credentials };
    let account_id = db::upsert_account(&state.db, account).await?;
    state.workers.start(&state, account_id);
    match (current, bearer(&headers)) {
        (Some(_), Some(token)) => Ok(Json(TokenResponse { token: token.to_string() })),
        _ => new_session(&state, user_id).await,
    }
}

pub async fn dev_login(
    State(state): State<AppState>,
    Json(request): Json<DevLoginRequest>,
) -> AppResult<Json<TokenResponse>> {
    if !state.config.dev_login {
        return Err(AppError::not_found());
    }
    if request.email.trim().is_empty() {
        return Err(AppError::bad_request("An email address is needed."));
    }
    let user_id = db::create_user(&state.db).await?;
    new_session(&state, user_id).await
}

/// One of a message's attachments, by its place in the message's list, fetched from the provider.
pub async fn attachment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, index)): Path<(Uuid, usize)>,
) -> AppResult<Response> {
    let user_id = user(&state, &headers).await?;
    let message = db::message(&state.db, user_id, id).await?.filter(|message| !message.deleted);
    let Some(message) = message else {
        return Err(AppError::not_found());
    };
    let raw = workers::raw_message(&state, &message).await.map_err(|error| {
        tracing::warn!("couldn't fetch a message for its attachment: {error:#}");
        AppError::new(StatusCode::BAD_GATEWAY, "The attachment couldn't be fetched. Try again.")
    })?;
    let Some(file) = mime::files(&raw).into_iter().nth(index) else {
        return Err(AppError::not_found());
    };
    let disposition = content_disposition(&file.name);
    Ok(([(CONTENT_TYPE, file.mime), (CONTENT_DISPOSITION, disposition)], file.bytes).into_response())
}

/// `attachment` with the name as ASCII and, for any other characters, as RFC 5987 UTF-8.
fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|character| match character {
            ' '..='~' if character != '"' && character != '\\' => character,
            _ => '_',
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", utf8_percent_encode(name, NON_ALPHANUMERIC))
}

/// A file for a draft to send. The body is the file; `x-file-name` its name, percent-encoded.
pub async fn upload(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> AppResult<Json<UploadResponse>> {
    let user_id = user(&state, &headers).await?;
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or_default();
    let name = percent_decode_str(header("x-file-name")).decode_utf8_lossy().trim().to_string();
    let name = name.rsplit(['/', '\\']).next().unwrap_or_default().chars().take(255).collect::<String>();
    let name = if name.is_empty() { "Attachment".to_string() } else { name };
    let mime = header("content-type").split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let mime = match mime.split_once('/') {
        Some((kind, subtype)) if !kind.is_empty() && !subtype.is_empty() => mime,
        _ => "application/octet-stream".to_string(),
    };
    let id = db::create_upload(&state.db, user_id, &name, &mime, &body).await?;
    Ok(Json(UploadResponse { id: id.to_string() }))
}

/// Gmail's Pub/Sub push: `{message: {data: base64({emailAddress, historyId})}}`.
pub async fn gmail_hook(
    State(state): State<AppState>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    Json(body): Json<Value>,
) -> StatusCode {
    let Some(expected) = &state.config.gmail_hook_token else { return StatusCode::NOT_FOUND };
    let given = query.get("token").map(String::as_str).unwrap_or_default();
    if sha256_hex(given.as_bytes()) != sha256_hex(expected.as_bytes()) {
        return StatusCode::FORBIDDEN;
    }
    use base64::Engine;
    let data = body["message"]["data"].as_str().unwrap_or_default();
    let decoded = base64::engine::general_purpose::STANDARD.decode(data).unwrap_or_default();
    let notice: Value = serde_json::from_slice(&decoded).unwrap_or(json!({}));
    let Some(address) = notice["emailAddress"].as_str() else { return StatusCode::NO_CONTENT };
    if let Ok(accounts) = db::gmail_accounts_by_address(&state.db, &address.to_lowercase()).await {
        for account_id in accounts {
            crate::hub::notify_ops(&state.db, account_id).await;
        }
    }
    StatusCode::NO_CONTENT
}
