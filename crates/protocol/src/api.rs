//! The JSON of the server's HTTP API.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeRequest {
    pub code: String,
    pub verifier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkTicket {
    pub ticket: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JmapAccountRequest {
    pub url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevLoginRequest {
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// The answer to `POST /api/uploads`, whose body is the file. Its name comes in `x-file-name`
/// (percent-encoded) and its type in `content-type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadResponse {
    pub id: String,
}
