//! The server's HTTP API: signing in and adding accounts.

use anyhow::{Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub async fn post<T: DeserializeOwned>(
    http: &reqwest::Client,
    server: &str,
    path: &str,
    token: Option<&str>,
    body: &impl Serialize,
) -> Result<T> {
    let mut request = http.post(format!("{}{path}", server.trim_end_matches('/'))).json(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) if error.is_connect() || error.is_timeout() => bail!("The server can't be reached."),
        Err(error) => return Err(error.into()),
    };
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|body| body["error"].as_str().map(String::from))
            .unwrap_or_else(|| format!("The server answered {status}."));
        bail!(message);
    }
    Ok(response.json().await?)
}
