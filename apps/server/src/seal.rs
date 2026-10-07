//! Provider credentials at rest: AES-256-GCM with a key derived from SECRET_KEY, a random nonce
//! in front of the ciphertext.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use sha2::{Digest, Sha256};

const NONCE_LEN: usize = 12;

pub struct Sealer(Aes256Gcm);

impl Sealer {
    pub fn new(secret: &str) -> Self {
        let key: [u8; 32] = Sha256::digest(secret.as_bytes()).into();
        Self(Aes256Gcm::new(&Key::<Aes256Gcm>::from(key)))
    }

    pub fn seal(&self, plain: &[u8]) -> Vec<u8> {
        let nonce: [u8; NONCE_LEN] = crate::random_bytes();
        let sealed = self.0.encrypt(&Nonce::from(nonce), plain).expect("AES-GCM encrypts any length we use");
        [nonce.as_slice(), &sealed].concat()
    }

    pub fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < NONCE_LEN {
            return None;
        }
        let (nonce, body) = sealed.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
        self.0.decrypt(&Nonce::from(nonce), body).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_what_it_sealed_and_nothing_else() {
        let sealer = Sealer::new("secret");
        let sealed = sealer.seal(b"refresh-token");
        assert_eq!(sealer.open(&sealed).as_deref(), Some(b"refresh-token".as_slice()));
        assert_eq!(Sealer::new("other").open(&sealed), None);
        let mut tampered = sealed.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert_eq!(sealer.open(&tampered), None);
    }
}
