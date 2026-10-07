//! The server's HTTP API: signing in, adding accounts, and the files of sends and attachments.

use std::time::Duration;

use anyhow::{Result, bail};
use mail_protocol::api::UploadResponse;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// How long a file may take to go up or come down.
const FILE_TIMEOUT: Duration = Duration::from_secs(600);

fn url(server: &str, path: &str) -> String {
    format!("{}{path}", server.trim_end_matches('/'))
}

/// The response, when the server answered with success; otherwise its error as the user reads it.
async fn answered(request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) if error.is_connect() || error.is_timeout() => bail!("The server can't be reached."),
        Err(error) => return Err(error.into()),
    };
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|body| body["error"].as_str().map(String::from))
        .unwrap_or_else(|| format!("The server answered {status}."));
    bail!(message)
}

pub async fn post<T: DeserializeOwned>(
    http: &reqwest::Client,
    server: &str,
    path: &str,
    token: Option<&str>,
    body: &impl Serialize,
) -> Result<T> {
    let mut request = http.post(url(server, path)).json(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    Ok(answered(request).await?.json().await?)
}

/// Sends a file for a draft. Returns the id the server keeps it under.
pub async fn upload(
    http: &reqwest::Client,
    server: &str,
    token: &str,
    name: &str,
    mime: &str,
    bytes: Vec<u8>,
) -> Result<String> {
    let mime = if mime.is_empty() { "application/octet-stream" } else { mime };
    let request = http
        .post(url(server, "/api/uploads"))
        .bearer_auth(token)
        .header("x-file-name", encode(name))
        .header(reqwest::header::CONTENT_TYPE, mime)
        .timeout(FILE_TIMEOUT)
        .body(bytes);
    Ok(answered(request).await?.json::<UploadResponse>().await?.id)
}

pub async fn download(http: &reqwest::Client, server: &str, path: &str, token: &str) -> Result<Vec<u8>> {
    let request = http.get(url(server, path)).bearer_auth(token).timeout(FILE_TIMEOUT);
    Ok(answered(request).await?.bytes().await?.to_vec())
}

/// Percent-encodes all but the characters a URL never needs encoded.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn encodes_names_for_headers_and_paths() {
        assert_eq!(super::encode("Q4 plan (final).pdf"), "Q4%20plan%20%28final%29.pdf");
        assert_eq!(super::encode("été/x"), "%C3%A9t%C3%A9%2Fx");
    }
}
