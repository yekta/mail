//! The HTTP client and the WebSocket are built without a TLS provider of their own, so the server
//! and the clients carry one crypto library: ring.

/// Makes ring the process's TLS provider. Safe to call more than once.
pub fn install() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
