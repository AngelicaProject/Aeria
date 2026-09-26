//! Macro text: the written form of game strings.
//!
//! [`print`] writes the string model as macro text and [`parse`] reads macro
//! text into a syntax tree with source spans and diagnostics. For every
//! model, `parse(print(nodes))` has no diagnostics and converts back to the
//! same nodes. The grammar is described in `docs/architecture/strings.md`.

use std::fmt::Write as _;

use crate::bytes::{Expr, Macro, Node};
use crate::catalog::{
    self, COMPARISONS, Close, Form, MacroSpec, NULLARY, PARAMETERS, Place, Role, STRUCTURE_WORDS,
};

/// Deepest nesting of tags, quoted strings, and expressions the parser
/// reads.
pub const MAX_NESTING_DEPTH: usize = 128;

/// A half-open byte range into the macro text.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Span {
    start: usize,
    end: usize,
}

impl Span {
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn start(self) -> usize {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> usize {
        self.end
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// The kind of a parse diagnostic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    /// The text ended inside a tag, a block, or a quoted string.
    UnexpectedEof,
    /// A character is not valid at this position.
    InvalidDelimiter,
    /// A backslash escapes a character that needs no escape.
    InvalidEscape,
    /// A tag name is not a macro.
    UnknownMacro,
    /// A macro has the wrong number or kind of arguments.
    InvalidArguments,
    /// A value is not valid, such as a number that does not fit 32 bits.
    InvalidValue,
    /// Bytes that are not a valid game string, kept only for display.
    RawBytes,
    /// Tags are nested deeper than [`MAX_NESTING_DEPTH`].
    NestingLimit,
}

/// A problem found while parsing macro text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub span: Span,
    pub kind: DiagnosticKind,
    /// A description written for people and the translation agent.
    pub message: String,
}

/// How a macro was written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Written {
    /// `<name args>`.
    Inline,
    /// The opening tag of a pair, such as `<i>` or `<color #FF0000FF>`.
    Open,
    /// The closing tag of a pair, such as `</i>`.
    Close,
    /// An element with content, such as `<if …>…</if>`.
    Block,
    /// `<code:XX args>`, for a code without a name or arguments that do
    /// not fit its entry.
    Generic,
}

/// One node of parsed macro text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxNode {
    /// The source range, including a block's content and closing tag.
    pub span: Span,
    pub kind: SyntaxKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyntaxKind {
    /// Text; the value has its escapes resolved.
    Text(String),
    Macro(MacroSyntax),
    /// `<raw XX …>`: bytes that are not a valid game string.
    Raw(Vec<u8>),
    /// Text that could not be parsed.
    Error,
}

/// A parsed macro.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroSyntax {
    pub code: u8,
    /// The catalog entry, unless the macro is written generically.
    pub spec: Option<&'static MacroSpec>,
    pub written: Written,
    /// The arguments in byte order.
    pub args: Vec<ExprSyntax>,
}

/// A parsed expression. For a block argument, the span is the block's
/// content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExprSyntax {
    pub span: Span,
    pub kind: ExprKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExprKind {
    Int(u32),
    /// A string: quoted or bare inline, or a block's content.
    Str(Vec<SyntaxNode>),
    Nullary(u8),
    Param(u8, Box<ExprSyntax>),
    Compare(u8, Box<ExprSyntax>, Box<ExprSyntax>),
    Raw(Vec<u8>),
    Error,
}

/// Parsed macro text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroString {
    source: String,
    nodes: Vec<SyntaxNode>,
    diagnostics: Vec<Diagnostic>,
}

impl MacroString {
    /// The macro text.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn nodes(&self) -> &[SyntaxNode] {
        &self.nodes
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The source text of `span`.
    #[must_use]
    pub fn slice(&self, span: Span) -> Option<&str> {
        self.source.get(span.start..span.end)
    }

    /// Whether the text parsed without diagnostics.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// The string model, when the text parsed without diagnostics.
    #[must_use]
    pub fn to_nodes(&self) -> Option<Vec<Node>> {
        self.is_well_formed()
            .then(|| self.nodes.iter().filter_map(node_model).collect())
    }
}

fn node_model(node: &SyntaxNode) -> Option<Node> {
    Some(match &node.kind {
        SyntaxKind::Text(text) => Node::Text(text.clone()),
        SyntaxKind::Macro(syntax) => Node::Macro(Macro {
            code: syntax.code,
            args: syntax
                .args
                .iter()
                .map(expr_to_model)
                .collect::<Option<_>>()?,
        }),
        SyntaxKind::Raw(bytes) => Node::Raw(bytes.clone()),
        SyntaxKind::Error => return None,
    })
}

/// The model of a parsed expression, unless it holds an error.
pub(crate) fn expr_to_model(expr: &ExprSyntax) -> Option<Expr> {
    Some(match &expr.kind {
        ExprKind::Int(value) => Expr::Int(*value),
        ExprKind::Str(nodes) => Expr::Str(nodes.iter().map(node_model).collect::<Option<_>>()?),
        ExprKind::Nullary(kind) => Expr::Nullary(*kind),
        ExprKind::Param(kind, operand) => Expr::Param(*kind, Box::new(expr_to_model(operand)?)),
        ExprKind::Compare(kind, left, right) => Expr::Compare(
            *kind,
            Box::new(expr_to_model(left)?),
            Box::new(expr_to_model(right)?),
        ),
        ExprKind::Raw(bytes) => Expr::Raw(bytes.clone()),
        ExprKind::Error => return None,
    })
}

// ---------------------------------------------------------------------------
// Printing

/// Writes nodes as macro text.
#[must_use]
pub fn print(nodes: &[Node]) -> String {
    let mut out = String::new();
    print_nodes(&mut out, nodes, false);
    out
}

fn print_nodes(out: &mut String, nodes: &[Node], quoted: bool) {
    for node in nodes {
        match node {
            Node::Text(text) => escape_text(out, text, quoted),
            Node::Raw(bytes) => {
                out.push_str("<raw");
                for byte in bytes {
                    let _ = write!(out, " {byte:02X}");
                }
                out.push('>');
            }
            Node::Macro(macro_node) => print_macro(out, macro_node),
        }
    }
}

fn escape_text(out: &mut String, text: &str, quoted: bool) {
    for character in text.chars() {
        if matches!(character, '\\' | '<' | '{') || (quoted && character == '"') {
            out.push('\\');
        }
        out.push(character);
    }
}

/// Whether `args` can be written in the entry's own form.
fn fits(spec: &MacroSpec, args: &[Expr]) -> bool {
    spec.accepts_count(args.len())
        && match spec.form {
            Form::Block { separator, leading } => {
                let inline = spec
                    .args
                    .iter()
                    .take(if spec.repeats {
                        spec.args.len() - 1
                    } else {
                        args.len()
                    })
                    .filter(|arg| arg.place == Place::Inline)
                    .count();
                let blocks = args.len() - inline;
                (separator.is_some() || blocks == 1) && (!leading || blocks >= 1)
            }
            Form::Inline | Form::Pair { .. } => true,
        }
}

fn print_macro(out: &mut String, macro_node: &Macro) {
    let Some(spec) = catalog::by_code(macro_node.code).filter(|spec| fits(spec, &macro_node.args))
    else {
        let _ = write!(out, "<code:{:02X}", macro_node.code);
        for arg in &macro_node.args {
            out.push(' ');
            print_inline(out, arg, Role::Other);
        }
        out.push('>');
        return;
    };
    let args = &macro_node.args;
    match spec.form {
        Form::Pair { close, implied } => {
            let is_close = match (close, args.as_slice()) {
                (Close::Int(value), [Expr::Int(actual)]) => *actual == value,
                (Close::StackColor, [Expr::Nullary(kind)]) => *kind == 0xEC,
                _ => false,
            };
            if is_close {
                let _ = write!(out, "</{}>", spec.name);
            } else if matches!((implied, args.as_slice()), (Some(value), [Expr::Int(actual)]) if *actual == value)
            {
                let _ = write!(out, "<{}>", spec.name);
            } else {
                print_inline_tag(out, spec, args);
            }
        }
        Form::Inline => print_inline_tag(out, spec, args),
        Form::Block { separator, leading } => {
            out.push('<');
            out.push_str(spec.name);
            let mut blocks = Vec::new();
            for (index, arg) in args.iter().enumerate() {
                let description = spec.arg(index).expect("fits checked the count");
                if description.place == Place::Inline {
                    out.push(' ');
                    print_inline(out, arg, description.role);
                } else {
                    blocks.push((arg, description.role));
                }
            }
            out.push('>');
            if !leading {
                while blocks.len() > 1
                    && matches!(blocks.last(), Some((Expr::Str(nodes), _)) if nodes.is_empty())
                {
                    blocks.pop();
                }
            }
            for (index, (arg, role)) in blocks.into_iter().enumerate() {
                if let Some(separator) = separator
                    && (leading || index > 0)
                {
                    let _ = write!(out, "<{separator}>");
                }
                match arg {
                    Expr::Str(nodes) => print_nodes(out, nodes, false),
                    other => {
                        out.push('{');
                        print_inline(out, other, role);
                        out.push('}');
                    }
                }
            }
            let _ = write!(out, "</{}>", spec.name);
        }
    }
}

fn print_inline_tag(out: &mut String, spec: &MacroSpec, args: &[Expr]) {
    out.push('<');
    out.push_str(spec.name);
    for (index, arg) in args.iter().enumerate() {
        out.push(' ');
        print_inline(
            out,
            arg,
            spec.arg(index).map_or(Role::Other, |arg| arg.role),
        );
    }
    out.push('>');
}

fn is_bare_word(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && text != "raw"
}

fn print_inline(out: &mut String, expr: &Expr, role: Role) {
    match expr {
        Expr::Int(value) if role == Role::Color => {
            let _ = write!(out, "#{value:08X}");
        }
        Expr::Int(value) => {
            let _ = write!(out, "{value}");
        }
        Expr::Str(nodes) => match nodes.as_slice() {
            [Node::Text(text)] if is_bare_word(text) => out.push_str(text),
            _ => {
                out.push('"');
                print_nodes(out, nodes, true);
                out.push('"');
            }
        },
        Expr::Nullary(kind) => {
            let name = NULLARY
                .iter()
                .find(|spec| spec.code == *kind)
                .map_or("?", |spec| spec.name);
            let _ = write!(out, "${name}");
        }
        Expr::Param(kind, operand) => {
            let prefix = PARAMETERS
                .iter()
                .find(|spec| spec.code == *kind)
                .map_or("?", |spec| spec.prefix);
            if let Expr::Int(number) = **operand {
                let _ = write!(out, "${prefix}{number}");
            } else {
                let _ = write!(out, "${prefix}(");
                print_inline(out, operand, Role::Other);
                out.push(')');
            }
        }
        Expr::Compare(kind, left, right) => {
            let operator = COMPARISONS
                .iter()
                .find(|spec| spec.code == *kind)
                .map_or("?", |spec| spec.operator);
            out.push('(');
            print_inline(out, left, Role::Other);
            let _ = write!(out, " {operator} ");
            print_inline(out, right, Role::Other);
            out.push(')');
        }
        Expr::Raw(bytes) => {
            out.push_str("raw(");
            for (index, byte) in bytes.iter().enumerate() {
                if index > 0 {
                    out.push(' ');
                }
                let _ = write!(out, "{byte:02X}");
            }
            out.push(')');
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing

/// Parses macro text. The result keeps the source, and reports every
/// problem as a diagnostic with a span.
#[must_use]
pub fn parse(source: &str) -> MacroString {
    let mut parser = Parser {
        source,
        position: 0,
        depth: 0,
        diagnostics: Vec::new(),
    };
    let (nodes, _) = parser.content(&Stop::End);
    MacroString {
        source: source.to_owned(),
        nodes,
        diagnostics: parser.diagnostics,
    }
}

/// What ends a run of content.
enum Stop<'a> {
    /// The end of the text.
    End,
    /// A closing quote.
    Quote,
    /// A block's closing tag or separator.
    Block {
        name: &'a str,
        separator: Option<&'a str>,
    },
}

/// What a `<` at the current position is, when it is not an ordinary tag.
enum Structure {
    /// A separator or closing tag that ends the current content.
    Ended(Ended),
    /// A misplaced separator or closing tag, consumed and diagnosed.
    Consumed,
    /// An ordinary tag.
    None,
}

/// Why content ended.
#[derive(Debug, Eq, PartialEq)]
enum Ended {
    End,
    Quote,
    Separator,
    Close,
    /// The text ended before the expected end; already diagnosed.
    Eof,
}

struct Parser<'a> {
    source: &'a str,
    position: usize,
    depth: usize,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn rest(&self) -> &'a str {
        let source = self.source;
        &source[self.position..]
    }

    fn diagnose(&mut self, span: Span, kind: DiagnosticKind, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            span,
            kind,
            message: message.into(),
        });
    }

    fn here(&self) -> Span {
        let width = self.peek().map_or(0, char::len_utf8);
        Span::new(self.position, self.position + width)
    }

    fn skip_spaces(&mut self) {
        while let Some(character) = self.peek() {
            if !character.is_whitespace() {
                break;
            }
            self.position += character.len_utf8();
        }
    }

    /// Skips to just after the next `>` outside quotes, to recover from an
    /// error inside a tag.
    fn recover(&mut self) {
        let mut quoted = false;
        while let Some(character) = self.peek() {
            self.position += character.len_utf8();
            match character {
                '\\' => {
                    if let Some(next) = self.peek() {
                        self.position += next.len_utf8();
                    }
                }
                '"' => quoted = !quoted,
                '>' if !quoted => return,
                _ => {}
            }
        }
    }

    fn enter(&mut self, start: usize) -> bool {
        if self.depth >= MAX_NESTING_DEPTH {
            self.diagnose(
                Span::new(start, self.source.len()),
                DiagnosticKind::NestingLimit,
                "tags are nested too deeply",
            );
            self.position = self.source.len();
            return false;
        }
        self.depth += 1;
        true
    }

    /// Reads text and tags until `stop`.
    fn content(&mut self, stop: &Stop<'_>) -> (Vec<SyntaxNode>, Ended) {
        let mut nodes = Vec::new();
        let mut text = String::new();
        let mut text_start = self.position;
        let flush = |nodes: &mut Vec<SyntaxNode>, text: &mut String, start: usize, end: usize| {
            if !text.is_empty() {
                nodes.push(SyntaxNode {
                    span: Span::new(start, end),
                    kind: SyntaxKind::Text(std::mem::take(text)),
                });
            }
        };
        loop {
            let Some(character) = self.peek() else {
                flush(&mut nodes, &mut text, text_start, self.position);
                return match stop {
                    Stop::End => (nodes, Ended::End),
                    Stop::Quote => {
                        self.diagnose(
                            Span::new(self.position, self.position),
                            DiagnosticKind::UnexpectedEof,
                            "a quoted string is missing its closing \"",
                        );
                        (nodes, Ended::Eof)
                    }
                    Stop::Block { name, .. } => {
                        let name = (*name).to_owned();
                        self.diagnose(
                            Span::new(self.position, self.position),
                            DiagnosticKind::UnexpectedEof,
                            format!("<{name}> is missing its closing </{name}>"),
                        );
                        (nodes, Ended::Eof)
                    }
                };
            };
            match character {
                '\\' => {
                    if text.is_empty() {
                        text_start = self.position;
                    }
                    text.extend(self.escape());
                }
                '"' if matches!(stop, Stop::Quote) => {
                    flush(&mut nodes, &mut text, text_start, self.position);
                    self.position += 1;
                    return (nodes, Ended::Quote);
                }
                '{' => {
                    self.diagnose(
                        self.here(),
                        DiagnosticKind::InvalidDelimiter,
                        "a { is only valid as the whole content of a branch; write \\{ for a literal {",
                    );
                    self.position += 1;
                }
                '\u{0}' | '\u{2}' => {
                    self.diagnose(
                        self.here(),
                        DiagnosticKind::InvalidValue,
                        "the text contains a control character the game reserves",
                    );
                    self.position += 1;
                }
                '<' => {
                    flush(&mut nodes, &mut text, text_start, self.position);
                    match self.structure_tag(stop) {
                        Structure::Ended(ended) => return (nodes, ended),
                        Structure::Consumed => {}
                        Structure::None => nodes.extend(self.tag()),
                    }
                    text_start = self.position;
                }
                other => {
                    if text.is_empty() {
                        text_start = self.position;
                    }
                    text.push(other);
                    self.position += other.len_utf8();
                }
            }
        }
    }

    /// Reads a backslash escape and returns its character.
    fn escape(&mut self) -> Option<char> {
        let start = self.position;
        self.position += 1;
        match self.peek() {
            Some(escaped @ ('\\' | '<' | '{' | '"')) => {
                self.position += 1;
                Some(escaped)
            }
            Some(other) => {
                self.position += other.len_utf8();
                self.diagnose(
                    Span::new(start, self.position),
                    DiagnosticKind::InvalidEscape,
                    format!("\\{other} is not an escape; only \\\\, \\<, \\{{, and \\\" are"),
                );
                None
            }
            None => {
                self.diagnose(
                    Span::new(start, self.position),
                    DiagnosticKind::InvalidEscape,
                    "a backslash at the end of the text escapes nothing",
                );
                None
            }
        }
    }

    /// Reads a separator or closing tag at `<`.
    fn structure_tag(&mut self, stop: &Stop<'_>) -> Structure {
        let rest = self.rest();
        let (closing, name_start) = if rest.starts_with("</") {
            (true, 2)
        } else {
            (false, 1)
        };
        let name_end = rest[name_start..]
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
            .map_or(rest.len(), |end| end + name_start);
        let name = &rest[name_start..name_end];
        if !rest[name_end..].starts_with('>') {
            return Structure::None;
        }
        let length = name_end + 1;
        let start = self.position;
        match stop {
            Stop::Block {
                name: block,
                separator,
            } => {
                if closing && name == *block {
                    self.position += length;
                    return Structure::Ended(Ended::Close);
                }
                if !closing && Some(name) == *separator {
                    self.position += length;
                    return Structure::Ended(Ended::Separator);
                }
            }
            Stop::End | Stop::Quote => {}
        }
        if STRUCTURE_WORDS.contains(&name) {
            self.position += length;
            self.diagnose(
                Span::new(start, start + length),
                DiagnosticKind::InvalidDelimiter,
                format!(
                    "<{}{name}> is not valid here",
                    if closing { "/" } else { "" }
                ),
            );
            return Structure::Consumed;
        }
        if closing
            && catalog::by_name(name).is_some_and(|spec| matches!(spec.form, Form::Block { .. }))
        {
            self.position += length;
            self.diagnose(
                Span::new(start, start + length),
                DiagnosticKind::InvalidDelimiter,
                format!("</{name}> does not close an open <{name}>"),
            );
            return Structure::Consumed;
        }
        Structure::None
    }

    /// Reads one tag at `<`. Returns `None` after a diagnostic that
    /// consumed the tag.
    fn tag(&mut self) -> Option<SyntaxNode> {
        let start = self.position;
        if self.position >= self.source.len() {
            return None;
        }
        if !self.enter(start) {
            return None;
        }
        let node = self.tag_inner(start);
        self.depth -= 1;
        node
    }

    fn tag_inner(&mut self, start: usize) -> Option<SyntaxNode> {
        self.position += 1;
        let closing = self.peek() == Some('/');
        if closing {
            self.position += 1;
        }
        let name_start = self.position;
        while let Some(character) = self.peek() {
            if !(character.is_ascii_alphanumeric() || matches!(character, '-' | ':')) {
                break;
            }
            self.position += 1;
        }
        let name = &self.source[name_start..self.position];
        if name.is_empty() {
            self.diagnose(
                Span::new(start, start + 1),
                DiagnosticKind::InvalidDelimiter,
                "a < starts a tag; write \\< for a literal <",
            );
            return None;
        }
        let node = if closing {
            self.closing_pair(start, name)
        } else if name == "raw" {
            self.raw_node(start)
        } else if let Some(code) = name.strip_prefix("code:") {
            self.generic(start, code)
        } else if let Some(spec) = catalog::by_name(name) {
            self.named(start, spec)
        } else {
            let name = name.to_owned();
            self.recover();
            self.diagnose(
                Span::new(start, self.position),
                DiagnosticKind::UnknownMacro,
                unknown_macro_message(&name),
            );
            None
        };
        node.or_else(|| {
            Some(SyntaxNode {
                span: Span::new(start, self.position),
                kind: SyntaxKind::Error,
            })
        })
    }

    fn closing_pair(&mut self, start: usize, name: &str) -> Option<SyntaxNode> {
        let spec = catalog::by_name(name);
        let Some((spec, close)) = spec.and_then(|spec| match spec.form {
            Form::Pair { close, .. } => Some((spec, close)),
            _ => None,
        }) else {
            let name = name.to_owned();
            self.recover();
            self.diagnose(
                Span::new(start, self.position),
                DiagnosticKind::InvalidDelimiter,
                if catalog::by_name(&name).is_some() {
                    format!("</{name}> does not close an open <{name}>")
                } else {
                    unknown_macro_message(&name)
                },
            );
            return None;
        };
        if self.peek() != Some('>') {
            self.recover();
            self.diagnose(
                Span::new(start, self.position),
                DiagnosticKind::InvalidDelimiter,
                format!("</{}> takes no arguments", spec.name),
            );
            return None;
        }
        self.position += 1;
        let span = Span::new(start, self.position);
        let arg = ExprSyntax {
            span: Span::new(self.position - 1, self.position - 1),
            kind: match close {
                Close::Int(value) => ExprKind::Int(value),
                Close::StackColor => ExprKind::Nullary(0xEC),
            },
        };
        Some(SyntaxNode {
            span,
            kind: SyntaxKind::Macro(MacroSyntax {
                code: spec.code,
                spec: Some(spec),
                written: Written::Close,
                args: vec![arg],
            }),
        })
    }

    fn raw_node(&mut self, start: usize) -> Option<SyntaxNode> {
        let body_start = self.position;
        let Some(end) = self.rest().find('>') else {
            self.position = self.source.len();
            self.diagnose(
                Span::new(start, self.position),
                DiagnosticKind::UnexpectedEof,
                "<raw> is missing its >",
            );
            return None;
        };
        let body = &self.source[body_start..body_start + end];
        self.position = body_start + end + 1;
        let span = Span::new(start, self.position);
        let Some(bytes) = parse_hex_bytes(body) else {
            self.diagnose(
                span,
                DiagnosticKind::InvalidValue,
                "<raw> holds two-digit hexadecimal bytes separated by spaces",
            );
            return None;
        };
        self.diagnose(
            span,
            DiagnosticKind::RawBytes,
            "<raw> bytes are not a valid game string and cannot be written; keep the text around them and remove the <raw> tag",
        );
        Some(SyntaxNode {
            span,
            kind: SyntaxKind::Raw(bytes),
        })
    }

    fn generic(&mut self, start: usize, code: &str) -> Option<SyntaxNode> {
        let Some(code) = (code.len() == 2)
            .then(|| u8::from_str_radix(code, 16).ok())
            .flatten()
        else {
            self.recover();
            self.diagnose(
                Span::new(start, self.position),
                DiagnosticKind::InvalidValue,
                "<code:XX> needs a two-digit hexadecimal macro code",
            );
            return None;
        };
        let args = self.inline_args(start, &|_| Role::Other)?;
        Some(SyntaxNode {
            span: Span::new(start, self.position),
            kind: SyntaxKind::Macro(MacroSyntax {
                code,
                spec: None,
                written: Written::Generic,
                args,
            }),
        })
    }

    /// Reads inline arguments up to and including `>`.
    fn inline_args(
        &mut self,
        start: usize,
        role: &dyn Fn(usize) -> Role,
    ) -> Option<Vec<ExprSyntax>> {
        let mut args = Vec::new();
        loop {
            let before = self.position;
            self.skip_spaces();
            match self.peek() {
                Some('>') => {
                    self.position += 1;
                    return Some(args);
                }
                None => {
                    self.diagnose(
                        Span::new(start, self.position),
                        DiagnosticKind::UnexpectedEof,
                        "a tag is missing its >",
                    );
                    return None;
                }
                Some(_) if before == self.position && !args.is_empty() => {
                    self.recover();
                    self.diagnose(
                        Span::new(before, self.position),
                        DiagnosticKind::InvalidDelimiter,
                        "arguments are separated by spaces",
                    );
                    return None;
                }
                Some(_) => {
                    let expr = self.expr(role(args.len()));
                    if matches!(expr.kind, ExprKind::Error) {
                        self.recover();
                        return None;
                    }
                    args.push(expr);
                }
            }
        }
    }

    fn named(&mut self, start: usize, spec: &'static MacroSpec) -> Option<SyntaxNode> {
        let role = |index: usize| spec.arg(index).map_or(Role::Other, |arg| arg.role);
        let (written, args) = match spec.form {
            Form::Inline => (Written::Inline, self.inline_args(start, &role)?),
            Form::Pair { implied, .. } => {
                let mut args = self.inline_args(start, &role)?;
                if args.is_empty()
                    && let Some(value) = implied
                {
                    args.push(ExprSyntax {
                        span: Span::new(self.position - 1, self.position - 1),
                        kind: ExprKind::Int(value),
                    });
                }
                (Written::Open, args)
            }
            Form::Block { separator, leading } => {
                let inline_roles: Vec<Role> = spec
                    .args
                    .iter()
                    .filter(|arg| arg.place == Place::Inline)
                    .map(|arg| arg.role)
                    .collect();
                let inline = self.inline_args(start, &|index| {
                    inline_roles.get(index).copied().unwrap_or(Role::Other)
                })?;
                if inline.len() != inline_roles.len() {
                    self.diagnose(
                        Span::new(start, self.position),
                        DiagnosticKind::InvalidArguments,
                        format!(
                            "<{}> takes {} in its opening tag, found {} argument(s)",
                            spec.name,
                            describe_names(spec, Place::Inline),
                            inline.len()
                        ),
                    );
                }
                let blocks = self.blocks(spec, separator, leading)?;
                (Written::Block, self.assemble(start, spec, inline, blocks)?)
            }
        };
        let span = Span::new(start, self.position);
        if !spec.accepts_count(args.len()) {
            self.diagnose(
                span,
                DiagnosticKind::InvalidArguments,
                format!(
                    "<{}> takes {}, found {} argument(s)",
                    spec.name,
                    describe_count(spec),
                    args.len()
                ),
            );
        }
        Some(SyntaxNode {
            span,
            kind: SyntaxKind::Macro(MacroSyntax {
                code: spec.code,
                spec: Some(spec),
                written,
                args,
            }),
        })
    }

    /// Reads the content of a block element up to its closing tag.
    fn blocks(
        &mut self,
        spec: &'static MacroSpec,
        separator: Option<&'static str>,
        leading: bool,
    ) -> Option<Vec<ExprSyntax>> {
        let stop = Stop::Block {
            name: spec.name,
            separator,
        };
        let mut blocks = Vec::new();
        if leading {
            let content_start = self.position;
            let (nodes, ended) = self.content(&stop);
            if !nodes
                .iter()
                .all(|node| matches!(&node.kind, SyntaxKind::Text(text) if text.trim().is_empty()))
            {
                self.diagnose(
                    Span::new(content_start, self.position),
                    DiagnosticKind::InvalidDelimiter,
                    format!(
                        "the content of <{}> starts with <{}>",
                        spec.name,
                        separator.unwrap_or_default()
                    ),
                );
            }
            match ended {
                Ended::Separator => {}
                Ended::Close => return Some(blocks),
                _ => return None,
            }
        }
        loop {
            let content_start = self.position;
            let expr = if self.peek() == Some('{') {
                self.position += 1;
                self.skip_spaces();
                let role = spec
                    .args
                    .iter()
                    .find(|arg| arg.place == Place::Block)
                    .map_or(Role::Other, |arg| arg.role);
                let inner = self.expr(role);
                self.skip_spaces();
                if self.peek() == Some('}') {
                    self.position += 1;
                } else {
                    self.diagnose(
                        self.here(),
                        DiagnosticKind::InvalidDelimiter,
                        "a { value } is missing its }",
                    );
                }
                ExprSyntax {
                    span: Span::new(content_start, self.position),
                    kind: inner.kind,
                }
            } else {
                let (nodes, ended) = self.content(&stop);
                let expr = ExprSyntax {
                    span: Span::new(content_start, self.content_end(&ended)),
                    kind: ExprKind::Str(nodes),
                };
                blocks.push(expr);
                match ended {
                    Ended::Separator => continue,
                    Ended::Close => return Some(blocks),
                    _ => return None,
                }
            };
            blocks.push(expr);
            let after = if self.peek() == Some('<') {
                self.structure_tag(&stop)
            } else {
                Structure::None
            };
            match after {
                Structure::Ended(Ended::Separator) => {}
                Structure::Ended(Ended::Close) => return Some(blocks),
                _ => {
                    self.diagnose(
                        self.here(),
                        DiagnosticKind::InvalidDelimiter,
                        format!(
                            "a {{ value }} must be the whole content of a branch of <{}>",
                            spec.name
                        ),
                    );
                    let (_, ended) = self.content(&stop);
                    match ended {
                        Ended::Separator => {}
                        Ended::Close => return Some(blocks),
                        _ => return None,
                    }
                }
            }
        }
    }

    /// The end of a block's content: before the separator or closing tag
    /// that ended it.
    fn content_end(&self, ended: &Ended) -> usize {
        match ended {
            Ended::Separator | Ended::Close => {
                let before = &self.source[..self.position];
                before.rfind('<').unwrap_or(self.position)
            }
            _ => self.position,
        }
    }

    /// Puts inline and block arguments into byte order.
    fn assemble(
        &mut self,
        start: usize,
        spec: &'static MacroSpec,
        inline: Vec<ExprSyntax>,
        mut blocks: Vec<ExprSyntax>,
    ) -> Option<Vec<ExprSyntax>> {
        let fixed_blocks = spec
            .args
            .iter()
            .filter(|arg| arg.place == Place::Block)
            .count();
        if !spec.repeats {
            if blocks.len() > fixed_blocks {
                self.diagnose(
                    Span::new(start, self.position),
                    DiagnosticKind::InvalidArguments,
                    format!(
                        "<{}> has {} branch(es), found {}",
                        spec.name,
                        fixed_blocks,
                        blocks.len()
                    ),
                );
                return None;
            }
            while blocks.len() < fixed_blocks {
                blocks.push(ExprSyntax {
                    span: Span::new(self.position, self.position),
                    kind: ExprKind::Str(Vec::new()),
                });
            }
        }
        let mut inline = inline.into_iter();
        let mut blocks = blocks.into_iter();
        let mut args = Vec::new();
        for (index, arg) in spec.args.iter().enumerate() {
            let last = index + 1 == spec.args.len();
            match arg.place {
                Place::Inline => args.extend(inline.next()),
                Place::Block if last && spec.repeats => args.extend(blocks.by_ref()),
                Place::Block => args.extend(blocks.next()),
            }
        }
        Some(args)
    }

    /// Reads one expression.
    fn expr(&mut self, role: Role) -> ExprSyntax {
        let start = self.position;
        if !self.enter(start) {
            return ExprSyntax {
                span: Span::new(start, self.position),
                kind: ExprKind::Error,
            };
        }
        let kind = self.expr_kind(role);
        self.depth -= 1;
        ExprSyntax {
            span: Span::new(start, self.position),
            kind,
        }
    }

    fn expr_kind(&mut self, role: Role) -> ExprKind {
        let start = self.position;
        let Some(first) = self.peek() else {
            self.diagnose(
                self.here(),
                DiagnosticKind::UnexpectedEof,
                "a value is missing",
            );
            return ExprKind::Error;
        };
        match first {
            '0'..='9' => {
                let digits = self.take_while(|character| character.is_ascii_digit());
                self.int_value(start, digits.parse().ok(), "a number must fit 32 bits")
            }
            '#' => {
                self.position += 1;
                let digits = self.take_while(|character| character.is_ascii_hexdigit());
                let value = (!digits.is_empty() && digits.len() <= 8)
                    .then(|| u32::from_str_radix(digits, 16).ok())
                    .flatten();
                self.int_value(
                    start,
                    value,
                    "a # value has one to eight hexadecimal digits, such as #FF0000FF",
                )
            }
            '$' => self.variable(start),
            '(' => self.comparison(start),
            '"' => {
                self.position += 1;
                let (nodes, ended) = self.content(&Stop::Quote);
                if ended == Ended::Quote {
                    ExprKind::Str(nodes)
                } else {
                    ExprKind::Error
                }
            }
            _ if self.rest().starts_with("raw(") => {
                self.position += 4;
                let Some(end) = self.rest().find(')') else {
                    self.position = self.source.len();
                    self.diagnose(
                        Span::new(start, self.position),
                        DiagnosticKind::UnexpectedEof,
                        "raw( is missing its )",
                    );
                    return ExprKind::Error;
                };
                let body = &self.source[self.position..self.position + end];
                self.position += end + 1;
                let span = Span::new(start, self.position);
                if let Some(bytes) = parse_hex_bytes(body) {
                    self.diagnose(
                        span,
                        DiagnosticKind::RawBytes,
                        "raw( ) bytes are not a valid value and cannot be written",
                    );
                    ExprKind::Raw(bytes)
                } else {
                    self.diagnose(
                        span,
                        DiagnosticKind::InvalidValue,
                        "raw( ) holds two-digit hexadecimal bytes separated by spaces",
                    );
                    ExprKind::Error
                }
            }
            character if character.is_ascii_alphabetic() || character == '_' => {
                let word = self
                    .take_while(|character| character.is_ascii_alphanumeric() || character == '_');
                ExprKind::Str(vec![SyntaxNode {
                    span: Span::new(start, self.position),
                    kind: SyntaxKind::Text(word.to_owned()),
                }])
            }
            _ => {
                let _ = role;
                self.diagnose(
                    self.here(),
                    DiagnosticKind::InvalidDelimiter,
                    "expected a value: a number, #hex, $parameter, (comparison), \"text\", or a word",
                );
                ExprKind::Error
            }
        }
    }

    fn take_while(&mut self, accept: impl Fn(char) -> bool) -> &'a str {
        let source = self.source;
        let start = self.position;
        while let Some(character) = self.peek() {
            if !accept(character) {
                break;
            }
            self.position += character.len_utf8();
        }
        &source[start..self.position]
    }

    fn int_value(&mut self, start: usize, value: Option<u32>, message: &str) -> ExprKind {
        if let Some(value) = value {
            ExprKind::Int(value)
        } else {
            self.diagnose(
                Span::new(start, self.position),
                DiagnosticKind::InvalidValue,
                message,
            );
            ExprKind::Error
        }
    }

    fn variable(&mut self, start: usize) -> ExprKind {
        self.position += 1;
        let name = self
            .take_while(|character| character.is_ascii_alphanumeric() || character == '_')
            .to_owned();
        if let Some(spec) = NULLARY.iter().find(|spec| spec.name == name) {
            return ExprKind::Nullary(spec.code);
        }
        for spec in PARAMETERS {
            let Some(number) = name.strip_prefix(spec.prefix) else {
                continue;
            };
            if number.is_empty() && self.peek() == Some('(') {
                self.position += 1;
                self.skip_spaces();
                let operand = self.expr(Role::Other);
                self.skip_spaces();
                if self.peek() == Some(')') {
                    self.position += 1;
                    return ExprKind::Param(spec.code, Box::new(operand));
                }
                self.diagnose(
                    self.here(),
                    DiagnosticKind::InvalidDelimiter,
                    "a parameter operand is missing its )",
                );
                return ExprKind::Error;
            }
            if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
                let Ok(value) = number.parse() else {
                    break;
                };
                let operand_start = start + 1 + spec.prefix.len();
                return ExprKind::Param(
                    spec.code,
                    Box::new(ExprSyntax {
                        span: Span::new(operand_start, self.position),
                        kind: ExprKind::Int(value),
                    }),
                );
            }
        }
        self.diagnose(
            Span::new(start, self.position),
            DiagnosticKind::InvalidValue,
            format!(
                "${name} is not a value; use $n1, $s1, $gn1, $gs1, or one of {}",
                NULLARY
                    .iter()
                    .map(|spec| format!("${}", spec.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
        ExprKind::Error
    }

    fn comparison(&mut self, start: usize) -> ExprKind {
        self.position += 1;
        self.skip_spaces();
        let left = self.expr(Role::Other);
        if matches!(left.kind, ExprKind::Error) {
            return ExprKind::Error;
        }
        self.skip_spaces();
        let Some(operator) = COMPARISONS
            .iter()
            .find(|spec| self.rest().starts_with(spec.operator))
        else {
            self.diagnose(
                self.here(),
                DiagnosticKind::InvalidDelimiter,
                "expected a comparison: ==, !=, <, <=, >, or >=",
            );
            return ExprKind::Error;
        };
        self.position += operator.operator.len();
        self.skip_spaces();
        let right = self.expr(Role::Other);
        if matches!(right.kind, ExprKind::Error) {
            return ExprKind::Error;
        }
        self.skip_spaces();
        if self.peek() != Some(')') {
            self.diagnose(
                Span::new(start, self.position),
                DiagnosticKind::InvalidDelimiter,
                "a comparison is missing its )",
            );
            return ExprKind::Error;
        }
        self.position += 1;
        ExprKind::Compare(operator.code, Box::new(left), Box::new(right))
    }
}

fn parse_hex_bytes(body: &str) -> Option<Vec<u8>> {
    let bytes: Option<Vec<u8>> = body
        .split_whitespace()
        .map(|token| {
            (token.len() == 2)
                .then(|| u8::from_str_radix(token, 16).ok())
                .flatten()
        })
        .collect();
    bytes.filter(|bytes| !bytes.is_empty())
}

fn describe_names(spec: &MacroSpec, place: Place) -> String {
    let names: Vec<&str> = spec
        .args
        .iter()
        .filter(|arg| arg.place == place)
        .map(|arg| arg.name)
        .collect();
    if names.is_empty() {
        "no arguments".to_owned()
    } else {
        names.join(", ")
    }
}

fn describe_count(spec: &MacroSpec) -> String {
    let names: Vec<&str> = spec.args.iter().map(|arg| arg.name).collect();
    match (spec.required, spec.args.len(), spec.repeats) {
        (_, 0, _) => "no arguments".to_owned(),
        (_, _, true) => format!("{} or more ({}, …)", spec.required, names.join(", ")),
        (required, total, false) if required == total => {
            format!("{total} ({})", names.join(", "))
        }
        (required, total, false) => format!("{required} to {total} ({})", names.join(", ")),
    }
}

fn unknown_macro_message(name: &str) -> String {
    let suggestion = catalog::MACROS
        .iter()
        .map(|spec| spec.name)
        .min_by_key(|candidate| edit_distance(name, candidate))
        .filter(|candidate| edit_distance(name, candidate) <= 2);
    match suggestion {
        Some(candidate) => format!("<{name}> is not a macro; did you mean <{candidate}>?"),
        None => format!("<{name}> is not a macro; write \\< for a literal <"),
    }
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_char) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            current.push(
                substitution
                    .min(previous[column + 1] + 1)
                    .min(current[column] + 1),
            );
        }
        previous = current;
    }
    previous[right.len()]
}
