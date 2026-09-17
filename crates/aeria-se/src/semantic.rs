//! Semantic projection, validation, text extraction, and protected structure
//! comparison for the lossless syntax tree.

use super::{
    ComparisonOperator, Diagnostic, Expression, ExpressionKind, KnownMacro, MacroNode, MacroString,
    OpaquePayload, PlaceholderExpression, Safety, Span, SyntaxKind, SyntaxNode, UnaryExpression,
};

/// The broad semantic family of a construct understood by Aeria.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticFamily {
    /// Ordinary text that can be translated.
    TranslatableText,
    /// Formatting or presentation state.
    FormattingPresentation,
    /// Conditional, selection, or language-dependent control flow.
    ConditionalSelection,
    /// Values supplied by runtime or contextual state.
    RuntimeContextValue,
    /// References to game-owned data.
    GameDataReference,
    /// Layout and textual control constructs.
    LayoutTextualControl,
    /// An expression structure, used for expression nodes rather than macros.
    Expression,
    /// A construct that must be preserved without assigning it semantics.
    OpaqueProtected,
}

impl KnownMacro {
    /// Every macro code defined by Lumina 7.7.0, in its stable enum order.
    ///
    /// Keeping this list next to the exhaustive classification match makes a
    /// newly added upstream macro a visible compile-time review point.
    pub const ALL: [Self; 57] = [
        Self::SetResetTime,
        Self::SetTime,
        Self::If,
        Self::Switch,
        Self::PcName,
        Self::IfPcGender,
        Self::IfPcName,
        Self::Josa,
        Self::Josaro,
        Self::IfSelf,
        Self::NewLine,
        Self::Wait,
        Self::Icon,
        Self::Color,
        Self::EdgeColor,
        Self::ShadowColor,
        Self::SoftHyphen,
        Self::Key,
        Self::Scale,
        Self::Bold,
        Self::Italic,
        Self::Edge,
        Self::Shadow,
        Self::NonBreakingSpace,
        Self::Icon2,
        Self::Hyphen,
        Self::Num,
        Self::Hex,
        Self::Kilo,
        Self::Byte,
        Self::Sec,
        Self::Time,
        Self::Float,
        Self::Link,
        Self::Sheet,
        Self::String,
        Self::Caps,
        Self::Head,
        Self::Split,
        Self::HeadAll,
        Self::Fixed,
        Self::Lower,
        Self::JaNoun,
        Self::EnNoun,
        Self::DeNoun,
        Self::FrNoun,
        Self::ChNoun,
        Self::LowerHead,
        Self::SheetSub,
        Self::SwitchPlatform,
        Self::ColorType,
        Self::EdgeColorType,
        Self::Ruby,
        Self::Digit,
        Self::Ordinal,
        Self::Sound,
        Self::LevelPos,
    ];

    /// Returns the conservative semantic family supported by Aeria.
    #[must_use]
    pub const fn semantic_family(self) -> SemanticFamily {
        match self {
            Self::SetResetTime
            | Self::SetTime
            | Self::PcName
            | Self::Num
            | Self::Hex
            | Self::Kilo
            | Self::Byte
            | Self::Sec
            | Self::Float
            | Self::Digit
            | Self::Ordinal => SemanticFamily::RuntimeContextValue,
            Self::If
            | Self::Switch
            | Self::IfPcGender
            | Self::IfPcName
            | Self::Josa
            | Self::Josaro
            | Self::IfSelf => SemanticFamily::ConditionalSelection,
            Self::Color
            | Self::EdgeColor
            | Self::ShadowColor
            | Self::Bold
            | Self::Italic
            | Self::ColorType
            | Self::EdgeColorType => SemanticFamily::FormattingPresentation,
            Self::Sheet
            | Self::SheetSub
            | Self::JaNoun
            | Self::EnNoun
            | Self::DeNoun
            | Self::FrNoun
            | Self::ChNoun
            | Self::SwitchPlatform
            | Self::LevelPos => SemanticFamily::GameDataReference,
            Self::NewLine
            | Self::Wait
            | Self::Icon
            | Self::SoftHyphen
            | Self::NonBreakingSpace
            | Self::Hyphen
            | Self::Link
            | Self::Ruby
            | Self::Sound => SemanticFamily::LayoutTextualControl,
            Self::String
            | Self::Caps
            | Self::Head
            | Self::HeadAll
            | Self::Lower
            | Self::LowerHead => SemanticFamily::TranslatableText,
            // Lumina 7.7.0 exposes these names and/or argument shapes, but
            // does not establish their runtime meaning. Preserve them as
            // protected constructs until that contract is documented.
            Self::Key
            | Self::Scale
            | Self::Edge
            | Self::Shadow
            | Self::Icon2
            | Self::Time
            | Self::Split
            | Self::Fixed => SemanticFamily::OpaqueProtected,
        }
    }
}

/// A semantic root or nested node, retaining its CST span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticNode {
    /// The exact CST span occupied by this node.
    pub span: Span,
    /// The semantic interpretation of this node.
    pub kind: SemanticNodeKind,
}

/// Semantic interpretation of a syntax node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticNodeKind {
    /// Ordinary user-facing text.
    Text,
    /// An escaped user-facing character.
    Escape { character: char },
    /// A known Lumina macro with a conservative family classification.
    Macro(SemanticMacro),
    /// A protected construct whose meaning is not assigned by Aeria.
    Opaque(SemanticOpaque),
    /// A malformed CST recovery node.
    Malformed,
}

/// A semantically classified known macro.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticMacro {
    /// The Lumina macro code.
    pub name: KnownMacro,
    /// The broad semantic family of the macro.
    pub family: SemanticFamily,
    /// Ordered semantic arguments.
    pub arguments: Vec<SemanticExpression>,
}

/// An opaque syntax node retained as protected data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticOpaque {
    /// The opaque payload category and preserved payload details.
    pub kind: OpaqueSemanticKind,
    /// Ordered arguments when the opaque payload had expression-shaped data.
    pub arguments: Vec<SemanticExpression>,
}

/// The semantic category of an opaque syntax node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpaqueSemanticKind {
    /// An unknown named macro.
    NamedMacro { name: String },
    /// An unsupported numeric macro payload.
    Macro { code: u8 },
    /// A raw fallback payload.
    Raw { bytes: Vec<u8> },
}

/// A semantic expression retaining its source span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticExpression {
    /// The exact CST span occupied by this expression.
    pub span: Span,
    /// The semantic interpretation of this expression.
    pub kind: SemanticExpressionKind,
}

/// Semantic interpretation of an expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticExpressionKind {
    /// A literal unsigned integer.
    Integer { value: u32 },
    /// A nested string expression, including any protected child nodes.
    String { parts: Vec<SemanticNode> },
    /// A runtime/context placeholder.
    RuntimePlaceholder(PlaceholderExpression),
    /// A runtime/context parameter expression.
    RuntimeParameter {
        operator: UnaryExpression,
        operand: Box<SemanticExpression>,
    },
    /// A comparison used by conditional control flow.
    Comparison {
        operator: ComparisonOperator,
        left: Box<SemanticExpression>,
        right: Box<SemanticExpression>,
    },
    /// An opaque expression fallback.
    Opaque { bytes: Vec<u8> },
    /// A malformed expression recovery node.
    Malformed { recovered: Vec<SemanticNode> },
}

/// A deterministic range of user-facing text in a CST.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextRange {
    /// The source span to use when referring back to the CST.
    pub span: Span,
    /// Whether the range is ordinary text or an escaped character.
    pub kind: TextRangeKind,
}

/// The kind of source text exposed for translation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextRangeKind {
    /// Ordinary unescaped text.
    Text,
    /// A user-facing character represented by a source escape.
    Escape,
}

/// The protected, non-prose structure of a macro string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedStructure {
    nodes: Vec<ProtectedNode>,
    comparable: bool,
}

impl ProtectedStructure {
    /// Returns protected nodes in source order.
    #[must_use]
    pub fn nodes(&self) -> &[ProtectedNode] {
        &self.nodes
    }

    /// Returns whether the structure came from a well-formed document.
    #[must_use]
    pub const fn is_comparable(&self) -> bool {
        self.comparable
    }
}

/// One protected macro or opaque construct in source order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedNode {
    /// The exact CST span occupied by this protected node.
    pub span: Span,
    /// The protected node representation.
    pub kind: ProtectedNodeKind,
}

/// Protected node representation used by strict structure comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtectedNodeKind {
    /// A normalized slot for one or more adjacent user-facing text/escape
    /// nodes. The prose itself is intentionally omitted.
    TextSlot,
    /// A known macro and its non-prose argument structure.
    Macro {
        name: KnownMacro,
        family: SemanticFamily,
        arguments: Vec<ProtectedExpression>,
    },
    /// An opaque construct identified by its exact source spelling.
    Opaque { identity: OpaqueIdentity },
    /// A recovery node from a malformed document.
    Malformed { spelling: String },
}

/// Exact identity used when comparing opaque constructs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpaqueIdentity {
    /// An unknown named macro and its full source spelling.
    NamedMacro { name: String, spelling: String },
    /// An unsupported macro payload and its full source spelling.
    Macro { code: u8, spelling: String },
    /// A raw fallback payload and its full source spelling.
    Raw { bytes: Vec<u8>, spelling: String },
}

/// A protected expression used for structure comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedExpression {
    /// The exact CST span occupied by this expression.
    pub span: Span,
    /// The non-prose expression representation.
    pub kind: ProtectedExpressionKind,
}

/// Non-prose expression structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProtectedExpressionKind {
    /// An integer literal.
    Integer { value: u32 },
    /// A numeric identifier belonging to a known game-data reference.
    GameReference { value: u32 },
    /// A user-facing string expression with protected children only.
    String { nodes: Vec<ProtectedNode> },
    /// A protected string operand whose spelling is part of structure.
    ProtectedString {
        spelling: String,
        game_reference: bool,
    },
    /// A runtime/context placeholder.
    RuntimePlaceholder(PlaceholderExpression),
    /// A runtime/context parameter expression.
    RuntimeParameter {
        operator: UnaryExpression,
        operand: Box<ProtectedExpression>,
    },
    /// A comparison expression.
    Comparison {
        operator: ComparisonOperator,
        left: Box<ProtectedExpression>,
        right: Box<ProtectedExpression>,
    },
    /// An opaque expression fallback identified by source spelling.
    Opaque { spelling: String },
    /// A malformed expression recovery node.
    Malformed { spelling: String },
}

/// The result of semantic validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticValidation {
    status: SemanticValidity,
    diagnostics: Vec<Diagnostic>,
}

impl SemanticValidation {
    /// Returns the validation status.
    #[must_use]
    pub const fn status(&self) -> SemanticValidity {
        self.status
    }

    /// Returns parser diagnostics that make the document unsafe.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// The intrinsic validity state of a semantic document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticValidity {
    /// The document is valid and contains only understood constructs.
    ValidAndUnderstood,
    /// The document is valid but contains protected opaque constructs.
    ValidWithOpaque,
    /// The document is malformed and unsafe for semantic editing/export.
    InvalidUnsafe,
}

/// The complete derived semantic view of a parsed document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticAnalysis {
    nodes: Vec<SemanticNode>,
    text_ranges: Vec<TextRange>,
    structure: ProtectedStructure,
    validation: SemanticValidation,
}

impl SemanticAnalysis {
    /// Returns semantic nodes in source order.
    #[must_use]
    pub fn nodes(&self) -> &[SemanticNode] {
        &self.nodes
    }

    /// Returns deterministic user-facing text ranges in traversal order.
    #[must_use]
    pub fn text_ranges(&self) -> &[TextRange] {
        &self.text_ranges
    }

    /// Alias for callers that use the translation-oriented terminology.
    #[must_use]
    pub fn translatable_text_ranges(&self) -> &[TextRange] {
        self.text_ranges()
    }

    /// Returns the protected non-prose structure.
    #[must_use]
    pub const fn structure(&self) -> &ProtectedStructure {
        &self.structure
    }

    /// Returns intrinsic semantic validation.
    #[must_use]
    pub const fn validation(&self) -> &SemanticValidation {
        &self.validation
    }
}

/// Builds the semantic projection of one parsed CST.
#[must_use]
pub fn analyze(document: &MacroString) -> SemanticAnalysis {
    let mut text_ranges = Vec::new();
    let nodes = project_nodes(document, document.nodes(), &mut text_ranges);
    let structure = ProtectedStructure {
        nodes: project_protected_nodes(document, document.nodes()),
        comparable: !matches!(document.safety(), Safety::Malformed),
    };
    let status = match document.safety() {
        Safety::Understood if contains_opaque_protected_macro(&structure.nodes) => {
            SemanticValidity::ValidWithOpaque
        }
        Safety::Understood => SemanticValidity::ValidAndUnderstood,
        Safety::Opaque => SemanticValidity::ValidWithOpaque,
        Safety::Malformed => SemanticValidity::InvalidUnsafe,
    };

    SemanticAnalysis {
        nodes,
        text_ranges,
        structure,
        validation: SemanticValidation {
            status,
            diagnostics: document.diagnostics().to_vec(),
        },
    }
}

/// Validates one parsed CST without executing expressions or resolving data.
#[must_use]
pub fn validate(document: &MacroString) -> SemanticValidation {
    analyze(document).validation
}

/// The outcome of comparing two protected structures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructureCompatibility {
    /// The protected structures are equivalent.
    Compatible,
    /// The structures are both comparable but differ.
    Incompatible,
    /// At least one input is malformed and comparison cannot be trusted.
    CannotSafelyCompare,
}

/// A structured result from strict protected-structure comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureComparison {
    /// Overall conservative comparison result.
    pub compatibility: StructureCompatibility,
    /// Differences in deterministic source/traversal order.
    pub differences: Vec<StructureDifference>,
}

/// One protected-structure difference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureDifference {
    /// The source span, when a source node exists for this difference.
    pub source_span: Option<Span>,
    /// The target span, when a target node exists for this difference.
    pub target_span: Option<Span>,
    /// The conservative difference category.
    pub kind: StructureDifferenceKind,
}

/// Difference categories reported by strict comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StructureDifferenceKind {
    /// A protected source node is absent from the target.
    MissingProtectedNode,
    /// A protected target node was added.
    ExtraProtectedNode,
    /// The protected node kind changed.
    DifferentProtectedKind,
    /// Protected nodes were reordered.
    ReorderedProtectedNodes,
    /// An opaque construct's exact identity changed.
    ChangedOpaqueConstruct,
    /// An expression or conditional structure changed.
    ChangedExpressionStructure,
    /// A runtime/context expression changed.
    ChangedRuntimeExpression,
    /// A game-data identity changed.
    ChangedGameReference,
    /// At least one input was malformed.
    UnsafeInput,
}

/// Compares the protected structures of two semantic analyses.
#[must_use]
pub fn compare_analyses(
    source: &SemanticAnalysis,
    target: &SemanticAnalysis,
) -> StructureComparison {
    compare_structures(&source.structure, &target.structure)
}

/// Parses and compares two macro strings using the semantic structure layer.
#[must_use]
pub fn compare_macro_strings(source: &str, target: &str) -> StructureComparison {
    let source = analyze(&super::parse(source));
    let target = analyze(&super::parse(target));
    compare_analyses(&source, &target)
}

/// Compares two protected structures conservatively.
#[must_use]
pub fn compare_structures(
    source: &ProtectedStructure,
    target: &ProtectedStructure,
) -> StructureComparison {
    if !source.comparable || !target.comparable {
        return StructureComparison {
            compatibility: StructureCompatibility::CannotSafelyCompare,
            differences: vec![StructureDifference {
                source_span: source.nodes.first().map(|node| node.span),
                target_span: target.nodes.first().map(|node| node.span),
                kind: StructureDifferenceKind::UnsafeInput,
            }],
        };
    }

    let mut differences = Vec::new();
    compare_node_lists(&source.nodes, &target.nodes, &mut differences);
    StructureComparison {
        compatibility: if differences.is_empty() {
            StructureCompatibility::Compatible
        } else {
            StructureCompatibility::Incompatible
        },
        differences,
    }
}

fn project_nodes(
    document: &MacroString,
    nodes: &[SyntaxNode],
    text_ranges: &mut Vec<TextRange>,
) -> Vec<SemanticNode> {
    nodes
        .iter()
        .map(|node| SemanticNode {
            span: node.span,
            kind: match &node.kind {
                SyntaxKind::Text => {
                    text_ranges.push(TextRange {
                        span: node.span,
                        kind: TextRangeKind::Text,
                    });
                    SemanticNodeKind::Text
                }
                SyntaxKind::Escape { character } => {
                    text_ranges.push(TextRange {
                        span: node.span,
                        kind: TextRangeKind::Escape,
                    });
                    SemanticNodeKind::Escape {
                        character: *character,
                    }
                }
                SyntaxKind::Macro(macro_node) => {
                    SemanticNodeKind::Macro(project_macro(document, macro_node, text_ranges))
                }
                SyntaxKind::Opaque(payload) => {
                    SemanticNodeKind::Opaque(project_opaque(document, payload))
                }
                SyntaxKind::Malformed => SemanticNodeKind::Malformed,
            },
        })
        .collect()
}

fn project_macro(
    document: &MacroString,
    macro_node: &MacroNode,
    text_ranges: &mut Vec<TextRange>,
) -> SemanticMacro {
    SemanticMacro {
        name: macro_node.name,
        family: macro_node.name.semantic_family(),
        arguments: macro_node
            .arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                if is_user_facing_argument(macro_node.name, index) {
                    project_expression(document, argument, text_ranges)
                } else {
                    let mut ignored_text_ranges = Vec::new();
                    project_expression(document, argument, &mut ignored_text_ranges)
                }
            })
            .collect(),
    }
}

fn project_opaque(document: &MacroString, payload: &OpaquePayload) -> SemanticOpaque {
    let mut ignored_text_ranges = Vec::new();
    match payload {
        OpaquePayload::NamedMacro { name, arguments } => SemanticOpaque {
            kind: OpaqueSemanticKind::NamedMacro { name: name.clone() },
            arguments: arguments
                .iter()
                .map(|argument| project_expression(document, argument, &mut ignored_text_ranges))
                .collect(),
        },
        OpaquePayload::Macro { code, arguments } => SemanticOpaque {
            kind: OpaqueSemanticKind::Macro { code: *code },
            arguments: arguments
                .iter()
                .map(|argument| project_expression(document, argument, &mut ignored_text_ranges))
                .collect(),
        },
        OpaquePayload::Raw { bytes } => SemanticOpaque {
            kind: OpaqueSemanticKind::Raw {
                bytes: bytes.clone(),
            },
            arguments: Vec::new(),
        },
    }
}

fn project_expression(
    document: &MacroString,
    expression: &Expression,
    text_ranges: &mut Vec<TextRange>,
) -> SemanticExpression {
    let kind = match &expression.kind {
        ExpressionKind::UnsignedInteger { value } => {
            SemanticExpressionKind::Integer { value: *value }
        }
        ExpressionKind::String { parts } => SemanticExpressionKind::String {
            parts: project_nodes(document, parts, text_ranges),
        },
        ExpressionKind::Placeholder(placeholder) => {
            SemanticExpressionKind::RuntimePlaceholder(*placeholder)
        }
        ExpressionKind::Unary { operator, operand } => SemanticExpressionKind::RuntimeParameter {
            operator: *operator,
            operand: Box::new(project_expression(document, operand, text_ranges)),
        },
        ExpressionKind::Binary {
            operator,
            left,
            right,
        } => SemanticExpressionKind::Comparison {
            operator: *operator,
            left: Box::new(project_expression(document, left, text_ranges)),
            right: Box::new(project_expression(document, right, text_ranges)),
        },
        ExpressionKind::Opaque { bytes } => SemanticExpressionKind::Opaque {
            bytes: bytes.clone(),
        },
        ExpressionKind::Malformed { recovered } => SemanticExpressionKind::Malformed {
            recovered: project_nodes(document, recovered, text_ranges),
        },
    };
    SemanticExpression {
        span: expression.span,
        kind,
    }
}

fn project_protected_nodes(document: &MacroString, nodes: &[SyntaxNode]) -> Vec<ProtectedNode> {
    let mut projected = Vec::new();
    let mut pending_text_slot: Option<Span> = None;

    for node in nodes {
        match &node.kind {
            SyntaxKind::Text | SyntaxKind::Escape { .. } => {
                pending_text_slot = Some(match pending_text_slot {
                    Some(span) => Span::new(span.start(), node.span.end()),
                    None => node.span,
                });
            }
            SyntaxKind::Macro(macro_node) => {
                push_text_slot(&mut projected, &mut pending_text_slot);
                projected.push(ProtectedNode {
                    span: node.span,
                    kind: ProtectedNodeKind::Macro {
                        name: macro_node.name,
                        family: macro_node.name.semantic_family(),
                        arguments: macro_node
                            .arguments
                            .iter()
                            .enumerate()
                            .map(|(index, argument)| {
                                project_protected_expression(
                                    document,
                                    argument,
                                    macro_node.name,
                                    index,
                                )
                            })
                            .collect(),
                    },
                });
            }
            SyntaxKind::Opaque(payload) => {
                push_text_slot(&mut projected, &mut pending_text_slot);
                projected.push(ProtectedNode {
                    span: node.span,
                    kind: ProtectedNodeKind::Opaque {
                        identity: opaque_identity(document, node.span, payload),
                    },
                });
            }
            SyntaxKind::Malformed => {
                push_text_slot(&mut projected, &mut pending_text_slot);
                projected.push(ProtectedNode {
                    span: node.span,
                    kind: ProtectedNodeKind::Malformed {
                        spelling: source_spelling(document, node.span),
                    },
                });
            }
        }
    }
    push_text_slot(&mut projected, &mut pending_text_slot);
    projected
}

fn push_text_slot(nodes: &mut Vec<ProtectedNode>, pending_text_slot: &mut Option<Span>) {
    if let Some(span) = pending_text_slot.take() {
        nodes.push(ProtectedNode {
            span,
            kind: ProtectedNodeKind::TextSlot,
        });
    }
}

fn project_protected_expression(
    document: &MacroString,
    expression: &Expression,
    macro_name: KnownMacro,
    argument_index: usize,
) -> ProtectedExpression {
    let kind = match &expression.kind {
        ExpressionKind::UnsignedInteger { value } => {
            if is_game_reference_argument(macro_name, argument_index) {
                ProtectedExpressionKind::GameReference { value: *value }
            } else {
                ProtectedExpressionKind::Integer { value: *value }
            }
        }
        ExpressionKind::String { .. } if !is_user_facing_argument(macro_name, argument_index) => {
            ProtectedExpressionKind::ProtectedString {
                spelling: source_spelling(document, expression.span),
                game_reference: is_game_reference_argument(macro_name, argument_index),
            }
        }
        ExpressionKind::String { parts } => ProtectedExpressionKind::String {
            nodes: project_protected_nodes(document, parts),
        },
        ExpressionKind::Placeholder(placeholder) => {
            ProtectedExpressionKind::RuntimePlaceholder(*placeholder)
        }
        ExpressionKind::Unary { operator, operand } => ProtectedExpressionKind::RuntimeParameter {
            operator: *operator,
            operand: Box::new(project_protected_expression(
                document,
                operand,
                macro_name,
                argument_index,
            )),
        },
        ExpressionKind::Binary {
            operator,
            left,
            right,
        } => ProtectedExpressionKind::Comparison {
            operator: *operator,
            left: Box::new(project_protected_expression(
                document,
                left,
                macro_name,
                argument_index,
            )),
            right: Box::new(project_protected_expression(
                document,
                right,
                macro_name,
                argument_index,
            )),
        },
        ExpressionKind::Opaque { .. } => ProtectedExpressionKind::Opaque {
            spelling: source_spelling(document, expression.span),
        },
        ExpressionKind::Malformed { .. } => ProtectedExpressionKind::Malformed {
            spelling: source_spelling(document, expression.span),
        },
    };
    ProtectedExpression {
        span: expression.span,
        kind,
    }
}

fn is_game_reference_argument(macro_name: KnownMacro, argument_index: usize) -> bool {
    match macro_name {
        KnownMacro::Sheet => matches!(argument_index, 0..=2),
        KnownMacro::SheetSub => matches!(argument_index, 0..=5),
        KnownMacro::JaNoun
        | KnownMacro::EnNoun
        | KnownMacro::DeNoun
        | KnownMacro::FrNoun
        | KnownMacro::ChNoun => matches!(argument_index, 0 | 2),
        KnownMacro::SwitchPlatform | KnownMacro::LevelPos => argument_index == 0,
        _ => false,
    }
}

fn is_user_facing_argument(macro_name: KnownMacro, argument_index: usize) -> bool {
    if macro_name.semantic_family() == SemanticFamily::OpaqueProtected {
        return false;
    }

    match macro_name {
        KnownMacro::If | KnownMacro::Switch => argument_index >= 1,
        KnownMacro::IfPcGender | KnownMacro::IfSelf => matches!(argument_index, 1 | 2),
        KnownMacro::IfPcName => matches!(argument_index, 2 | 3),
        KnownMacro::Josa | KnownMacro::Josaro => matches!(argument_index, 1 | 2),
        KnownMacro::String
        | KnownMacro::Caps
        | KnownMacro::Head
        | KnownMacro::HeadAll
        | KnownMacro::Lower
        | KnownMacro::LowerHead => argument_index == 0,
        KnownMacro::Split => matches!(argument_index, 0 | 1),
        KnownMacro::Link => argument_index == 4,
        KnownMacro::Ruby => matches!(argument_index, 0 | 1),
        _ => false,
    }
}

fn opaque_identity(document: &MacroString, span: Span, payload: &OpaquePayload) -> OpaqueIdentity {
    let spelling = source_spelling(document, span);
    match payload {
        OpaquePayload::NamedMacro { name, .. } => OpaqueIdentity::NamedMacro {
            name: name.clone(),
            spelling,
        },
        OpaquePayload::Macro { code, .. } => OpaqueIdentity::Macro {
            code: *code,
            spelling,
        },
        OpaquePayload::Raw { bytes } => OpaqueIdentity::Raw {
            bytes: bytes.clone(),
            spelling,
        },
    }
}

fn source_spelling(document: &MacroString, span: Span) -> String {
    document.slice(span).unwrap_or_default().to_owned()
}

fn compare_node_lists(
    source: &[ProtectedNode],
    target: &[ProtectedNode],
    differences: &mut Vec<StructureDifference>,
) {
    if source.len() == target.len()
        && source.len() > 1
        && source.iter().map(protected_node_key).collect::<Vec<_>>()
            != target.iter().map(protected_node_key).collect::<Vec<_>>()
        && same_multiset(source, target)
    {
        differences.push(StructureDifference {
            source_span: source.first().map(|node| node.span),
            target_span: target.first().map(|node| node.span),
            kind: StructureDifferenceKind::ReorderedProtectedNodes,
        });
        return;
    }

    let common = source.len().min(target.len());
    for index in 0..common {
        compare_protected_nodes(&source[index], &target[index], differences);
    }
    for node in &source[common..] {
        differences.push(StructureDifference {
            source_span: Some(node.span),
            target_span: None,
            kind: StructureDifferenceKind::MissingProtectedNode,
        });
    }
    for node in &target[common..] {
        differences.push(StructureDifference {
            source_span: None,
            target_span: Some(node.span),
            kind: StructureDifferenceKind::ExtraProtectedNode,
        });
    }
}

fn compare_protected_nodes(
    source: &ProtectedNode,
    target: &ProtectedNode,
    differences: &mut Vec<StructureDifference>,
) {
    match (&source.kind, &target.kind) {
        (ProtectedNodeKind::TextSlot, ProtectedNodeKind::TextSlot) => {}
        (
            ProtectedNodeKind::Macro {
                name: source_name,
                arguments: source_arguments,
                ..
            },
            ProtectedNodeKind::Macro {
                name: target_name,
                arguments: target_arguments,
                ..
            },
        ) if source_name == target_name => {
            compare_expression_lists(
                source_arguments,
                target_arguments,
                source.span,
                target.span,
                differences,
            );
        }
        (
            ProtectedNodeKind::Opaque {
                identity: source_identity,
            },
            ProtectedNodeKind::Opaque {
                identity: target_identity,
            },
        ) if source_identity == target_identity => {}
        (ProtectedNodeKind::Opaque { .. }, ProtectedNodeKind::Opaque { .. }) => {
            differences.push(StructureDifference {
                source_span: Some(source.span),
                target_span: Some(target.span),
                kind: StructureDifferenceKind::ChangedOpaqueConstruct,
            });
        }
        _ => differences.push(StructureDifference {
            source_span: Some(source.span),
            target_span: Some(target.span),
            kind: StructureDifferenceKind::DifferentProtectedKind,
        }),
    }
}

fn compare_expression_lists(
    source: &[ProtectedExpression],
    target: &[ProtectedExpression],
    source_span: Span,
    target_span: Span,
    differences: &mut Vec<StructureDifference>,
) {
    if source.len() != target.len() {
        differences.push(StructureDifference {
            source_span: Some(source_span),
            target_span: Some(target_span),
            kind: StructureDifferenceKind::ChangedExpressionStructure,
        });
        return;
    }
    for (source, target) in source.iter().zip(target) {
        compare_protected_expressions(source, target, differences);
    }
}

#[allow(clippy::too_many_lines)]
fn compare_protected_expressions(
    source: &ProtectedExpression,
    target: &ProtectedExpression,
    differences: &mut Vec<StructureDifference>,
) {
    let source_is_game_reference =
        matches!(&source.kind, ProtectedExpressionKind::GameReference { .. });
    let target_is_game_reference =
        matches!(&target.kind, ProtectedExpressionKind::GameReference { .. });
    if source_is_game_reference || target_is_game_reference {
        match (
            protected_numeric_value(&source.kind),
            protected_numeric_value(&target.kind),
        ) {
            (Some(source_value), Some(target_value)) if source_value != target_value => {
                push_expression_difference(
                    source,
                    target,
                    differences,
                    StructureDifferenceKind::ChangedGameReference,
                );
            }
            (Some(_), Some(_)) => {}
            _ => push_expression_difference(
                source,
                target,
                differences,
                StructureDifferenceKind::ChangedExpressionStructure,
            ),
        }
        return;
    }

    match (&source.kind, &target.kind) {
        (
            ProtectedExpressionKind::Integer {
                value: source_value,
            },
            ProtectedExpressionKind::Integer {
                value: target_value,
            },
        ) => {
            if source_value != target_value {
                push_expression_difference(
                    source,
                    target,
                    differences,
                    StructureDifferenceKind::ChangedExpressionStructure,
                );
            }
        }
        (
            ProtectedExpressionKind::String {
                nodes: source_nodes,
            },
            ProtectedExpressionKind::String {
                nodes: target_nodes,
            },
        ) => compare_node_lists(source_nodes, target_nodes, differences),
        (
            ProtectedExpressionKind::ProtectedString {
                spelling: source_spelling,
                game_reference: source_game_reference,
            },
            ProtectedExpressionKind::ProtectedString {
                spelling: target_spelling,
                game_reference: target_game_reference,
            },
        ) => compare_protected_strings(
            source,
            target,
            source_spelling,
            target_spelling,
            *source_game_reference || *target_game_reference,
            differences,
        ),
        (
            ProtectedExpressionKind::RuntimePlaceholder(source_placeholder),
            ProtectedExpressionKind::RuntimePlaceholder(target_placeholder),
        ) => {
            if source_placeholder != target_placeholder {
                push_expression_difference(
                    source,
                    target,
                    differences,
                    StructureDifferenceKind::ChangedRuntimeExpression,
                );
            }
        }
        (
            ProtectedExpressionKind::RuntimeParameter { .. },
            ProtectedExpressionKind::RuntimeParameter { .. },
        ) => compare_runtime_parameters(source, target, differences),
        (
            ProtectedExpressionKind::Comparison { .. },
            ProtectedExpressionKind::Comparison { .. },
        ) => compare_comparisons(source, target, differences),
        (
            ProtectedExpressionKind::Opaque {
                spelling: source_spelling,
            },
            ProtectedExpressionKind::Opaque {
                spelling: target_spelling,
            },
        ) => {
            if source_spelling != target_spelling {
                push_expression_difference(
                    source,
                    target,
                    differences,
                    StructureDifferenceKind::ChangedOpaqueConstruct,
                );
            }
        }
        (
            ProtectedExpressionKind::Malformed {
                spelling: source_spelling,
            },
            ProtectedExpressionKind::Malformed {
                spelling: target_spelling,
            },
        ) => {
            if source_spelling != target_spelling {
                push_expression_difference(
                    source,
                    target,
                    differences,
                    StructureDifferenceKind::ChangedExpressionStructure,
                );
            }
        }
        (
            ProtectedExpressionKind::RuntimePlaceholder(_)
            | ProtectedExpressionKind::RuntimeParameter { .. },
            ProtectedExpressionKind::RuntimePlaceholder(_)
            | ProtectedExpressionKind::RuntimeParameter { .. },
        ) => push_expression_difference(
            source,
            target,
            differences,
            StructureDifferenceKind::ChangedRuntimeExpression,
        ),
        _ => push_expression_difference(
            source,
            target,
            differences,
            StructureDifferenceKind::ChangedExpressionStructure,
        ),
    }
}

fn protected_numeric_value(kind: &ProtectedExpressionKind) -> Option<u32> {
    match kind {
        ProtectedExpressionKind::Integer { value }
        | ProtectedExpressionKind::GameReference { value } => Some(*value),
        _ => None,
    }
}

fn compare_protected_strings(
    source: &ProtectedExpression,
    target: &ProtectedExpression,
    source_spelling: &str,
    target_spelling: &str,
    game_reference: bool,
    differences: &mut Vec<StructureDifference>,
) {
    if source_spelling != target_spelling {
        push_expression_difference(
            source,
            target,
            differences,
            if game_reference {
                StructureDifferenceKind::ChangedGameReference
            } else {
                StructureDifferenceKind::ChangedExpressionStructure
            },
        );
    }
}

fn compare_runtime_parameters(
    source: &ProtectedExpression,
    target: &ProtectedExpression,
    differences: &mut Vec<StructureDifference>,
) {
    let (
        ProtectedExpressionKind::RuntimeParameter {
            operator: source_operator,
            operand: source_operand,
        },
        ProtectedExpressionKind::RuntimeParameter {
            operator: target_operator,
            operand: target_operand,
        },
    ) = (&source.kind, &target.kind)
    else {
        return;
    };
    if source_operator != target_operator {
        push_expression_difference(
            source,
            target,
            differences,
            StructureDifferenceKind::ChangedRuntimeExpression,
        );
        return;
    }

    let mut operand_differences = Vec::new();
    compare_protected_expressions(source_operand, target_operand, &mut operand_differences);
    if !operand_differences.is_empty() {
        push_expression_difference(
            source,
            target,
            differences,
            StructureDifferenceKind::ChangedRuntimeExpression,
        );
    }
}

fn compare_comparisons(
    source: &ProtectedExpression,
    target: &ProtectedExpression,
    differences: &mut Vec<StructureDifference>,
) {
    let (
        ProtectedExpressionKind::Comparison {
            operator: source_operator,
            left: source_left,
            right: source_right,
        },
        ProtectedExpressionKind::Comparison {
            operator: target_operator,
            left: target_left,
            right: target_right,
        },
    ) = (&source.kind, &target.kind)
    else {
        return;
    };
    if source_operator != target_operator {
        push_expression_difference(
            source,
            target,
            differences,
            StructureDifferenceKind::ChangedExpressionStructure,
        );
        return;
    }
    compare_protected_expressions(source_left, target_left, differences);
    compare_protected_expressions(source_right, target_right, differences);
}

fn push_expression_difference(
    source: &ProtectedExpression,
    target: &ProtectedExpression,
    differences: &mut Vec<StructureDifference>,
    kind: StructureDifferenceKind,
) {
    differences.push(StructureDifference {
        source_span: Some(source.span),
        target_span: Some(target.span),
        kind,
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProtectedNodeKey {
    TextSlot,
    Macro(KnownMacro),
    Opaque,
    Malformed,
}

fn protected_node_key(node: &ProtectedNode) -> ProtectedNodeKey {
    match node.kind {
        ProtectedNodeKind::TextSlot => ProtectedNodeKey::TextSlot,
        ProtectedNodeKind::Macro { name, .. } => ProtectedNodeKey::Macro(name),
        ProtectedNodeKind::Opaque { .. } => ProtectedNodeKey::Opaque,
        ProtectedNodeKind::Malformed { .. } => ProtectedNodeKey::Malformed,
    }
}

fn contains_opaque_protected_macro(nodes: &[ProtectedNode]) -> bool {
    nodes.iter().any(|node| match &node.kind {
        ProtectedNodeKind::Macro {
            family: SemanticFamily::OpaqueProtected,
            ..
        } => true,
        ProtectedNodeKind::Macro { arguments, .. } => {
            arguments.iter().any(contains_opaque_protected_expression)
        }
        ProtectedNodeKind::TextSlot
        | ProtectedNodeKind::Opaque { .. }
        | ProtectedNodeKind::Malformed { .. } => false,
    })
}

fn contains_opaque_protected_expression(expression: &ProtectedExpression) -> bool {
    match &expression.kind {
        ProtectedExpressionKind::String { nodes } => contains_opaque_protected_macro(nodes),
        ProtectedExpressionKind::RuntimeParameter { operand, .. } => {
            contains_opaque_protected_expression(operand)
        }
        ProtectedExpressionKind::Comparison { left, right, .. } => {
            contains_opaque_protected_expression(left)
                || contains_opaque_protected_expression(right)
        }
        ProtectedExpressionKind::Integer { .. }
        | ProtectedExpressionKind::GameReference { .. }
        | ProtectedExpressionKind::ProtectedString { .. }
        | ProtectedExpressionKind::RuntimePlaceholder(_)
        | ProtectedExpressionKind::Opaque { .. }
        | ProtectedExpressionKind::Malformed { .. } => false,
    }
}

fn same_multiset(source: &[ProtectedNode], target: &[ProtectedNode]) -> bool {
    let mut target_keys: Vec<_> = target.iter().map(protected_node_key).collect();
    source.iter().map(protected_node_key).all(|key| {
        target_keys
            .iter()
            .position(|candidate| *candidate == key)
            .map(|position| {
                target_keys.remove(position);
            })
            .is_some()
    }) && target_keys.is_empty()
}
