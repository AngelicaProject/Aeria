//! Local source search, indexes, translation memory, and query services.

#![forbid(unsafe_code)]

pub mod index;
pub mod text;

pub use index::{
    MAX_SEARCH_LIMIT, MIN_SIMILARITY, SearchError, SimilarSource, SourceHit, SourceIndex,
    SourcePage, SourceQuery, Tokenizer,
};
pub use text::{plain_text, similarity, text_contains};
