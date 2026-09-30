//! Project knowledge: what a project decided about style, terms,
//! characters, and story, and the lessons it learned, kept as files in the
//! `aeria-knowledge` directory at the project root and committed with the
//! translations. People and agents edit the files directly; an entry a
//! person settled is marked `settled` and agents do not change it without
//! asking. The format is described in `docs/formats/knowledge-v1.md`.
//!
//! The crate reads and checks the files, picks the knowledge one scene
//! needs, and holds the rules every translation follows ([`rules`]).

pub mod glossary;
pub mod knowledge;
pub mod rules;
pub mod sections;
pub mod voices;

pub use glossary::{
    Glossary, GlossaryDiagnostic, GlossaryEntry, GlossaryError, contains_term, parse_glossary,
    write_glossary,
};
pub use knowledge::{
    Domain, KNOWLEDGE_DIR, Knowledge, KnowledgeFile, KnowledgeTexts, Lesson, MAX_KNOWLEDGE_BYTES,
    read_file, sheet_domain,
};
pub use sections::{Section, parse_sections, write_sections};
pub use voices::{VoiceDiagnostic, VoiceProfile, VoiceProfiles, parse_voices, speaker_label};
