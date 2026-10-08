//! The browser's side of adding a Gmail account. The app opens `/auth/google/start`, Google
//! asks for consent, and the browser is sent back to `mailapp://auth` with a one-time code that
//! only the app holding the secret behind the challenge can use. A link ticket in the start URL
//! adds the account to the user who asked for the ticket.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use mail_protocol::{APP_REDIRECT, Provider};
use reqwest::Url;
use serde_json::json;

use crate::db::{NewAccount, NewSignIn};
use crate::{AppState, db, google, random_token, sha256_hex};

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.chars().all(|character| character.is_ascii_hexdigit())
}

fn back(parameters: &[(&str, &str)]) -> Response {
    let Ok(url) = Url::parse_with_params(APP_REDIRECT, parameters) else {
        return failed("This sign-in can't be finished. Start again.");
    };
    Redirect::to(url.as_str()).into_response()
}

fn failed(message: &str) -> Response {
    let page = format!(
        "<!doctype html><meta name=viewport content=\"width=device-width\"><title>Sign-in failed</title>\
         <style>{}body{{font:15px \"Avenir Next\",-apple-system,system-ui,sans-serif;background:var(--background);color:var(--foreground);\
         display:grid;place-items:center;height:90vh;margin:0}}</style><p>{}</p>",
        include_str!("../../../packages/theme/tokens.css"),
        message
    );
    (StatusCode::BAD_REQUEST, Html(page)).into_response()
}

pub async fn start(State(state): State<AppState>, Query(query): Query<HashMap<String, String>>) -> Response {
    let text = |key: &str| query.get(key).map(String::as_str).unwrap_or_default();
    let (challenge, app_state, ticket) = (text("challenge"), text("state"), text("ticket"));
    if !state.config.google_enabled() {
        return failed("This server isn't set up for Gmail.");
    }
    if !is_sha256_hex(challenge) || app_state.is_empty() || app_state.len() > 200 {
        return failed("This sign-in link is malformed. Start again.");
    }
    let link_user_id = match ticket.is_empty() {
        true => None,
        false => match db::take_link_ticket(&state.db, &sha256_hex(ticket.as_bytes())).await {
            Ok(Some(user_id)) => Some(user_id),
            Ok(None) => return failed("This link has expired. Start again from the app."),
            Err(error) => {
                tracing::error!("couldn't read a link ticket: {error}");
                return failed("Something went wrong. Try again in a moment.");
            }
        },
    };

    let (id, google_verifier, nonce) = (random_token(), random_token(), random_token());
    let sign_in =
        NewSignIn { id: &id, challenge, app_state, google_verifier: &google_verifier, nonce: &nonce, link_user_id };
    if let Err(error) = db::create_sign_in(&state.db, &sign_in).await {
        tracing::error!("couldn't save a sign-in: {error}");
        return failed("Something went wrong. Try again in a moment.");
    }
    Redirect::to(&google::authorize_url(&state.config, &id, &nonce, &google_verifier)).into_response()
}

pub async fn callback(State(state): State<AppState>, Query(query): Query<HashMap<String, String>>) -> Response {
    let text = |key: &str| query.get(key).map(String::as_str).unwrap_or_default();
    let sign_in = match db::pending_sign_in(&state.db, text("state")).await {
        Ok(Some(sign_in)) => sign_in,
        Ok(None) => return failed("This sign-in has expired. Start again."),
        Err(error) => {
            tracing::error!("couldn't load a sign-in: {error}");
            return failed("Something went wrong. Try again in a moment.");
        }
    };
    if !text("error").is_empty() || text("code").is_empty() {
        return back(&[("error", "access_denied"), ("state", &sign_in.app_state)]);
    }

    let grant = match google::exchange(
        &state.http,
        &state.config,
        text("code"),
        &sign_in.google_verifier,
        &sign_in.nonce,
    )
    .await
    {
        Ok(grant) => grant,
        Err(error) => {
            tracing::warn!("Google sign-in failed: {error}");
            return failed("Google didn't give access to the mail. Start again and allow it.");
        }
    };

    let code = random_token();
    let saved = async {
        // A second account joins the user who asked; a known one signs its user in; else a new user.
        let existing = db::account_by_login(&state.db, Provider::Gmail, &grant.email, "").await?;
        let user_id = match (sign_in.link_user_id, existing) {
            (Some(user_id), _) => user_id,
            (None, Some(account)) => account.user_id,
            (None, None) => db::create_user(&state.db).await?,
        };
        let credentials = state.sealer.seal(json!({ "refresh_token": grant.refresh_token }).to_string().as_bytes());
        let account = NewAccount { user_id, provider: Provider::Gmail, address: &grant.email, login: "", credentials };
        let account_id = db::upsert_account(&state.db, account).await?;
        state.workers.start(&state, account_id);
        db::complete_sign_in(&state.db, &sign_in.id, user_id, &sha256_hex(code.as_bytes())).await
    };
    if let Err(error) = saved.await {
        tracing::error!("couldn't complete a sign-in: {error}");
        return failed("Something went wrong. Try again in a moment.");
    }
    back(&[("code", &code), ("state", &sign_in.app_state)])
}
