//! Project knowledge: what a person decided about how the translation reads
//! (`style.md`) and the terms it renders the same way that are not strings
//! of the game (`terms.csv`), kept in the `aeria-knowledge` directory at the
//! project root and committed with the translations. The format is
//! described in `docs/formats/knowledge-v1.md`.
//!
//! The crate reads and checks the files and holds the rules every
//! translation follows ([`rules`]).

pub mod glossary;
pub mod knowledge;
pub mod rules;

pub use glossary::{
    Glossary, GlossaryDiagnostic, GlossaryEntry, GlossaryError, contains_term, parse_glossary,
    write_glossary,
};
pub use knowledge::{
    KNOWLEDGE_DIR, Knowledge, KnowledgeFile, KnowledgeTexts, MAX_KNOWLEDGE_BYTES, create_empty,
    read_file,
};
