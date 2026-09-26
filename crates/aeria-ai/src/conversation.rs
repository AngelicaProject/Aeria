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
use crate::guidance::ProjectFile;
use crate::images::{ImageError, ImageRef, inspect};
use crate::jobs::JobProposal;
use crate::prompt::AgentMode;
use crate::settings::ModelSelection;
use crate::tools::{UnitLocation, UnitState};

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
    /// The mode of the last message, reused for automatic job updates.
    #[serde(default)]
    pub mode: AgentMode,
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
            mode: AgentMode::default(),
        }
    }

    /// Appends a message Aeria writes on the user's behalf, such as a job
    /// update that wakes Angelica.
    pub fn push_automatic(&mut self, text: &str, now_unix_ms: u64) {
        self.messages.push(ChatMessage::User {
            content: text.to_owned(),
            automatic: true,
            images: Vec::new(),
        });
        self.updated_at_unix_ms = now_unix_ms;
    }

    /// Appends a user message with its attached images, titling a new
    /// conversation after its text.
    pub fn push_user(&mut self, text: &str, images: Vec<ImageRef>, now_unix_ms: u64) {
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
            automatic: false,
            images,
        });
        self.updated_at_unix_ms = now_unix_ms;
    }
}

/// Where a proposed translation stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProposalStatus {
    Pending,
    Applied,
    Rejected,
    /// The string changed after the proposal was made.
    Conflict,
    Failed,
}

/// A change Angelica proposed in a conversation: a translation, or a new
/// version of a project-shared file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProposalRecord {
    pub id: String,
    /// The changed file; `None` for a translation.
    #[serde(default)]
    pub file: Option<ProjectFile>,
    /// A job to start; `None` for other proposals. `target` holds a
    /// one-line summary.
    #[serde(default)]
    pub job: Option<JobProposal>,
    /// A domain Angelica asked to read; `target` holds the requested link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
    /// Translations to mark reviewed; `target` holds Angelica's reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<crate::tools::ReviewBatch>,
    /// A new token limit for a job; `target` holds a one-line summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_limit: Option<crate::jobs::JobLimitProposal>,
    /// The string of a translation; `None` for a file change.
    #[serde(default)]
    pub location: Option<UnitLocation>,
    #[serde(default)]
    pub source: String,
    /// The rebuilt target macro string, or the new file content.
    pub target: String,
    /// The state the change was made against. For a file change,
    /// `expected.target` is the file's content, `None` when it was absent.
    pub expected: UnitState,
    pub status: ProposalStatus,
    pub message: Option<String>,
    pub created_at_unix_ms: u64,
}

/// Most proposals kept per conversation; the oldest settled ones go first.
pub const MAX_PROPOSALS: usize = 2000;

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

    #[error("the image is not accepted: {0}")]
    Image(#[from] ImageError),
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
                if id.ends_with(".proposals") {
                    return None;
                }
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

    fn images_path(&self, id: &str) -> Result<PathBuf, ConversationError> {
        Ok(self.path(id)?.with_extension("images"))
    }

    fn image_path(&self, id: &str, image: &ImageRef) -> Result<PathBuf, ConversationError> {
        if !ImageRef::is_valid_id(&image.id) {
            return Err(invalid(&self.images_path(id)?, "the image ID is invalid"));
        }
        Ok(self
            .images_path(id)?
            .join(format!("{}.{}", image.id, image.format.extension())))
    }

    /// Checks an image and stores it for a conversation under a new ID,
    /// through a synced temporary file and rename. The conversation file
    /// itself need not exist yet.
    ///
    /// # Errors
    ///
    /// Returns [`ConversationError::Image`] for an image that is not
    /// accepted, or an error when it cannot be written.
    pub fn save_image(&self, id: &str, bytes: &[u8]) -> Result<ImageRef, ConversationError> {
        let image = inspect(bytes)?;
        let directory = self.images_path(id)?;
        let path = self.image_path(id, &image)?;
        fs::create_dir_all(&directory)
            .map_err(|source| io_error("create image directory", &directory, source))?;
        let partial = path.with_extension("partial");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|source| io_error("create image temporary file", &partial, source))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error("write image temporary file", &partial, source))?;
        drop(file);
        fs::rename(&partial, &path).map_err(|source| io_error("publish image", &path, source))?;
        Ok(image)
    }

    /// Reads a conversation's image; `None` when its file is gone.
    ///
    /// # Errors
    ///
    /// Returns an error for an unreadable file or one that no longer
    /// matches its reference.
    pub fn load_image(
        &self,
        id: &str,
        image: &ImageRef,
    ) -> Result<Option<Vec<u8>>, ConversationError> {
        let path = self.image_path(id, image)?;
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io_error("open image", &path, source)),
        };
        let mut bytes = Vec::new();
        file.take(crate::images::MAX_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| io_error("read image", &path, source))?;
        let found = inspect(&bytes).map_err(|error| invalid(&path, &error.0))?;
        if (found.format, found.width, found.height) != (image.format, image.width, image.height) {
            return Err(invalid(&path, "the image does not match its reference"));
        }
        Ok(Some(bytes))
    }

    fn proposals_path(&self, id: &str) -> Result<PathBuf, ConversationError> {
        Ok(self.path(id)?.with_extension("proposals.json"))
    }

    /// Loads a conversation's proposals; none is the empty list.
    ///
    /// # Errors
    ///
    /// Returns an error for an unreadable or invalid file.
    pub fn load_proposals(&self, id: &str) -> Result<Vec<ProposalRecord>, ConversationError> {
        let path = self.proposals_path(id)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(io_error("read proposals", &path, source)),
        };
        serde_json::from_slice(&bytes).map_err(|error| invalid(&path, &error.to_string()))
    }

    /// Replaces a conversation's proposals, keeping at most
    /// [`MAX_PROPOSALS`] with pending ones preferred.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written.
    pub fn save_proposals(
        &self,
        id: &str,
        proposals: &[ProposalRecord],
    ) -> Result<(), ConversationError> {
        let path = self.proposals_path(id)?;
        let mut kept = proposals.to_vec();
        while kept.len() > MAX_PROPOSALS {
            let oldest_settled = kept
                .iter()
                .position(|proposal| proposal.status != ProposalStatus::Pending)
                .unwrap_or(0);
            kept.remove(oldest_settled);
        }
        let bytes =
            serde_json::to_vec(&kept).map_err(|error| invalid(&path, &error.to_string()))?;
        fs::create_dir_all(&self.directory)
            .map_err(|source| io_error("create conversation directory", &self.directory, source))?;
        let partial = path.with_extension("json.partial");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&partial)
            .map_err(|source| io_error("create proposals temporary file", &partial, source))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error("write proposals temporary file", &partial, source))?;
        drop(file);
        fs::rename(&partial, &path).map_err(|source| io_error("publish proposals", &path, source))
    }

    /// Deletes a conversation with its proposals and images. Deleting a
    /// missing one succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists but cannot be removed.
    pub fn delete(&self, id: &str) -> Result<(), ConversationError> {
        for path in [self.path(id)?, self.proposals_path(id)?] {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(source) => return Err(io_error("delete conversation", &path, source)),
            }
        }
        let images = self.images_path(id)?;
        match fs::remove_dir_all(&images) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(io_error("delete conversation images", &images, source)),
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
        older.push_user("  \nЧто такое <if>?", Vec::new(), 1);
        store.save(&older).expect("save older");
        let mut newer = Conversation::new(2);
        newer.push_user(&"Переведи ".repeat(20), Vec::new(), 5);
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
    fn proposals_are_stored_beside_the_conversation_and_deleted_with_it() {
        let directory = tempfile::tempdir().expect("directory");
        let store = ConversationStore::new(directory.path());
        let conversation = Conversation::new(1);
        store.save(&conversation).expect("save");
        assert!(
            store
                .load_proposals(&conversation.id)
                .expect("none")
                .is_empty()
        );
        let proposal = ProposalRecord {
            id: "p1".to_owned(),
            file: None,
            job: None,
            web: None,
            review: None,
            job_limit: None,
            location: Some(UnitLocation {
                sheet: "Item".to_owned(),
                row: 1,
                subrow: 0,
                column: Some(0),
            }),
            source: "Bye".to_owned(),
            target: "Пока".to_owned(),
            expected: UnitState {
                target: None,
                review_state: None,
            },
            status: ProposalStatus::Pending,
            message: None,
            created_at_unix_ms: 1,
        };
        store
            .save_proposals(&conversation.id, std::slice::from_ref(&proposal))
            .expect("save proposals");
        assert_eq!(
            store.load_proposals(&conversation.id).expect("load"),
            vec![proposal]
        );
        assert_eq!(store.list().expect("list").len(), 1);
        store.delete(&conversation.id).expect("delete");
        assert!(
            store
                .load_proposals(&conversation.id)
                .expect("gone")
                .is_empty()
        );
    }

    #[test]
    fn images_are_stored_beside_the_conversation_and_deleted_with_it() {
        let directory = tempfile::tempdir().expect("directory");
        let store = ConversationStore::new(directory.path());
        let mut conversation = Conversation::new(1);
        let bytes = crate::images::tests::png(16, 9);
        let image = store.save_image(&conversation.id, &bytes).expect("image");
        assert_eq!((image.width, image.height), (16, 9));
        conversation.push_user("", vec![image.clone()], 2);
        store.save(&conversation).expect("save");
        assert_eq!(
            store.load(&conversation.id).expect("load").messages,
            conversation.messages
        );
        assert_eq!(store.list().expect("list").len(), 1);
        assert_eq!(
            store.load_image(&conversation.id, &image).expect("read"),
            Some(bytes)
        );
        assert!(matches!(
            store.save_image(&conversation.id, b"GIF89a"),
            Err(ConversationError::Image(_))
        ));
        let mut forged = image.clone();
        forged.id = "../../escape".to_owned();
        assert!(store.load_image(&conversation.id, &forged).is_err());

        store.delete(&conversation.id).expect("delete");
        assert_eq!(
            store.load_image(&conversation.id, &image).expect("gone"),
            None
        );
        assert!(
            !directory
                .path()
                .join(format!("{}.images", conversation.id))
                .exists()
        );
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
