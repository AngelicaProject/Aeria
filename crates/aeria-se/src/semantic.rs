//! Validity and user-facing text of parsed macro text.

use crate::catalog::SemanticFamily;
use crate::syntax::{Diagnostic, ExprKind, MacroString, SyntaxKind, SyntaxNode};

/// The intrinsic validity of macro text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticValidity {
    /// Valid, and every construct is understood.
    ValidAndUnderstood,
    /// Valid, with constructs that are preserved without interpretation: a
    /// macro code without a name, or a macro whose meaning is not
    /// established.
    ValidWithOpaque,
    /// Not valid: the text has diagnostics and cannot be written.
    InvalidUnsafe,
}

/// The validity of macro text and the diagnostics that make it invalid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticValidation {
    status: SemanticValidity,
    diagnostics: Vec<Diagnostic>,
}

impl SemanticValidation {
    #[must_use]
    pub const fn status(&self) -> SemanticValidity {
        self.status
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

impl MacroString {
    /// Validates the text without evaluating it.
    #[must_use]
    pub fn semantic_validation(&self) -> SemanticValidation {
        let status = if !self.is_well_formed() {
            SemanticValidity::InvalidUnsafe
        } else if contains_opaque(self.nodes()) {
            SemanticValidity::ValidWithOpaque
        } else {
            SemanticValidity::ValidAndUnderstood
        };
        SemanticValidation {
            status,
            diagnostics: self.diagnostics().to_vec(),
        }
    }

    /// The text players read: text and the user-facing content of macros,
    /// such as the branches of `<if>`, in order. Where a macro separates
    /// two pieces of text, they are joined with a space.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut pieces = Vec::new();
        collect_text(self.nodes(), &mut pieces, &mut false);
        let mut text = String::new();
        for (piece, separated) in pieces {
            if separated && !text.is_empty() && !text.ends_with(char::is_whitespace) {
                text.push(' ');
            }
            text.push_str(&piece);
        }
        text.trim().to_owned()
    }

    /// Whether the text is formatting only: it is well formed and its
    /// user-facing text contains no letter. Punctuation, digits, spacing,
    /// numbers, and icons usually differ between game languages by
    /// convention rather than by translation. Empty text is formatting
    /// only.
    #[must_use]
    pub fn is_formatting_only(&self) -> bool {
        let mut pieces = Vec::new();
        collect_text(self.nodes(), &mut pieces, &mut false);
        self.is_well_formed()
            && !pieces
                .iter()
                .any(|(piece, _)| piece.chars().any(char::is_alphabetic))
    }
}

/// The family of a parsed macro node; `None` for constructs Aeria does not
/// understand.
pub(crate) fn family(node: &SyntaxNode) -> Option<SemanticFamily> {
    match &node.kind {
        SyntaxKind::Macro(syntax) => syntax.spec.map(|spec| spec.family),
        _ => None,
    }
}

fn contains_opaque(nodes: &[SyntaxNode]) -> bool {
    nodes.iter().any(|node| match &node.kind {
        SyntaxKind::Macro(syntax) => {
            syntax
                .spec
                .is_none_or(|spec| spec.family == SemanticFamily::OpaqueProtected)
                || syntax.args.iter().any(|arg| match &arg.kind {
                    ExprKind::Str(nodes) => contains_opaque(nodes),
                    _ => false,
                })
        }
        SyntaxKind::Text(_) => false,
        SyntaxKind::Raw(_) | SyntaxKind::Error => true,
    })
}

fn collect_text(nodes: &[SyntaxNode], pieces: &mut Vec<(String, bool)>, separated: &mut bool) {
    for node in nodes {
        match &node.kind {
            SyntaxKind::Text(text) => {
                pieces.push((text.clone(), *separated));
                *separated = false;
            }
            SyntaxKind::Macro(syntax) => {
                *separated = true;
                if let Some(spec) = syntax.spec {
                    for (index, arg) in syntax.args.iter().enumerate() {
                        if let ExprKind::Str(nodes) = &arg.kind
                            && spec.is_translatable_arg(index)
                        {
                            collect_text(nodes, pieces, separated);
                            *separated = true;
                        }
                    }
                }
            }
            SyntaxKind::Raw(_) | SyntaxKind::Error => *separated = true,
        }
    }
}
