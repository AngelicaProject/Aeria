//! Game strings for Aeria: the string model, macro text, validation, and
//! the structure policy for assisted translation.
//!
//! - [`bytes`] converts `SeString` bytes to the string model without loss.
//! - [`catalog`] describes every named macro.
//! - [`parse`] and [`print`] convert between the model and macro text, the
//!   written form people and the translation agent read and edit.
//! - [`codec`] converts between bytes and macro text.
//!
//! The crate does not evaluate game expressions.

#![forbid(unsafe_code)]

mod assisted;
pub mod bytes;
pub mod catalog;
pub mod codec;
mod semantic;
pub mod speaker;
mod syntax;

pub use assisted::{
    Construct, ConstructRule, StructureError, authoring_reference, check_assisted_structure,
    check_assisted_structure_with, construct_rule, constructs, describe as describe_macro,
    missing_game_data,
};
pub use catalog::SemanticFamily;
pub use semantic::{SemanticValidation, SemanticValidity};
pub use syntax::{
    Diagnostic, DiagnosticKind, ExprKind, ExprSyntax, MAX_NESTING_DEPTH, MacroString, MacroSyntax,
    Span, SyntaxKind, SyntaxNode, Written, parse, print,
};
