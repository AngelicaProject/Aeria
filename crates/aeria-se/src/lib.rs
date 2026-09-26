//! Game strings for Aeria: the string model, macro text, validation, and
//! tagged text for assisted translation.
//!
//! - [`bytes`] converts `SeString` bytes to the string model without loss.
//! - [`catalog`] describes every named macro.
//! - [`parse`] and [`print`] convert between the model and macro text, the
//!   written form people and the translation agent read and edit.
//! - [`codec`] converts between bytes and macro text.
//!
//! The crate does not evaluate game expressions.

#![forbid(unsafe_code)]

pub mod bytes;
pub mod catalog;
pub mod codec;
pub mod preview;
mod semantic;
mod syntax;
mod tagged;

pub use catalog::SemanticFamily;
pub use semantic::{SemanticValidation, SemanticValidity};
pub use syntax::{
    Diagnostic, DiagnosticKind, ExprKind, ExprSyntax, MAX_NESTING_DEPTH, MacroString, MacroSyntax,
    Span, SyntaxKind, SyntaxNode, Written, parse, print,
};
pub use tagged::{
    Tag, TaggedError, TaggedText, check_assisted_structure, describe as describe_macro, project,
    rebuild,
};
