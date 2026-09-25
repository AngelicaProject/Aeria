//! Provider API keys in the OS secret store.
//!
//! Keys are stored per provider ID and never written to settings, logs,
//! errors, or IPC responses.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use thiserror::Error;

/// Service name under which Aeria stores provider keys.
pub const KEYRING_SERVICE: &str = "Aeria";

/// A provider API key. Its `Debug` output is redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key after trimming surrounding whitespace.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty key or one containing whitespace or
    /// control characters, which cannot be sent as a bearer token.
    pub fn new(value: &str) -> Result<Self, SecretStoreError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(SecretStoreError::InvalidKey {
                message: "API key must not be empty".to_owned(),
            });
        }
        if trimmed
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(SecretStoreError::InvalidKey {
                message: "API key must not contain spaces or control characters".to_owned(),
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the key text for sending it to its provider.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey(<redacted>)")
    }
}

/// Errors from the secret store. They never contain key material.
#[derive(Debug, Error)]
pub enum SecretStoreError {
    #[error("{message}")]
    InvalidKey { message: String },

    #[error("the OS secret store is unavailable: {message}")]
    Unavailable { message: String },

    #[error("the OS secret store failed: {message}")]
    Failed { message: String },
}

/// Storage for provider API keys.
pub trait SecretStore: Send + Sync {
    /// Returns the provider's key, or `None` when none is stored.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    fn get(&self, provider_id: &str) -> Result<Option<ApiKey>, SecretStoreError>;

    /// Stores or replaces the provider's key.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be written.
    fn set(&self, provider_id: &str, key: &ApiKey) -> Result<(), SecretStoreError>;

    /// Removes the provider's key. Removing a missing key succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be written.
    fn delete(&self, provider_id: &str) -> Result<(), SecretStoreError>;
}

/// The platform credential store: Windows Credential Manager, the Secret
/// Service on Linux, or the macOS Keychain.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyringSecretStore;

impl KeyringSecretStore {
    fn entry(provider_id: &str) -> Result<keyring::Entry, SecretStoreError> {
        keyring::Entry::new(KEYRING_SERVICE, &format!("ai-provider/{provider_id}"))
            .map_err(|error| keyring_error(&error))
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, provider_id: &str) -> Result<Option<ApiKey>, SecretStoreError> {
        match Self::entry(provider_id)?.get_password() {
            Ok(value) => ApiKey::new(&value)
                .map(Some)
                .map_err(|_| SecretStoreError::Failed {
                    message: "the stored API key is not valid".to_owned(),
                }),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(&error)),
        }
    }

    fn set(&self, provider_id: &str, key: &ApiKey) -> Result<(), SecretStoreError> {
        Self::entry(provider_id)?
            .set_password(key.expose())
            .map_err(|error| keyring_error(&error))
    }

    fn delete(&self, provider_id: &str) -> Result<(), SecretStoreError> {
        match Self::entry(provider_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keyring_error(&error)),
        }
    }
}

fn keyring_error(error: &keyring::Error) -> SecretStoreError {
    // keyring errors describe the store and entry, never the secret itself.
    match error {
        keyring::Error::NoStorageAccess(_)
        | keyring::Error::PlatformFailure(_)
        | keyring::Error::NoDefaultStore => SecretStoreError::Unavailable {
            message: error.to_string(),
        },
        _ => SecretStoreError::Failed {
            message: error.to_string(),
        },
    }
}

/// An in-process store for tests and for platforms without a secret store.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    keys: Mutex<HashMap<String, ApiKey>>,
}

impl MemorySecretStore {
    fn keys(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, ApiKey>>, SecretStoreError> {
        self.keys.lock().map_err(|_| SecretStoreError::Failed {
            message: "memory secret store lock is poisoned".to_owned(),
        })
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, provider_id: &str) -> Result<Option<ApiKey>, SecretStoreError> {
        Ok(self.keys()?.get(provider_id).cloned())
    }

    fn set(&self, provider_id: &str, key: &ApiKey) -> Result<(), SecretStoreError> {
        self.keys()?.insert(provider_id.to_owned(), key.clone());
        Ok(())
    }

    fn delete(&self, provider_id: &str) -> Result<(), SecretStoreError> {
        self.keys()?.remove(provider_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_is_trimmed_validated_and_redacted() {
        let key = ApiKey::new("  sk-test-123 \n").expect("valid key");
        assert_eq!(key.expose(), "sk-test-123");
        assert_eq!(format!("{key:?}"), "ApiKey(<redacted>)");
        assert!(ApiKey::new("   ").is_err());
        assert!(ApiKey::new("sk test").is_err());
    }

    #[test]
    fn memory_store_sets_replaces_and_deletes() {
        let store = MemorySecretStore::default();
        assert_eq!(store.get("p1").expect("get"), None);
        store
            .set("p1", &ApiKey::new("a").expect("key"))
            .expect("set");
        store
            .set("p1", &ApiKey::new("b").expect("key"))
            .expect("replace");
        assert_eq!(
            store.get("p1").expect("get"),
            Some(ApiKey::new("b").expect("key"))
        );
        store.delete("p1").expect("delete");
        store.delete("p1").expect("delete missing");
        assert_eq!(store.get("p1").expect("get"), None);
    }
}
