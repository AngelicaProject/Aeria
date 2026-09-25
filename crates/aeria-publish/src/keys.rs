//! Publisher signing keys in the OS credential store.
//!
//! The private key of a pack is stored per `packId` and never written to the
//! project, settings, logs, errors, or IPC responses. A backup file is written
//! only on the user's explicit request.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use aeria_export::PackSigner;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Service name under which Aeria stores secrets.
pub const KEYRING_SERVICE: &str = "Aeria";
const BACKUP_FORMAT: &str = "aeria-signing-key";
const BACKUP_VERSION: u64 = 1;

/// A P-256 private scalar. Its `Debug` output is redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct SigningSecret([u8; 32]);

impl SigningSecret {
    /// Generates a new key from the operating system's random source.
    ///
    /// # Errors
    /// Returns [`KeyError::Random`] when no randomness is available.
    pub fn generate() -> Result<Self, KeyError> {
        loop {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|error| KeyError::Random {
                message: error.to_string(),
            })?;
            // A scalar outside the curve order is rejected; the chance is
            // about 2^-32, so draw again.
            if PackSigner::from_secret_bytes(&bytes).is_ok() {
                return Ok(Self(bytes));
            }
        }
    }

    fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (index, pair) in text.as_bytes().chunks(2).enumerate() {
            let digits = std::str::from_utf8(pair).ok()?;
            if digits.bytes().any(|b| b.is_ascii_uppercase()) {
                return None;
            }
            bytes[index] = u8::from_str_radix(digits, 16).ok()?;
        }
        PackSigner::from_secret_bytes(&bytes).ok()?;
        Some(Self(bytes))
    }

    fn to_hex(&self) -> String {
        self.0.iter().fold(String::with_capacity(64), |mut out, b| {
            use fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        })
    }

    /// The signer for this key.
    ///
    /// # Panics
    /// Never: every constructor checks the scalar.
    #[must_use]
    pub fn signer(&self) -> PackSigner {
        PackSigner::from_secret_bytes(&self.0).expect("checked when created")
    }

    /// Fingerprint of the public key, as recorded in pack settings and feeds.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        self.signer().fingerprint()
    }

    /// The backup file for this key: JSON with the key in hex.
    #[must_use]
    pub fn to_backup(&self, pack_id: &str) -> String {
        let backup = BackupJson {
            format: BACKUP_FORMAT.to_owned(),
            version: BACKUP_VERSION,
            pack_id: pack_id.to_owned(),
            fingerprint: self.fingerprint(),
            secret_key: self.to_hex(),
        };
        let mut text = serde_json::to_string_pretty(&backup).unwrap_or_default();
        text.push('\n');
        text
    }

    /// Reads a backup written by [`Self::to_backup`]. The fingerprint in the
    /// file must match the key.
    ///
    /// # Errors
    /// Returns [`KeyError::InvalidBackup`] for anything else. The message
    /// never contains key material.
    pub fn from_backup(text: &str) -> Result<Self, KeyError> {
        let invalid = |message: &str| KeyError::InvalidBackup {
            message: message.to_owned(),
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let backup: BackupJson =
            serde_json::from_str(text).map_err(|_| invalid("the file is not a key backup"))?;
        if backup.format != BACKUP_FORMAT || backup.version != BACKUP_VERSION {
            return Err(invalid("the file is not a version 1 Aeria key backup"));
        }
        let secret =
            Self::from_hex(&backup.secret_key).ok_or_else(|| invalid("the key is not valid"))?;
        if secret.fingerprint() != backup.fingerprint {
            return Err(invalid("the fingerprint does not match the key"));
        }
        Ok(secret)
    }
}

impl fmt::Debug for SigningSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SigningSecret(<redacted>)")
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackupJson {
    format: String,
    version: u64,
    pack_id: String,
    fingerprint: String,
    secret_key: String,
}

/// Errors from key storage. They never contain key material.
#[derive(Debug, Error)]
pub enum KeyError {
    #[error("the OS secret store is unavailable: {message}")]
    Unavailable { message: String },
    #[error("the OS secret store failed: {message}")]
    Failed { message: String },
    #[error("invalid key backup: {message}")]
    InvalidBackup { message: String },
    #[error("no random source is available: {message}")]
    Random { message: String },
}

/// Storage for pack signing keys, one per `packId`.
pub trait SigningKeyStore: Send + Sync {
    /// # Errors
    /// Returns an error when the store cannot be read.
    fn get(&self, pack_id: &str) -> Result<Option<SigningSecret>, KeyError>;
    /// Stores or replaces the key.
    ///
    /// # Errors
    /// Returns an error when the store cannot be written.
    fn set(&self, pack_id: &str, secret: &SigningSecret) -> Result<(), KeyError>;
    /// Removes the key. Removing a missing key succeeds.
    ///
    /// # Errors
    /// Returns an error when the store cannot be written.
    fn delete(&self, pack_id: &str) -> Result<(), KeyError>;
}

/// The platform credential store, under the service [`KEYRING_SERVICE`] and
/// the account `pack-signing/<packId>`.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyringSigningKeyStore;

impl KeyringSigningKeyStore {
    fn entry(pack_id: &str) -> Result<keyring::Entry, KeyError> {
        keyring::Entry::new(KEYRING_SERVICE, &format!("pack-signing/{pack_id}"))
            .map_err(|error| keyring_error(&error))
    }
}

impl SigningKeyStore for KeyringSigningKeyStore {
    fn get(&self, pack_id: &str) -> Result<Option<SigningSecret>, KeyError> {
        match Self::entry(pack_id)?.get_password() {
            Ok(value) => {
                SigningSecret::from_hex(&value)
                    .map(Some)
                    .ok_or_else(|| KeyError::Failed {
                        message: "the stored signing key is not valid".to_owned(),
                    })
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(&error)),
        }
    }

    fn set(&self, pack_id: &str, secret: &SigningSecret) -> Result<(), KeyError> {
        Self::entry(pack_id)?
            .set_password(&secret.to_hex())
            .map_err(|error| keyring_error(&error))
    }

    fn delete(&self, pack_id: &str) -> Result<(), KeyError> {
        match Self::entry(pack_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(&error)),
        }
    }
}

fn keyring_error(error: &keyring::Error) -> KeyError {
    match error {
        keyring::Error::NoStorageAccess(_)
        | keyring::Error::PlatformFailure(_)
        | keyring::Error::NoDefaultStore => KeyError::Unavailable {
            message: error.to_string(),
        },
        // These variants carry the stored bytes; never format them.
        keyring::Error::BadEncoding(_) | keyring::Error::BadDataFormat(..) => KeyError::Failed {
            message: "the stored signing key cannot be read".to_owned(),
        },
        _ => KeyError::Failed {
            message: error.to_string(),
        },
    }
}

/// An in-process store for tests.
#[derive(Debug, Default)]
pub struct MemorySigningKeyStore {
    keys: Mutex<HashMap<String, SigningSecret>>,
}

impl MemorySigningKeyStore {
    fn keys(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, SigningSecret>>, KeyError> {
        self.keys.lock().map_err(|_| KeyError::Failed {
            message: "memory key store lock is poisoned".to_owned(),
        })
    }
}

impl SigningKeyStore for MemorySigningKeyStore {
    fn get(&self, pack_id: &str) -> Result<Option<SigningSecret>, KeyError> {
        Ok(self.keys()?.get(pack_id).cloned())
    }

    fn set(&self, pack_id: &str, secret: &SigningSecret) -> Result<(), KeyError> {
        self.keys()?.insert(pack_id.to_owned(), secret.clone());
        Ok(())
    }

    fn delete(&self, pack_id: &str) -> Result<(), KeyError> {
        self.keys()?.remove(pack_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_differ_and_round_trip_through_backups() {
        let first = SigningSecret::generate().expect("key");
        let second = SigningSecret::generate().expect("key");
        assert_ne!(first, second);
        let backup = first.to_backup("ru-main");
        assert!(backup.contains(&first.fingerprint()));
        assert_eq!(SigningSecret::from_backup(&backup).expect("restore"), first);
    }

    #[test]
    fn damaged_backups_are_rejected_without_echoing_the_key() {
        let key = SigningSecret::generate().expect("key");
        let backup = key.to_backup("ru-main");
        let hex = key.to_hex();
        let wrong_fingerprint = backup.replace(&key.fingerprint(), &"0".repeat(64));
        let wrong_key = backup.replace(&hex, &"f".repeat(64));
        for text in [
            wrong_fingerprint,
            wrong_key,
            backup.replace("\"version\": 1", "\"version\": 2"),
            backup.replace(&hex, &hex.to_uppercase()),
            "{}".to_owned(),
        ] {
            let error = SigningSecret::from_backup(&text).expect_err("rejected");
            assert!(!error.to_string().contains(&hex));
        }
        assert!(!format!("{key:?}").contains(&hex));
    }

    #[test]
    fn memory_store_keeps_one_key_per_pack() {
        let store = MemorySigningKeyStore::default();
        let key = SigningSecret::generate().expect("key");
        assert_eq!(store.get("ru-main").expect("get"), None);
        store.set("ru-main", &key).expect("set");
        assert_eq!(store.get("ru-main").expect("get"), Some(key));
        assert_eq!(store.get("fr-main").expect("get"), None);
        store.delete("ru-main").expect("delete");
        store.delete("ru-main").expect("delete missing");
        assert_eq!(store.get("ru-main").expect("get"), None);
    }
}
