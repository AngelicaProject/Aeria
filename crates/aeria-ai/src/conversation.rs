//! Local, per-project Angelica conversations.
//!
//! Conversations are machine-local application data and are never written
//! to a project repository.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chat::{ChatMessage, Usage};
use crate::settings::ModelSelection;

pub const FORMAT_VERSION: u32 = 1;
/// Largest conversation file accepted.
pub const MAX_CONVERSATION_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Most conversations listed per project.
pub const MAX_LISTED_CONVERSATIONS: usize = 200;
const TITLE_CHARS: usize = 60;

/// One stored conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conversation {
    pub format_version: u32,
    pub id: String,
    pub title: String,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    /// The model and effort last used, reused for the next message.
    pub model: Option<ModelSelection>,
    pub messages: Vec<ChatMessage>,
    pub usage: Usage,
}

impl Conversation {
    /// Creates an empty conversation with a new ID.
    #[must_use]
    pub fn new(now_unix_ms: u64) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            title: String::new(),
            created_at_unix_ms: now_unix_ms,
            updated_at_unix_ms: now_unix_ms,
            model: None,
            messages: Vec::new(),
            usage: Usage::default(),
        }
    }

    /// Appends a user message, titling a new conversation after it.
    pub fn push_user(&mut self, text: &str, now_unix_ms: u64) {
        if self.title.is_empty() {
            let line = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .trim();
            let mut title: String = line.chars().take(TITLE_CHARS).collect();
            if line.chars().count() > TITLE_CHARS {
                title.push('…');
            }
            self.title = title;
        }
        self.messages.push(ChatMessage::User {
            content: text.to_owned(),
        });
        self.updated_at_unix_ms = now_unix_ms;
    }
}

/// A conversation in the history list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_at_unix_ms: u64,
}

/// Errors from conversation storage.
#[derive(Debug, Error)]
pub enum ConversationError {
    #[error("conversation filesystem operation '{operation}' failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },

    #[error("invalid conversation file {path}: {message}")]
    Invalid { path: PathBuf, message: String },

    #[error("conversation {id:?} was not found")]
    NotFound { id: String },
}

/// Conversation files of one project: `<directory>/<id>.json`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConversationStore {
    directory: PathBuf,
}

impl ConversationStore {
    #[must_use]
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    fn path(&self, id: &str) -> Result<PathBuf, ConversationError> {
        if uuid::Uuid::parse_str(id).is_err() || id.len() != 36 {
            return Err(ConversationError::NotFound { id: id.to_owned() });
        }
        Ok(self.directory.join(format!("{id}.json")))
    }

    /// Lists conversations, newest first. Unreadable files are skipped so
    /// one damaged conversation does not hide the others.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory exists but cannot be read.
    pub fn list(&self) -> Result<Vec<ConversationSummary>, ConversationError> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(io_error("list conversations", &self.directory, source)),
        };
        let mut summaries: Vec<ConversationSummary> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                let id = name.strip_suffix(".json")?;
                let conversation = self.load(id).ok()?;
                Some(ConversationSummary {
                    id: conversation.id,
                    title: conversation.title,
                    updated_at_unix_ms: conversation.updated_at_unix_ms,
                })
            })
            .collect();
        summaries.sort_by(|left, right| {
            right
                .updated_at_unix_ms
                .cmp(&left.updated_at_unix_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        summaries.truncate(MAX_LISTED_CONVERSATIONS);
        Ok(summaries)
    }

    /// Loads one conversation.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` for an unknown or malformed ID, or a typed error
    /// for an unreadable or invalid file.
    pub fn load(&self, id: &str) -> Result<Conversation, ConversationError> {
        let path = self.path(id)?;
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ConversationError::NotFound { id: id.to_owned() });
            }
            Err(source) => return Err(io_error("open conversation", &path, source)),
        };
        let mut bytes = Vec::new();
        file.take(MAX_CONVERSATION_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| io_error("read conversation", &path, source))?;
        if bytes.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
            return Err(invalid(&path, "the file exceeds the size limit"));
        }
        let conversation: Conversation =
            serde_json::from_slice(&bytes).map_err(|error| invalid(&path, &error.to_string()))?;
        if conversation.format_version != FORMAT_VERSION {
            return Err(invalid(
                &path,
                &format!("unsupported format version {}", conversation.format_version),
            ));
        }
        if conversation.id != id {
            return Err(invalid(&path, "the stored ID does not match the file name"));
        }
        Ok(conversation)
    }

    /// Writes a conversation through a synced temporary file and rename.
    ///
    /// # Errors
    ///
    /// Returns an error when the conversation is too large or cannot be
    /// written.
    pub fn save(&self, conversation: &Conversation) -> Result<(), ConversationError> {
        let path = self.path(&conversation.id)?;
        let bytes =
            serde_json::to_vec(conversation).map_err(|error| invalid(&path, &error.to_string()))?;
        if bytes.len() as u64 > MAX_CONVERSATION_FILE_BYTES {
            return Err(invalid(&path, "the conversation exceeds the size limit"));
        }
        fs::create_dir_all(&self.directory)
            .map_err(|source| io_error("create conversation directory", &self.directory, source))?;
        let partial = path.with_extension("json.partial");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&partial)
            .map_err(|source| io_error("create conversation temporary file", &partial, source))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error("write conversation temporary file", &partial, source))?;
        drop(file);
        fs::rename(&partial, &path)
            .map_err(|source| io_error("publish conversation", &path, source))
    }

    /// Deletes a conversation. Deleting a missing one succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists but cannot be removed.
    pub fn delete(&self, id: &str) -> Result<(), ConversationError> {
        let path = self.path(id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(io_error("delete conversation", &path, source)),
        }
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> ConversationError {
    ConversationError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

fn invalid(path: &Path, message: &str) -> ConversationError {
    ConversationError::Invalid {
        path: path.to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversations_round_trip_and_list_newest_first() {
        let directory = tempfile::tempdir().expect("directory");
        let store = ConversationStore::new(directory.path().join("project"));
        assert!(store.list().expect("empty list").is_empty());

        let mut older = Conversation::new(1);
        older.push_user("  \nЧто такое <if>?", 1);
        store.save(&older).expect("save older");
        let mut newer = Conversation::new(2);
        newer.push_user(&"Переведи ".repeat(20), 5);
        store.save(&newer).expect("save newer");

        assert_eq!(store.load(&older.id).expect("load"), older);
        assert_eq!(older.title, "Что такое <if>?");
        assert!(newer.title.ends_with('…'));
        let listed = store.list().expect("list");
        assert_eq!(listed[0].id, newer.id);
        assert_eq!(listed[1].id, older.id);

        store.delete(&newer.id).expect("delete");
        store.delete(&newer.id).expect("delete missing");
        assert_eq!(store.list().expect("list").len(), 1);
    }

    #[test]
    fn ids_that_are_not_uuids_never_reach_the_filesystem() {
        let directory = tempfile::tempdir().expect("directory");
        let store = ConversationStore::new(directory.path());
        for id in ["../escape", "x", "00000000-0000-0000-0000-00000000000g"] {
            assert!(matches!(
                store.load(id),
                Err(ConversationError::NotFound { .. })
            ));
            assert!(store.delete(id).is_err());
        }
    }

    #[test]
    fn a_damaged_file_is_reported_and_skipped_in_the_list() {
        let directory = tempfile::tempdir().expect("directory");
        let store = ConversationStore::new(directory.path());
        let good = Conversation::new(1);
        store.save(&good).expect("save");
        let damaged = uuid::Uuid::new_v4().to_string();
        fs::write(directory.path().join(format!("{damaged}.json")), "{").expect("write");
        assert!(matches!(
            store.load(&damaged),
            Err(ConversationError::Invalid { .. })
        ));
        assert_eq!(store.list().expect("list").len(), 1);
    }
}
