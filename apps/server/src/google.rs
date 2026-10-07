//! Google's OAuth with PKCE. The server is the OAuth client: it keeps the refresh token, sealed,
//! and syncs Gmail with it.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::Config;

pub const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const SCOPES: &str = "openid email https://www.googleapis.com/auth/gmail.modify";
const GMAIL_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";
const ISSUERS: [&str; 2] = ["https://accounts.google.com", "accounts.google.com"];

pub struct Grant {
    pub email: String,
    pub refresh_token: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

fn redirect_uri(config: &Config) -> String {
    format!("{}/auth/google/callback", config.public_url)
}

pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn authorize_url(config: &Config, state: &str, nonce: &str, verifier: &str) -> String {
    let mut url = Url::parse(&config.google_authorize_url).expect("a valid authorize URL");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.google_client_id)
        .append_pair("redirect_uri", &redirect_uri(config))
        .append_pair("scope", SCOPES)
        .append_pair("state", state)
        .append_pair("nonce", nonce)
        .append_pair("code_challenge", &pkce_challenge(verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    url.into()
}

pub async fn exchange(
    http: &reqwest::Client,
    config: &Config,
    code: &str,
    verifier: &str,
    nonce: &str,
) -> Result<Grant, String> {
    let redirect_uri = redirect_uri(config);
    let response = http
        .post(&config.google_token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri),
            ("client_id", &config.google_client_id),
            ("client_secret", &config.google_client_secret),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|err| format!("token request failed: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("token endpoint answered {status}: {body}"));
    }
    let tokens: TokenResponse = response.json().await.map_err(|err| format!("bad token response: {err}"))?;
    if !tokens.scope.split(' ').any(|scope| scope == GMAIL_SCOPE) {
        return Err("Gmail access wasn't granted".into());
    }
    let Some(refresh_token) = tokens.refresh_token else {
        return Err("Google sent no refresh token".into());
    };
    let email = email_from_id_token(&tokens.id_token, &config.google_client_id, nonce)?;
    Ok(Grant { email, refresh_token })
}

/// The ID token comes straight from Google's token endpoint over TLS, so per OIDC Core 3.1.3.7
/// its signature doesn't need checking; the claims still do.
fn email_from_id_token(id_token: &str, client_id: &str, nonce: &str) -> Result<String, String> {
    let payload = id_token.split('.').nth(1).ok_or("malformed ID token")?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|_| "malformed ID token")?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|_| "malformed ID token")?;

    let issuer = claims["iss"].as_str().unwrap_or_default();
    if !ISSUERS.contains(&issuer) {
        return Err(format!("unexpected issuer {issuer}"));
    }
    let audience_ok = match &claims["aud"] {
        Value::String(aud) => aud == client_id,
        Value::Array(auds) => auds.iter().any(|aud| aud == client_id),
        _ => false,
    };
    if !audience_ok {
        return Err("ID token is for another client".into());
    }
    if claims["exp"].as_i64().unwrap_or(0) < chrono::Utc::now().timestamp() {
        return Err("ID token expired".into());
    }
    if claims["nonce"].as_str() != Some(nonce) {
        return Err("nonce mismatch".into());
    }
    if claims["email_verified"] != true && claims["email_verified"] != "true" {
        return Err("Google hasn't verified this address".into());
    }
    let email = claims["email"].as_str().filter(|email| !email.is_empty()).ok_or("ID token has no email")?;
    Ok(email.to_lowercase())
}

#[cfg(test)]
pub fn id_token(claims: &Value) -> String {
    format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn claims() -> Value {
        json!({
            "iss": "https://accounts.google.com",
            "aud": "client",
            "exp": chrono::Utc::now().timestamp() + 60,
            "nonce": "n",
            "email": "Ann@Example.com",
            "email_verified": true,
        })
    }

    #[test]
    fn reads_the_address_from_a_valid_token() {
        assert_eq!(email_from_id_token(&id_token(&claims()), "client", "n").unwrap(), "ann@example.com");
    }

    #[test]
    fn rejects_wrong_claims() {
        let with = |key: &str, value: Value| {
            let mut claims = claims();
            claims[key] = value;
            id_token(&claims)
        };
        assert!(email_from_id_token(&with("iss", json!("https://evil.com")), "client", "n").is_err());
        assert!(email_from_id_token(&with("aud", json!("other")), "client", "n").is_err());
        assert!(email_from_id_token(&with("exp", json!(1)), "client", "n").is_err());
        assert!(email_from_id_token(&with("nonce", json!("x")), "client", "n").is_err());
        assert!(email_from_id_token(&with("email_verified", json!(false)), "client", "n").is_err());
    }
}
