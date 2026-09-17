//! Lossless syntax support for Lumina's encodeable macro-string format.
//!
//! This crate deliberately stops at a syntax layer. It does not evaluate game
//! expressions, encode `SeString` bytes, or decide what a macro means to a
//! translator.

#![forbid(unsafe_code)]

mod semantic;

pub use semantic::{
    OpaqueIdentity, OpaqueSemanticKind, ProtectedExpression, ProtectedExpressionKind,
    ProtectedNode, ProtectedNodeKind, ProtectedStructure, SemanticAnalysis, SemanticExpression,
    SemanticExpressionKind, SemanticFamily, SemanticMacro, SemanticNode, SemanticNodeKind,
    SemanticOpaque, SemanticValidation, SemanticValidity, StructureComparison,
    StructureCompatibility, StructureDifference, StructureDifferenceKind, TextRange, TextRangeKind,
    analyze, compare_analyses, compare_macro_strings, compare_structures, validate,
};

/// Maximum parser call depth used while inspecting nested macros and
/// expressions.
pub const MAX_NESTING_DEPTH: usize = 128;

/// A half-open byte range into the original macro string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    start: usize,
    end: usize,
}

impl Span {
    /// Creates a span from byte offsets.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Returns the first byte offset.
    #[must_use]
    pub const fn start(self) -> usize {
        self.start
    }

    /// Returns the exclusive end byte offset.
    #[must_use]
    pub const fn end(self) -> usize {
        self.end
    }

    /// Returns the span length in bytes.
    #[must_use]
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns whether the span is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// Aggregate safety status for a parsed macro string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Safety {
    /// Every node is understood by this syntax layer.
    Understood,
    /// At least one node is opaque but can be preserved exactly.
    Opaque,
    /// At least one node is malformed or could not be parsed safely.
    Malformed,
}

/// The kind of a parse diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    /// The input ended before a required delimiter or escape target.
    UnexpectedEof,
    /// A delimiter was not valid at the current grammar position.
    InvalidDelimiter,
    /// A backslash did not introduce a character.
    InvalidEscape,
    /// A Lumina fallback payload or expression could not be decoded.
    InvalidFallback,
    /// The configured parser nesting bound was reached.
    NestingLimit,
}

/// A structured parser diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    /// The input span associated with the diagnostic.
    pub span: Span,
    /// The diagnostic classification.
    pub kind: DiagnosticKind,
    /// A short human-readable description.
    pub message: String,
}

/// The root syntax document returned by [`parse`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroString {
    source: String,
    nodes: Vec<SyntaxNode>,
    diagnostics: Vec<Diagnostic>,
    safety: Safety,
}

impl MacroString {
    /// Returns the original source representation.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the root nodes in source order.
    #[must_use]
    pub fn nodes(&self) -> &[SyntaxNode] {
        &self.nodes
    }

    /// Returns parser diagnostics in source order.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Returns the aggregate safety classification.
    #[must_use]
    pub const fn safety(&self) -> Safety {
        self.safety
    }

    /// Returns the original text covered by `span`, if it is a valid UTF-8
    /// boundary inside this document.
    #[must_use]
    pub fn slice(&self, span: Span) -> Option<&str> {
        self.source.get(span.start..span.end)
    }

    /// Serializes the syntax document without changing its source spelling.
    ///
    /// Mutation-aware structural serialization is future work for this slice;
    /// today this deliberately returns the owned source buffer so edits cannot
    /// silently rewrite syntax that has not been modeled yet.
    #[must_use]
    pub fn serialize(&self) -> String {
        self.source.clone()
    }

    /// Returns whether parsing produced no malformed constructs.
    #[must_use]
    pub const fn is_well_formed(&self) -> bool {
        !matches!(self.safety, Safety::Malformed)
    }

    /// Derives semantic classifications, text ranges, and protected
    /// structure from this syntax document.
    #[must_use]
    pub fn semantic_analysis(&self) -> SemanticAnalysis {
        semantic::analyze(self)
    }

    /// Validates this syntax document without executing its expressions.
    #[must_use]
    pub fn semantic_validation(&self) -> SemanticValidation {
        semantic::validate(self)
    }
}

/// Parses Lumina's encodeable macro-string representation.
#[must_use]
pub fn parse(source: &str) -> MacroString {
    let mut parser = Parser::new(source);
    let nodes = parser.parse_document();
    let safety = if parser.diagnostics.is_empty() {
        if parser.opaque_found {
            Safety::Opaque
        } else {
            Safety::Understood
        }
    } else {
        Safety::Malformed
    };

    MacroString {
        source: source.to_owned(),
        nodes,
        diagnostics: parser.diagnostics,
        safety,
    }
}

/// Alias with an explicit name for call sites that work with multiple string
/// formats.
#[must_use]
pub fn parse_macro_string(source: &str) -> MacroString {
    parse(source)
}

/// One root or string-expression syntax node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxNode {
    /// The exact source range occupied by this node.
    pub span: Span,
    /// The structural kind of this node.
    pub kind: SyntaxKind,
}

/// A syntax node kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyntaxKind {
    /// Ordinary text, whose source spelling is available through its span.
    Text,
    /// A backslash escape and its decoded character.
    Escape { character: char },
    /// A known macro payload and its ordered arguments.
    Macro(MacroNode),
    /// A payload fallback that Lumina can print but Aeria does not interpret.
    Opaque(OpaquePayload),
    /// A recovery node for malformed input.
    Malformed,
}

/// A macro whose name is known to Lumina 7.7.0.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroNode {
    /// The native macro name.
    pub name: KnownMacro,
    /// Ordered macro arguments.
    pub arguments: Vec<Expression>,
}

/// Macro names emitted by Lumina 7.7.0's `ToMacroString()` implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnownMacro {
    SetResetTime,
    SetTime,
    If,
    Switch,
    PcName,
    IfPcGender,
    IfPcName,
    Josa,
    Josaro,
    IfSelf,
    NewLine,
    Wait,
    Icon,
    Color,
    EdgeColor,
    ShadowColor,
    SoftHyphen,
    Key,
    Scale,
    Bold,
    Italic,
    Edge,
    Shadow,
    NonBreakingSpace,
    Icon2,
    Hyphen,
    Num,
    Hex,
    Kilo,
    Byte,
    Sec,
    Time,
    Float,
    Link,
    Sheet,
    String,
    Caps,
    Head,
    Split,
    HeadAll,
    Fixed,
    Lower,
    JaNoun,
    EnNoun,
    DeNoun,
    FrNoun,
    ChNoun,
    LowerHead,
    SheetSub,
    SwitchPlatform,
    ColorType,
    EdgeColorType,
    Ruby,
    Digit,
    Ordinal,
    Sound,
    LevelPos,
}

impl KnownMacro {
    /// Returns the exact native spelling emitted by Lumina.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SetResetTime => "setresettime",
            Self::SetTime => "settime",
            Self::If => "if",
            Self::Switch => "switch",
            Self::PcName => "pcname",
            Self::IfPcGender => "ifpcgender",
            Self::IfPcName => "ifpcname",
            Self::Josa => "josa",
            Self::Josaro => "josaro",
            Self::IfSelf => "ifself",
            Self::NewLine => "br",
            Self::Wait => "wait",
            Self::Icon => "icon",
            Self::Color => "color",
            Self::EdgeColor => "edgecolor",
            Self::ShadowColor => "shadowcolor",
            Self::SoftHyphen => "-",
            Self::Key => "key",
            Self::Scale => "scale",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Edge => "edge",
            Self::Shadow => "shadow",
            Self::NonBreakingSpace => "nbsp",
            Self::Icon2 => "icon2",
            Self::Hyphen => "--",
            Self::Num => "num",
            Self::Hex => "hex",
            Self::Kilo => "kilo",
            Self::Byte => "byte",
            Self::Sec => "sec",
            Self::Time => "time",
            Self::Float => "float",
            Self::Link => "link",
            Self::Sheet => "sheet",
            Self::String => "string",
            Self::Caps => "caps",
            Self::Head => "head",
            Self::Split => "split",
            Self::HeadAll => "headall",
            Self::Fixed => "fixed",
            Self::Lower => "lower",
            Self::JaNoun => "janoun",
            Self::EnNoun => "ennoun",
            Self::DeNoun => "denoun",
            Self::FrNoun => "frnoun",
            Self::ChNoun => "chnoun",
            Self::LowerHead => "lowerhead",
            Self::SheetSub => "sheetsub",
            Self::SwitchPlatform => "switchplatform",
            Self::ColorType => "colortype",
            Self::EdgeColorType => "edgecolortype",
            Self::Ruby => "ruby",
            Self::Digit => "digit",
            Self::Ordinal => "ordinal",
            Self::Sound => "sound",
            Self::LevelPos => "levelpos",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "setresettime" => Self::SetResetTime,
            "settime" => Self::SetTime,
            "if" => Self::If,
            "switch" => Self::Switch,
            "pcname" => Self::PcName,
            "ifpcgender" => Self::IfPcGender,
            "ifpcname" => Self::IfPcName,
            "josa" => Self::Josa,
            "josaro" => Self::Josaro,
            "ifself" => Self::IfSelf,
            "br" => Self::NewLine,
            "wait" => Self::Wait,
            "icon" => Self::Icon,
            "color" => Self::Color,
            "edgecolor" => Self::EdgeColor,
            "shadowcolor" => Self::ShadowColor,
            "-" => Self::SoftHyphen,
            "key" => Self::Key,
            "scale" => Self::Scale,
            "bold" => Self::Bold,
            "italic" => Self::Italic,
            "edge" => Self::Edge,
            "shadow" => Self::Shadow,
            "nbsp" => Self::NonBreakingSpace,
            "icon2" => Self::Icon2,
            "--" => Self::Hyphen,
            "num" => Self::Num,
            "hex" => Self::Hex,
            "kilo" => Self::Kilo,
            "byte" => Self::Byte,
            "sec" => Self::Sec,
            "time" => Self::Time,
            "float" => Self::Float,
            "link" => Self::Link,
            "sheet" => Self::Sheet,
            "string" => Self::String,
            "caps" => Self::Caps,
            "head" => Self::Head,
            "split" => Self::Split,
            "headall" => Self::HeadAll,
            "fixed" => Self::Fixed,
            "lower" => Self::Lower,
            "janoun" => Self::JaNoun,
            "ennoun" => Self::EnNoun,
            "denoun" => Self::DeNoun,
            "frnoun" => Self::FrNoun,
            "chnoun" => Self::ChNoun,
            "lowerhead" => Self::LowerHead,
            "sheetsub" => Self::SheetSub,
            "switchplatform" => Self::SwitchPlatform,
            "colortype" => Self::ColorType,
            "edgecolortype" => Self::EdgeColorType,
            "ruby" => Self::Ruby,
            "digit" => Self::Digit,
            "ordinal" => Self::Ordinal,
            "sound" => Self::Sound,
            "levelpos" => Self::LevelPos,
            _ => return None,
        })
    }
}

/// An opaque payload fallback emitted by Lumina for unsupported/raw payloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpaquePayload {
    /// A syntactically valid named macro whose semantics are not known to
    /// this Lumina contract.
    NamedMacro {
        /// The exact macro name spelling between `<` and its argument list.
        name: String,
        /// Ordered arguments preserved for structural inspection.
        arguments: Vec<Expression>,
    },
    /// An unsupported macro code with optional expression arguments.
    Macro {
        /// The raw macro code byte.
        code: u8,
        /// Ordered arguments, if the payload body was expression-shaped.
        arguments: Vec<Expression>,
    },
    /// Raw bytes printed by Lumina's invalid-payload fallback.
    Raw { bytes: Vec<u8> },
}

/// One ordered macro argument expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Expression {
    /// The exact source range occupied by this expression.
    pub span: Span,
    /// The structural expression kind.
    pub kind: ExpressionKind,
}

/// Expression forms in Lumina's macro-string representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpressionKind {
    /// An unsigned integer expression. The original spelling is available via
    /// the expression span.
    UnsignedInteger { value: u32 },
    /// A nested SeString/string expression.
    String { parts: Vec<SyntaxNode> },
    /// A native nullary expression.
    Placeholder(PlaceholderExpression),
    /// A native unary expression with one ordered operand.
    Unary {
        operator: UnaryExpression,
        operand: Box<Expression>,
    },
    /// A native binary comparison expression.
    Binary {
        operator: ComparisonOperator,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    /// An unsupported but losslessly representable expression fallback.
    Opaque { bytes: Vec<u8> },
    /// A malformed expression recovered for inspection.
    Malformed { recovered: Vec<SyntaxNode> },
}

/// Native nullary expression names emitted by Lumina.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaceholderExpression {
    Millisecond,
    Second,
    Minute,
    Hour,
    Day,
    Weekday,
    Month,
    Year,
    StackColor,
}

impl PlaceholderExpression {
    /// Returns the native spelling emitted by Lumina.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Millisecond => "t_msec",
            Self::Second => "t_sec",
            Self::Minute => "t_min",
            Self::Hour => "t_hour",
            Self::Day => "t_day",
            Self::Weekday => "t_wday",
            Self::Month => "t_mon",
            Self::Year => "t_year",
            Self::StackColor => "stackcolor",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "t_msec" => Self::Millisecond,
            "t_sec" => Self::Second,
            "t_min" => Self::Minute,
            "t_hour" => Self::Hour,
            "t_day" => Self::Day,
            "t_wday" => Self::Weekday,
            "t_mon" => Self::Month,
            "t_year" => Self::Year,
            "stackcolor" => Self::StackColor,
            _ => return None,
        })
    }
}

/// Native unary expression names emitted by Lumina.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryExpression {
    LocalNumber,
    GlobalNumber,
    LocalString,
    GlobalString,
}

impl UnaryExpression {
    /// Returns the native spelling emitted by Lumina.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalNumber => "lnum",
            Self::GlobalNumber => "gnum",
            Self::LocalString => "lstr",
            Self::GlobalString => "gstr",
        }
    }

    fn from_prefix(value: &str) -> Option<(Self, &'static str)> {
        [
            (Self::LocalNumber, "lnum"),
            (Self::GlobalNumber, "gnum"),
            (Self::LocalString, "lstr"),
            (Self::GlobalString, "gstr"),
        ]
        .into_iter()
        .find(|(_, prefix)| value.starts_with(prefix))
    }
}

/// Operators emitted for binary comparison expressions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComparisonOperator {
    GreaterThanOrEqual,
    GreaterThan,
    LessThanOrEqual,
    LessThan,
    Equal,
    NotEqual,
}

/// Internal parser. It only borrows the source while constructing spans and
/// owned node metadata; the returned document owns its source copy.
struct Parser<'a> {
    source: &'a str,
    diagnostics: Vec<Diagnostic>,
    opaque_found: bool,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            diagnostics: Vec::new(),
            opaque_found: false,
        }
    }

    fn parse_document(&mut self) -> Vec<SyntaxNode> {
        let mut nodes = Vec::new();
        let mut position = 0;
        while position < self.source.len() {
            let (character, width) = self
                .character_at(position)
                .expect("position is a char boundary");
            match character {
                '\\' => {
                    if let Some((escaped, escaped_width)) = self.character_at(position + width) {
                        nodes.push(SyntaxNode {
                            span: Span::new(position, position + width + escaped_width),
                            kind: SyntaxKind::Escape { character: escaped },
                        });
                        position += width + escaped_width;
                    } else {
                        self.diagnose(
                            Span::new(position, self.source.len()),
                            DiagnosticKind::InvalidEscape,
                            "escape is missing its character",
                        );
                        nodes.push(SyntaxNode {
                            span: Span::new(position, self.source.len()),
                            kind: SyntaxKind::Malformed,
                        });
                        position = self.source.len();
                    }
                }
                '<' => {
                    let (node, next) = self.parse_syntax_node(position, 0);
                    nodes.push(node);
                    position = next.max(position + width);
                }
                _ => {
                    let start = position;
                    position += width;
                    while let Some((next, next_width)) = self.character_at(position) {
                        if matches!(next, '\\' | '<') {
                            break;
                        }
                        position += next_width;
                    }
                    nodes.push(SyntaxNode {
                        span: Span::new(start, position),
                        kind: SyntaxKind::Text,
                    });
                }
            }
        }
        nodes
    }

    fn parse_syntax_node(&mut self, start: usize, depth: usize) -> (SyntaxNode, usize) {
        if depth >= MAX_NESTING_DEPTH {
            self.diagnose(
                Span::new(start, self.source.len()),
                DiagnosticKind::NestingLimit,
                "maximum macro nesting depth exceeded",
            );
            return (
                SyntaxNode {
                    span: Span::new(start, self.source.len()),
                    kind: SyntaxKind::Malformed,
                },
                self.source.len(),
            );
        }

        let name_start = self.skip_whitespace(start + 1);
        if self.starts_with(name_start, "payload:") {
            return self.parse_opaque_payload(start, name_start + "payload:".len(), depth);
        }
        self.parse_macro(start, name_start, depth)
    }

    fn parse_macro(
        &mut self,
        start: usize,
        name_start: usize,
        depth: usize,
    ) -> (SyntaxNode, usize) {
        let name_end = self.scan_macro_name(name_start);
        let name = self.source.get(name_start..name_end).unwrap_or_default();
        if name.is_empty() {
            self.diagnose(
                Span::new(name_start, name_end),
                DiagnosticKind::InvalidDelimiter,
                "macro name is missing",
            );
            let end = self.recover_macro(start);
            return (Self::malformed_node(start, end), end);
        }

        let mut position = self.skip_whitespace(name_end);
        let arguments = if self.char_is(position, '>') {
            position += 1;
            Vec::new()
        } else if self.char_is(position, '(') {
            match self.parse_arguments(position, depth + 1) {
                Ok((arguments, after_arguments)) => {
                    position = self.skip_whitespace(after_arguments);
                    if !self.char_is(position, '>') {
                        self.diagnose(
                            self.diagnostic_span(position),
                            if position >= self.source.len() {
                                DiagnosticKind::UnexpectedEof
                            } else {
                                DiagnosticKind::InvalidDelimiter
                            },
                            "expected '>' after macro arguments",
                        );
                        let end = self.recover_macro(start);
                        return (Self::malformed_node(start, end), end);
                    }
                    position += 1;
                    arguments
                }
                Err(end) => return (Self::malformed_node(start, end), end),
            }
        } else {
            self.diagnose(
                self.diagnostic_span(position),
                if position >= self.source.len() {
                    DiagnosticKind::UnexpectedEof
                } else {
                    DiagnosticKind::InvalidDelimiter
                },
                "expected '(' or '>' after macro name",
            );
            let end = self.recover_macro(start);
            return (Self::malformed_node(start, end), end);
        };

        let kind = if let Some(name) = KnownMacro::from_str(name) {
            SyntaxKind::Macro(MacroNode { name, arguments })
        } else {
            self.opaque_found = true;
            SyntaxKind::Opaque(OpaquePayload::NamedMacro {
                name: name.to_owned(),
                arguments,
            })
        };

        (
            SyntaxNode {
                span: Span::new(start, position),
                kind,
            },
            position,
        )
    }

    fn parse_arguments(
        &mut self,
        open: usize,
        depth: usize,
    ) -> Result<(Vec<Expression>, usize), usize> {
        let mut position = open + 1;
        let mut first = true;
        let mut arguments = Vec::new();
        loop {
            if position >= self.source.len() {
                self.diagnose(
                    Span::new(position, position),
                    DiagnosticKind::UnexpectedEof,
                    "macro arguments are missing a closing ')'",
                );
                return Err(self.source.len());
            }
            if self.char_is(position, ')') {
                return Ok((arguments, position + 1));
            }
            if !first {
                if !self.char_is(position, ',') {
                    self.diagnose(
                        Span::new(position, position + self.character_width(position)),
                        DiagnosticKind::InvalidDelimiter,
                        "expected ',' or ')' between macro arguments",
                    );
                    return Err(self.recover_macro(open));
                }
                position += 1;
            }

            let expression = self.parse_expression(position, false, depth);
            position = expression.span.end();
            arguments.push(expression);
            first = false;
        }
    }

    fn parse_expression(&mut self, start: usize, binary_left: bool, depth: usize) -> Expression {
        if depth >= MAX_NESTING_DEPTH {
            self.diagnose(
                Span::new(start, self.source.len()),
                DiagnosticKind::NestingLimit,
                "maximum expression nesting depth exceeded",
            );
            return Expression {
                span: Span::new(start, self.source.len()),
                kind: ExpressionKind::Malformed {
                    recovered: Vec::new(),
                },
            };
        }
        if self.char_is(start, '[') {
            return self.parse_binary_expression(start, depth);
        }
        if self.starts_with(start, "<expr:") {
            return self.parse_opaque_expression(start);
        }
        self.parse_string_expression(start, binary_left, depth)
    }

    fn parse_binary_expression(&mut self, start: usize, depth: usize) -> Expression {
        let left_start = start + 1;
        let left = self.parse_expression(left_start, true, depth + 1);
        let position = left.span.end();
        let Some((operator, after_operator)) = self.parse_comparison_operator(position) else {
            let end = self.recover_expression(start);
            return Expression {
                span: Span::new(start, end),
                kind: ExpressionKind::Malformed {
                    recovered: Vec::new(),
                },
            };
        };
        let right = self.parse_expression(after_operator, false, depth + 1);
        let mut end = right.span.end();
        let closed = self.char_is(end, ']');
        if closed {
            end += 1;
        } else {
            self.diagnose(
                self.diagnostic_span(end),
                if end >= self.source.len() {
                    DiagnosticKind::UnexpectedEof
                } else {
                    DiagnosticKind::InvalidDelimiter
                },
                "expected ']' after comparison expression",
            );
        }

        if closed && !is_malformed_expression(&left) && !is_malformed_expression(&right) {
            Expression {
                span: Span::new(start, end),
                kind: ExpressionKind::Binary {
                    operator,
                    left: Box::new(left),
                    right: Box::new(right),
                },
            }
        } else {
            Expression {
                span: Span::new(start, end),
                kind: ExpressionKind::Malformed {
                    recovered: Vec::new(),
                },
            }
        }
    }

    fn parse_string_expression(
        &mut self,
        start: usize,
        binary_left: bool,
        depth: usize,
    ) -> Expression {
        let (position, parts, decoded, plain, malformed) =
            self.scan_string_expression(start, binary_left, depth);

        let span = Span::new(start, position);
        if malformed {
            return Expression {
                span,
                kind: ExpressionKind::Malformed { recovered: parts },
            };
        }
        if !plain {
            return Expression {
                span,
                kind: ExpressionKind::String { parts },
            };
        }

        if let Some(placeholder) = PlaceholderExpression::from_str(&decoded) {
            return Expression {
                span,
                kind: ExpressionKind::Placeholder(placeholder),
            };
        }
        if let Some((operator, prefix)) = UnaryExpression::from_prefix(&decoded) {
            let operand_text = &decoded[prefix.len()..];
            if let Some(value) = parse_lumina_integer(operand_text) {
                let operand_start = start + prefix.len();
                return Expression {
                    span,
                    kind: ExpressionKind::Unary {
                        operator,
                        operand: Box::new(Expression {
                            span: Span::new(operand_start, position),
                            kind: ExpressionKind::UnsignedInteger { value },
                        }),
                    },
                };
            }
        }
        if let Some(value) = parse_lumina_integer(&decoded) {
            return Expression {
                span,
                kind: ExpressionKind::UnsignedInteger { value },
            };
        }

        Expression {
            span,
            kind: ExpressionKind::String { parts },
        }
    }

    fn scan_string_expression(
        &mut self,
        start: usize,
        binary_left: bool,
        depth: usize,
    ) -> (usize, Vec<SyntaxNode>, String, bool, bool) {
        let mut position = start;
        let mut parts = Vec::new();
        let mut text_start = None;
        let mut decoded = String::new();
        let mut plain = true;
        let mut malformed = false;

        while let Some((character, width)) = self.character_at(position) {
            if binary_left && matches!(character, '=' | '!' | '<' | '>') {
                break;
            }
            match character {
                '\\' => {
                    flush_text(&mut parts, text_start.take(), position);
                    let escaped_start = position;
                    position += width;
                    let Some((escaped, escaped_width)) = self.character_at(position) else {
                        self.diagnose(
                            Span::new(escaped_start, self.source.len()),
                            DiagnosticKind::InvalidEscape,
                            "escape is missing its character",
                        );
                        parts.push(SyntaxNode {
                            span: Span::new(escaped_start, self.source.len()),
                            kind: SyntaxKind::Malformed,
                        });
                        malformed = true;
                        position = self.source.len();
                        break;
                    };
                    parts.push(SyntaxNode {
                        span: Span::new(escaped_start, position + escaped_width),
                        kind: SyntaxKind::Escape { character: escaped },
                    });
                    if plain {
                        decoded.push(escaped);
                    }
                    position += escaped_width;
                }
                '<' if !binary_left => {
                    flush_text(&mut parts, text_start.take(), position);
                    let (node, next) = self.parse_syntax_node(position, depth + 1);
                    malformed |= matches!(node.kind, SyntaxKind::Malformed);
                    parts.push(node);
                    plain = false;
                    position = next.max(position + width);
                }
                '[' | '>' | ')' | ',' | '(' | ']' => {
                    break;
                }
                _ => {
                    text_start.get_or_insert(position);
                    if plain {
                        decoded.push(character);
                    }
                    position += width;
                }
            }
        }
        flush_text(&mut parts, text_start, position);
        (position, parts, decoded, plain, malformed)
    }

    fn parse_opaque_expression(&mut self, start: usize) -> Expression {
        let body_start = start + "<expr:".len();
        let Some(close) = self.find_char(body_start, '>') else {
            self.diagnose(
                Span::new(start, self.source.len()),
                DiagnosticKind::UnexpectedEof,
                "opaque expression is missing '>'",
            );
            return Expression {
                span: Span::new(start, self.source.len()),
                kind: ExpressionKind::Malformed {
                    recovered: Vec::new(),
                },
            };
        };
        let body = self
            .source
            .get(body_start..close)
            .unwrap_or_default()
            .trim();
        let Some(bytes) = parse_expression_fallback(body) else {
            self.diagnose(
                Span::new(body_start, close),
                DiagnosticKind::InvalidFallback,
                "opaque expression fallback is not valid Lumina syntax",
            );
            return Expression {
                span: Span::new(start, close + 1),
                kind: ExpressionKind::Malformed {
                    recovered: Vec::new(),
                },
            };
        };
        self.opaque_found = true;
        Expression {
            span: Span::new(start, close + 1),
            kind: ExpressionKind::Opaque { bytes },
        }
    }

    fn parse_opaque_payload(
        &mut self,
        start: usize,
        body_start: usize,
        depth: usize,
    ) -> (SyntaxNode, usize) {
        let Some((first, first_width)) = self.character_at(body_start) else {
            self.diagnose(
                Span::new(body_start, body_start),
                DiagnosticKind::UnexpectedEof,
                "opaque payload is missing its body",
            );
            return (
                Self::malformed_node(start, self.source.len()),
                self.source.len(),
            );
        };

        if first.is_whitespace() {
            let Some(close) = self.find_char(body_start, '>') else {
                self.diagnose(
                    Span::new(start, self.source.len()),
                    DiagnosticKind::UnexpectedEof,
                    "raw payload fallback is missing '>'",
                );
                return (
                    Self::malformed_node(start, self.source.len()),
                    self.source.len(),
                );
            };
            let body = self
                .source
                .get(body_start..close)
                .unwrap_or_default()
                .trim();
            let Some(bytes) = parse_raw_payload_fallback(body) else {
                self.diagnose(
                    Span::new(body_start, close),
                    DiagnosticKind::InvalidFallback,
                    "raw payload fallback is not valid Lumina syntax",
                );
                return (Self::malformed_node(start, close + 1), close + 1);
            };
            self.opaque_found = true;
            return (
                SyntaxNode {
                    span: Span::new(start, close + 1),
                    kind: SyntaxKind::Opaque(OpaquePayload::Raw { bytes }),
                },
                close + 1,
            );
        }

        let Some(code) = self.parse_hex_pair(body_start) else {
            self.diagnose(
                Span::new(body_start, body_start + first_width),
                DiagnosticKind::InvalidFallback,
                "opaque macro payload is missing a two-digit code",
            );
            let end = self.recover_macro(start);
            return (Self::malformed_node(start, end), end);
        };
        let mut position = self.skip_whitespace(body_start + 2);
        let arguments = if self.char_is(position, '>') {
            position += 1;
            Vec::new()
        } else if self.char_is(position, '(') {
            match self.parse_arguments(position, depth + 1) {
                Ok((arguments, after_arguments)) => {
                    position = self.skip_whitespace(after_arguments);
                    if !self.char_is(position, '>') {
                        self.diagnose(
                            self.diagnostic_span(position),
                            DiagnosticKind::InvalidDelimiter,
                            "expected '>' after opaque payload arguments",
                        );
                        let end = self.recover_macro(start);
                        return (Self::malformed_node(start, end), end);
                    }
                    position += 1;
                    arguments
                }
                Err(end) => return (Self::malformed_node(start, end), end),
            }
        } else {
            self.diagnose(
                self.diagnostic_span(position),
                DiagnosticKind::InvalidFallback,
                "expected '(' or '>' after opaque macro code",
            );
            let end = self.recover_macro(start);
            return (Self::malformed_node(start, end), end);
        };

        self.opaque_found = true;
        (
            SyntaxNode {
                span: Span::new(start, position),
                kind: SyntaxKind::Opaque(OpaquePayload::Macro { code, arguments }),
            },
            position,
        )
    }

    fn parse_comparison_operator(
        &mut self,
        position: usize,
    ) -> Option<(ComparisonOperator, usize)> {
        for (spelling, operator) in [
            (">=", ComparisonOperator::GreaterThanOrEqual),
            ("<=", ComparisonOperator::LessThanOrEqual),
            ("==", ComparisonOperator::Equal),
            ("!=", ComparisonOperator::NotEqual),
            (">", ComparisonOperator::GreaterThan),
            ("<", ComparisonOperator::LessThan),
        ] {
            if self.starts_with(position, spelling) {
                return Some((operator, position + spelling.len()));
            }
        }
        self.diagnose(
            Span::new(
                position,
                (position + self.character_width(position)).min(self.source.len()),
            ),
            DiagnosticKind::InvalidDelimiter,
            "expected a comparison operator",
        );
        None
    }

    fn parse_hex_pair(&self, position: usize) -> Option<u8> {
        let bytes = self.source.as_bytes();
        let high = *bytes.get(position)?;
        let low = *bytes.get(position + 1)?;
        Some(hex_digit(high)? * 16 + hex_digit(low)?)
    }

    fn scan_macro_name(&self, start: usize) -> usize {
        let mut position = start;
        while let Some((character, width)) = self.character_at(position) {
            if character.is_whitespace()
                || matches!(character, '\\' | '<' | '>' | '(' | ')' | '[' | ']' | ',')
            {
                break;
            }
            position += width;
        }
        position
    }

    fn skip_whitespace(&self, mut position: usize) -> usize {
        while let Some((character, width)) = self.character_at(position) {
            if !character.is_whitespace() {
                break;
            }
            position += width;
        }
        position
    }

    fn recover_macro(&self, start: usize) -> usize {
        let mut position = start + self.character_width(start);
        while let Some((character, width)) = self.character_at(position) {
            match character {
                '\\' => {
                    position += width;
                    if let Some((_, escaped_width)) = self.character_at(position) {
                        position += escaped_width;
                    }
                }
                '>' => return position + width,
                '<' => return position,
                _ => position += width,
            }
        }
        self.source.len()
    }

    fn recover_expression(&self, start: usize) -> usize {
        let mut position = start;
        while let Some((character, width)) = self.character_at(position) {
            if matches!(character, ']' | ')' | '>' | ',') {
                return position;
            }
            position += width;
        }
        self.source.len()
    }

    fn find_char(&self, start: usize, target: char) -> Option<usize> {
        let relative = self.source.get(start..)?.find(target)?;
        Some(start + relative)
    }

    fn starts_with(&self, position: usize, expected: &str) -> bool {
        self.source
            .get(position..)
            .is_some_and(|remaining| remaining.starts_with(expected))
    }

    fn char_is(&self, position: usize, expected: char) -> bool {
        self.character_at(position)
            .is_some_and(|(actual, _)| actual == expected)
    }

    fn character_at(&self, position: usize) -> Option<(char, usize)> {
        self.source
            .get(position..)?
            .chars()
            .next()
            .map(|character| (character, character.len_utf8()))
    }

    fn character_width(&self, position: usize) -> usize {
        self.character_at(position).map_or(0, |(_, width)| width)
    }

    fn diagnostic_span(&self, position: usize) -> Span {
        Span::new(
            position,
            (position + self.character_width(position)).min(self.source.len()),
        )
    }

    fn malformed_node(start: usize, end: usize) -> SyntaxNode {
        SyntaxNode {
            span: Span::new(start, end.max(start)),
            kind: SyntaxKind::Malformed,
        }
    }

    fn diagnose(&mut self, span: Span, kind: DiagnosticKind, message: &str) {
        self.diagnostics.push(Diagnostic {
            span,
            kind,
            message: message.to_owned(),
        });
    }
}

fn flush_text(parts: &mut Vec<SyntaxNode>, start: Option<usize>, end: usize) {
    if let Some(start) = start
        && start < end
    {
        parts.push(SyntaxNode {
            span: Span::new(start, end),
            kind: SyntaxKind::Text,
        });
    }
}

fn is_malformed_expression(expression: &Expression) -> bool {
    matches!(expression.kind, ExpressionKind::Malformed { .. })
}

fn parse_expression_fallback(body: &str) -> Option<Vec<u8>> {
    if body == "invalid empty" {
        return Some(Vec::new());
    }
    if let Some(rest) = body.strip_prefix("0x") {
        let byte = rest.get(..2)?;
        let suffix = rest.get(2..)?;
        if suffix != " is unsupported" {
            return None;
        }
        return Some(vec![parse_hex_byte(byte)?]);
    }
    parse_hex_tokens(body)
}

fn parse_raw_payload_fallback(body: &str) -> Option<Vec<u8>> {
    if body == "invalid empty" {
        return Some(Vec::new());
    }
    parse_hex_tokens(body)
}

fn parse_hex_tokens(body: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    for token in body.split_whitespace() {
        if token.len() != 2 {
            return None;
        }
        bytes.push(parse_hex_byte(token)?);
    }
    (!bytes.is_empty()).then_some(bytes)
}

fn parse_hex_byte(value: &str) -> Option<u8> {
    let bytes = value.as_bytes();
    Some(hex_digit(*bytes.first()?)? * 16 + hex_digit(*bytes.get(1)?)?)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

/// Matches Lumina's `TryParseInt` spelling rules, including digit separators,
/// radix prefixes, signs, and wrapping conversion through `int`.
fn parse_lumina_integer(value: &str) -> Option<u32> {
    let mut data = value.as_bytes();
    if data.is_empty() {
        return None;
    }

    let mut negative = false;
    while let Some(first) = data.first().copied() {
        match first {
            b'-' => negative = !negative,
            b'+' => {}
            b'0'..=b'9' => break,
            _ => return None,
        }
        data = &data[1..];
    }

    let mut radix = 10u32;
    if data.len() > 2
        && data[0] == b'0'
        && data[1] != b'_'
        && data[1] != b'\''
        && !data[1].is_ascii_digit()
    {
        radix = match data[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            b'd' | b'D' => 10,
            _ => return None,
        };
        data = &data[2..];
    }
    if data.is_empty() {
        return None;
    }

    let mut number = 0u32;
    for digit in data {
        match *digit {
            b'_' | b'\'' => {}
            value => {
                let value = if value.is_ascii_digit() {
                    value - b'0'
                } else if value.is_ascii_uppercase() {
                    value - b'A' + 10
                } else if value.is_ascii_lowercase() {
                    value - b'a' + 10
                } else {
                    return None;
                };
                if u32::from(value) >= radix {
                    return None;
                }
                number = number.wrapping_mul(radix).wrapping_add(u32::from(value));
            }
        }
    }

    Some(if negative {
        0u32.wrapping_sub(number)
    } else {
        number
    })
}
