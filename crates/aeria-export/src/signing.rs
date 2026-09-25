use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::error::ExportError;

pub(crate) const SIGNATURE_DOMAIN: &[u8] = b"AERIA-HPK-V1-SIGNATURE";
pub(crate) const ROTATION_DOMAIN: &[u8] = b"AERIA-HPK-V1-KEY-ROTATION";

/// A publisher signing key (ECDSA P-256 with SHA-256). Signatures are
/// deterministic (RFC 6979), so signed packs stay reproducible.
pub struct PackSigner {
    key: SigningKey,
}

/// A previous key's statement that `next` is its successor, carried in every
/// pack signed by the new key so Harmonia can move its pin without asking.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyEndorsement {
    pub previous_public_key: [u8; 65],
    pub signature: [u8; 64],
}

impl PackSigner {
    /// # Errors
    /// Returns [`ExportError::SigningKey`] for a scalar outside the curve order.
    pub fn from_secret_bytes(secret: &[u8; 32]) -> Result<Self, ExportError> {
        SigningKey::from_slice(secret)
            .map(|key| Self { key })
            .map_err(|_| ExportError::SigningKey)
    }

    #[must_use]
    pub fn public_key(&self) -> [u8; 65] {
        let point = self.key.verifying_key().to_sec1_point(false);
        let mut bytes = [0u8; 65];
        bytes.copy_from_slice(point.as_bytes());
        bytes
    }

    #[must_use]
    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public_key())
    }

    /// Endorses `next_public_key` as this key's successor.
    #[must_use]
    pub fn endorse(&self, next_public_key: &[u8; 65]) -> KeyEndorsement {
        KeyEndorsement {
            previous_public_key: self.public_key(),
            signature: self.sign(ROTATION_DOMAIN, next_public_key),
        }
    }

    pub(crate) fn sign(&self, domain: &[u8], payload: &[u8]) -> [u8; 64] {
        let message = [domain, payload].concat();
        let signature: Signature = self.key.sign(&message);
        signature.to_bytes().into()
    }
}

/// SHA-256 of the 65-byte public key, in lowercase hex.
#[must_use]
pub fn fingerprint(public_key: &[u8; 65]) -> String {
    crate::hex(&Sha256::digest(public_key))
}

impl KeyEndorsement {
    pub(crate) fn is_valid_for(&self, next_public_key: &[u8; 65]) -> bool {
        let Ok(key) = VerifyingKey::from_sec1_bytes(&self.previous_public_key) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(&self.signature) else {
            return false;
        };
        key.verify(
            &[ROTATION_DOMAIN, next_public_key.as_slice()].concat(),
            &signature,
        )
        .is_ok()
    }
}
