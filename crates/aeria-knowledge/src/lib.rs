//! Project knowledge: what a person decided about how the translation reads
//! (`style.md`), the terms it renders the same way (`terms.csv`), and the
//! sheets of the game whose strings are terms too (`term-sheets.csv`), kept
//! in the `aeria-knowledge` directory at the project root and committed with
//! the translations. The format is
//! described in `docs/formats/knowledge-v1.md`.
//!
//! The crate reads and checks the files and holds the rules every
//! translation follows ([`rules`]).

pub mod glossary;
pub mod knowledge;
pub mod rules;
pub mod sheets;

pub use glossary::{
    Glossary, GlossaryDiagnostic, GlossaryEntry, GlossaryError, TermReview, contains_term,
    parse_glossary, text_of, write_glossary,
};
pub use knowledge::{
    KNOWLEDGE_DIR, Knowledge, KnowledgeFile, KnowledgeTexts, MAX_KNOWLEDGE_BYTES, create_empty,
    read_file,
};
pub use sheets::{TermSheet, parse_term_sheets, write_term_sheets};
