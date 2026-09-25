//! Tagged text for assisted translation.
//!
//! A source macro string is projected into text in which prose is plain text
//! and every protected construct is inline markup: `<x id="N"/>` for a
//! construct without translatable content, and `<g id="N"><b>…</b>…</g>`
//! for a construct whose user-facing string arguments (such as the branches
//! of `<if(…)>`) are translated in place. A translation written in this form
//! is rebuilt into a macro string by splicing the translated text back into
//! the source syntax, so protected constructs keep their exact spelling.
//!
//! [`rebuild`] checks the tags against the structure policy and
//! [`check_assisted_structure`] checks the rebuilt syntax tree again,
//! independently of the tags:
//!
//! - every construct is kept; none is dropped;
//! - a construct stays in its container (the root or one branch);
//! - constructs may move within their container, except that formatting
//!   constructs keep their relative order so start and end pairs cannot cross;
//! - only runtime values without branches (such as the player's name) may
//!   repeat;
//! - branch constructs keep their number of branches, and changed game
//!   references, parameters, or opaque constructs are new constructs, which
//!   are rejected.

use std::fmt::Write as _;

use crate::semantic::is_user_facing_argument;
use crate::{
    ExpressionKind, MacroString, ProtectedExpression, ProtectedExpressionKind, ProtectedNode,
    ProtectedNodeKind, SemanticFamily, Span, SyntaxKind, SyntaxNode, parse,
};

/// Longest construct spelling shown in a tag legend.
const LEGEND_SPELLING_CHARS: usize = 160;

/// A source string in tagged form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaggedText {
    /// The tagged text, with `&`, `<`, and `>` in prose written as entities.
    pub text: String,
    /// Every tag, ordered by ID.
    pub tags: Vec<Tag>,
}

/// One protected construct of the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tag {
    pub id: u32,
    /// `None` for a construct Aeria does not understand.
    pub family: Option<SemanticFamily>,
    /// The exact source spelling of the construct.
    pub spelling: String,
    /// Number of translatable branches; zero for an `<x/>` tag.
    pub branches: usize,
    /// The construct may appear more than once in the translation.
    pub repeatable: bool,
}

impl Tag {
    /// A one-line description for the model.
    #[must_use]
    pub fn legend(&self) -> String {
        let role = match self.family {
            Some(SemanticFamily::RuntimeContextValue) => "runtime value",
            Some(SemanticFamily::ConditionalSelection) => "condition",
            Some(SemanticFamily::FormattingPresentation) => {
                "formatting; keep the order of formatting tags"
            }
            Some(SemanticFamily::GameDataReference) => "game data reference",
            Some(SemanticFamily::LayoutTextualControl) => "layout control",
            Some(SemanticFamily::TranslatableText) => "text transform",
            Some(SemanticFamily::Expression | SemanticFamily::OpaqueProtected) | None => {
                "protected construct"
            }
        };
        let mut spelling: String = self.spelling.chars().take(LEGEND_SPELLING_CHARS).collect();
        if spelling.len() < self.spelling.len() {
            spelling.push('…');
        }
        let shape = if self.branches > 0 {
            format!("; {} translatable branches in <b>", self.branches)
        } else if self.repeatable {
            "; may repeat".to_owned()
        } else {
            String::new()
        };
        format!("{}: {spelling} ({role}{shape})", self.id)
    }
}

/// A reason a tagged translation or its structure was refused. Messages are
/// written for the model that produced the translation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaggedError {
    pub message: String,
}

impl TaggedError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Container {
    Root,
    Branch { group: u32, index: usize },
}

impl Container {
    fn describe(self) -> String {
        match self {
            Self::Root => "the top level".to_owned(),
            Self::Branch { group, index } => format!("branch {} of tag {group}", index + 1),
        }
    }
}

struct Construct {
    span: Span,
    container: Container,
    branch_spans: Vec<Span>,
    tag: Tag,
}

struct SourceModel {
    document: MacroString,
    constructs: Vec<Construct>,
}

impl SourceModel {
    fn build(source: &str) -> Result<(Self, String), TaggedError> {
        let document = parse(source);
        if !document.is_well_formed() {
            return Err(TaggedError::new(
                "the source string is malformed and cannot be translated with tags",
            ));
        }
        let mut model = Self {
            document,
            constructs: Vec::new(),
        };
        let mut text = String::new();
        let nodes = model.document.nodes().to_vec();
        model.walk(&nodes, Container::Root, &mut text);
        Ok((model, text))
    }

    fn walk(&mut self, nodes: &[SyntaxNode], container: Container, text: &mut String) {
        for node in nodes {
            match &node.kind {
                SyntaxKind::Text => {
                    let raw = self
                        .document
                        .slice(node.span)
                        .unwrap_or_default()
                        .to_owned();
                    push_entities(text, &raw);
                }
                SyntaxKind::Escape { character } => {
                    push_entities(text, &character.to_string());
                }
                SyntaxKind::Macro(macro_node) => {
                    let branches: Vec<(Span, Vec<SyntaxNode>)> = macro_node
                        .arguments
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| is_user_facing_argument(macro_node.name, *index))
                        .filter_map(|(_, argument)| match &argument.kind {
                            ExpressionKind::String { parts } => {
                                Some((argument.span, parts.clone()))
                            }
                            _ => None,
                        })
                        .collect();
                    let family = macro_node.name.semantic_family();
                    let id = self.push(node.span, container, Some(family), &branches);
                    if branches.is_empty() {
                        let _ = write!(text, "<x id=\"{id}\"/>");
                    } else {
                        let _ = write!(text, "<g id=\"{id}\">");
                        for (index, (_, parts)) in branches.iter().enumerate() {
                            text.push_str("<b>");
                            self.walk(parts, Container::Branch { group: id, index }, text);
                            text.push_str("</b>");
                        }
                        text.push_str("</g>");
                    }
                }
                SyntaxKind::Opaque(_) | SyntaxKind::Malformed => {
                    let id = self.push(node.span, container, None, &[]);
                    let _ = write!(text, "<x id=\"{id}\"/>");
                }
            }
        }
    }

    fn push(
        &mut self,
        span: Span,
        container: Container,
        family: Option<SemanticFamily>,
        branches: &[(Span, Vec<SyntaxNode>)],
    ) -> u32 {
        let id = u32::try_from(self.constructs.len() + 1).unwrap_or(u32::MAX);
        self.constructs.push(Construct {
            span,
            container,
            branch_spans: branches.iter().map(|(span, _)| *span).collect(),
            tag: Tag {
                id,
                family,
                spelling: self.document.slice(span).unwrap_or_default().to_owned(),
                branches: branches.len(),
                repeatable: branches.is_empty()
                    && family == Some(SemanticFamily::RuntimeContextValue),
            },
        });
        id
    }

    fn construct(&self, id: u32) -> Option<&Construct> {
        usize::try_from(id)
            .ok()
            .and_then(|id| id.checked_sub(1))
            .and_then(|index| self.constructs.get(index))
    }
}

fn push_entities(text: &mut String, raw: &str) {
    for character in raw.chars() {
        match character {
            '&' => text.push_str("&amp;"),
            '<' => text.push_str("&lt;"),
            '>' => text.push_str("&gt;"),
            other => text.push(other),
        }
    }
}

/// Projects a source macro string into tagged text.
///
/// # Errors
///
/// Returns an error for a malformed source, which has no safe projection.
pub fn project(source: &str) -> Result<TaggedText, TaggedError> {
    let (model, text) = SourceModel::build(source)?;
    Ok(TaggedText {
        text,
        tags: model
            .constructs
            .into_iter()
            .map(|construct| construct.tag)
            .collect(),
    })
}

#[derive(Debug)]
enum Item {
    Text(String),
    Atom(u32),
    Group { id: u32, branches: Vec<Vec<Item>> },
}

enum Token {
    Text(String),
    Atom(u32),
    Open(u32),
    Close,
    BranchOpen,
    BranchClose,
}

fn tokenize(tagged: &str) -> Result<Vec<Token>, TaggedError> {
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut rest = tagged;
    while let Some(character) = rest.chars().next() {
        match character {
            '<' => {
                let end = rest.find('>').ok_or_else(|| {
                    TaggedError::new("a '<' has no closing '>'; write &lt; for a literal '<'")
                })?;
                let markup = &rest[1..end];
                let token = parse_markup(markup).ok_or_else(|| {
                    TaggedError::new(format!(
                        "unexpected markup <{markup}>; only <x id=\"N\"/>, <g id=\"N\">, </g>, <b>, and </b> are allowed, and a literal '<' is written &lt;"
                    ))
                })?;
                if !text.is_empty() {
                    tokens.push(Token::Text(std::mem::take(&mut text)));
                }
                tokens.push(token);
                rest = &rest[end + 1..];
            }
            '&' => {
                let decoded = [
                    ("&lt;", '<'),
                    ("&gt;", '>'),
                    ("&amp;", '&'),
                    ("&quot;", '"'),
                    ("&apos;", '\''),
                ]
                .into_iter()
                .find(|(entity, _)| rest.starts_with(entity));
                if let Some((entity, value)) = decoded {
                    text.push(value);
                    rest = &rest[entity.len()..];
                } else {
                    text.push('&');
                    rest = &rest[1..];
                }
            }
            other => {
                text.push(other);
                rest = &rest[other.len_utf8()..];
            }
        }
    }
    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
    Ok(tokens)
}

fn parse_markup(markup: &str) -> Option<Token> {
    let markup = markup.trim();
    match markup {
        "/g" => return Some(Token::Close),
        "b" => return Some(Token::BranchOpen),
        "/b" => return Some(Token::BranchClose),
        _ => {}
    }
    let (atomic, body) = match markup.strip_suffix('/') {
        Some(body) => (true, body.trim_end()),
        None => (false, markup),
    };
    let (name, attributes) = body.split_once(char::is_whitespace)?;
    let value = attributes
        .trim()
        .strip_prefix("id")?
        .trim_start()
        .strip_prefix('=')?
        .trim();
    let value = value.trim_matches(|character| character == '"' || character == '\'');
    let id: u32 = value.parse().ok()?;
    match (name, atomic) {
        ("x", true) => Some(Token::Atom(id)),
        ("g", false) => Some(Token::Open(id)),
        _ => None,
    }
}

fn build_tree(tokens: Vec<Token>) -> Result<Vec<Item>, TaggedError> {
    // Stack frames: the items of the open container, and for a group, its
    // ID and finished branches.
    enum Frame {
        Container(Vec<Item>),
        Group { id: u32, branches: Vec<Vec<Item>> },
    }
    let mut stack = vec![Frame::Container(Vec::new())];
    for token in tokens {
        match token {
            Token::Text(text) => match stack.last_mut() {
                Some(Frame::Container(items)) => items.push(Item::Text(text)),
                Some(Frame::Group { id, .. }) => {
                    if !text.trim().is_empty() {
                        return Err(TaggedError::new(format!(
                            "text inside tag {id} must be inside a <b> branch"
                        )));
                    }
                }
                None => unreachable!("the root frame is never popped"),
            },
            Token::Atom(id) => match stack.last_mut() {
                Some(Frame::Container(items)) => items.push(Item::Atom(id)),
                _ => {
                    return Err(TaggedError::new(format!(
                        "tag {id} must be inside a <b> branch"
                    )));
                }
            },
            Token::Open(id) => {
                if !matches!(stack.last(), Some(Frame::Container(_))) {
                    return Err(TaggedError::new(format!(
                        "tag {id} must be inside a <b> branch"
                    )));
                }
                stack.push(Frame::Group {
                    id,
                    branches: Vec::new(),
                });
            }
            Token::BranchOpen => {
                if !matches!(stack.last(), Some(Frame::Group { .. })) {
                    return Err(TaggedError::new("<b> is only allowed directly inside <g>"));
                }
                stack.push(Frame::Container(Vec::new()));
            }
            Token::BranchClose => {
                let Some(Frame::Container(items)) = stack.pop() else {
                    return Err(TaggedError::new("</b> does not close a <b>"));
                };
                match stack.last_mut() {
                    Some(Frame::Group { branches, .. }) => branches.push(items),
                    _ => return Err(TaggedError::new("</b> does not close a <b>")),
                }
            }
            Token::Close => {
                let Some(Frame::Group { id, branches }) = stack.pop() else {
                    return Err(TaggedError::new(
                        "</g> does not close a <g>; close <b> first",
                    ));
                };
                match stack.last_mut() {
                    Some(Frame::Container(items)) => items.push(Item::Group { id, branches }),
                    _ => return Err(TaggedError::new("</g> does not close a <g>")),
                }
            }
        }
    }
    match (stack.pop(), stack.is_empty()) {
        (Some(Frame::Container(items)), true) => Ok(items),
        _ => Err(TaggedError::new("a <g> or <b> is not closed")),
    }
}

fn check_container(
    model: &SourceModel,
    items: &[Item],
    container: Container,
    errors: &mut Vec<TaggedError>,
) {
    let mut counts = std::collections::BTreeMap::<u32, usize>::new();
    let mut formatting_order = Vec::new();
    for item in items {
        let (id, group) = match item {
            Item::Text(_) => continue,
            Item::Atom(id) => (*id, None),
            Item::Group { id, branches } => (*id, Some(branches)),
        };
        let Some(construct) = model.construct(id) else {
            errors.push(TaggedError::new(format!(
                "tag {id} does not exist in the source"
            )));
            continue;
        };
        if construct.container != container {
            errors.push(TaggedError::new(format!(
                "tag {id} belongs in {}, not {}",
                construct.container.describe(),
                container.describe()
            )));
            continue;
        }
        *counts.entry(id).or_default() += 1;
        if construct.tag.family == Some(SemanticFamily::FormattingPresentation) {
            formatting_order.push(id);
        }
        match (group, construct.tag.branches) {
            (None, 0) => {}
            (None, _) => errors.push(TaggedError::new(format!(
                "tag {id} has branches and must be written <g id=\"{id}\"> with {} <b> branches",
                construct.tag.branches
            ))),
            (Some(_), 0) => errors.push(TaggedError::new(format!(
                "tag {id} has no branches and must be written <x id=\"{id}\"/>"
            ))),
            (Some(branches), expected) => {
                if branches.len() == expected {
                    for (index, branch) in branches.iter().enumerate() {
                        check_container(
                            model,
                            branch,
                            Container::Branch { group: id, index },
                            errors,
                        );
                    }
                } else {
                    errors.push(TaggedError::new(format!(
                        "tag {id} needs exactly {expected} <b> branches, found {}",
                        branches.len()
                    )));
                }
            }
        }
    }
    let mut expected_formatting = Vec::new();
    for construct in model
        .constructs
        .iter()
        .filter(|construct| construct.container == container)
    {
        let id = construct.tag.id;
        match counts.get(&id).copied().unwrap_or(0) {
            0 => errors.push(TaggedError::new(format!(
                "tag {id} ({}) is missing; every tag must be kept",
                construct.tag.spelling
            ))),
            1 => {}
            _ if construct.tag.repeatable => {}
            _ => errors.push(TaggedError::new(format!("tag {id} may appear only once"))),
        }
        if construct.tag.family == Some(SemanticFamily::FormattingPresentation) {
            expected_formatting.push(id);
        }
    }
    if errors.is_empty() && formatting_order != expected_formatting {
        errors.push(TaggedError::new(format!(
            "formatting tags must keep their source order {expected_formatting:?}; found {formatting_order:?}"
        )));
    }
}

fn escape_macro_text(text: &str, in_argument: bool, output: &mut String) {
    for character in text.chars() {
        let special = matches!(character, '\\' | '<')
            || (in_argument && matches!(character, '[' | ']' | '(' | ')' | ',' | '>'));
        if special {
            output.push('\\');
        }
        output.push(character);
    }
}

fn emit(model: &SourceModel, items: &[Item], in_argument: bool, output: &mut String) {
    for item in items {
        match item {
            Item::Text(text) => escape_macro_text(text, in_argument, output),
            Item::Atom(id) => {
                if let Some(construct) = model.construct(*id) {
                    output.push_str(&construct.tag.spelling);
                }
            }
            Item::Group { id, branches } => {
                let Some(construct) = model.construct(*id) else {
                    continue;
                };
                let mut cursor = construct.span.start();
                for (span, branch) in construct.branch_spans.iter().zip(branches) {
                    output.push_str(
                        model
                            .document
                            .slice(Span::new(cursor, span.start()))
                            .unwrap_or_default(),
                    );
                    emit(model, branch, true, output);
                    cursor = span.end();
                }
                output.push_str(
                    model
                        .document
                        .slice(Span::new(cursor, construct.span.end()))
                        .unwrap_or_default(),
                );
            }
        }
    }
}

/// Rebuilds a target macro string from a tagged translation of `source`.
///
/// # Errors
///
/// Returns every violated rule, written for the model that produced the
/// translation. Nothing is rebuilt unless the tags and the rebuilt syntax
/// both satisfy the structure policy.
pub fn rebuild(source: &str, tagged: &str) -> Result<String, Vec<TaggedError>> {
    let (model, _) = SourceModel::build(source).map_err(|error| vec![error])?;
    let items = tokenize(tagged)
        .and_then(build_tree)
        .map_err(|error| vec![error])?;
    let mut errors = Vec::new();
    check_container(&model, &items, Container::Root, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut target = String::new();
    emit(&model, &items, false, &mut target);
    check_assisted_structure(source, &target)?;
    Ok(target)
}

/// Checks a target macro string against the structure policy for assisted
/// translation, on the syntax trees alone.
///
/// # Errors
///
/// Returns every violated rule.
pub fn check_assisted_structure(source: &str, target: &str) -> Result<(), Vec<TaggedError>> {
    let source_analysis = parse(source).semantic_analysis();
    let target_document = parse(target);
    if !target_document.is_well_formed() {
        return Err(vec![TaggedError::new(
            "the translation is not a well-formed macro string",
        )]);
    }
    if !source_analysis.structure().is_comparable() {
        return Err(vec![TaggedError::new("the source string is malformed")]);
    }
    let target_analysis = target_document.semantic_analysis();
    let mut errors = Vec::new();
    check_nodes(
        source_analysis.structure().nodes(),
        target_analysis.structure().nodes(),
        "the top level",
        &mut errors,
    );
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn check_nodes(
    source: &[ProtectedNode],
    target: &[ProtectedNode],
    place: &str,
    errors: &mut Vec<TaggedError>,
) {
    let source: Vec<&ProtectedNode> = source.iter().filter(|node| !is_text(node)).collect();
    let target: Vec<&ProtectedNode> = target.iter().filter(|node| !is_text(node)).collect();
    let source_shapes: Vec<ProtectedNodeKind> =
        source.iter().map(|node| shape(&node.kind)).collect();
    let target_shapes: Vec<ProtectedNodeKind> =
        target.iter().map(|node| shape(&node.kind)).collect();

    let before = errors.len();
    for (index, shape) in source_shapes.iter().enumerate() {
        if source_shapes[..index].contains(shape) {
            continue;
        }
        let source_count = source_shapes
            .iter()
            .filter(|candidate| *candidate == shape)
            .count();
        let target_count = target_shapes
            .iter()
            .filter(|candidate| *candidate == shape)
            .count();
        if target_count < source_count {
            errors.push(TaggedError::new(format!(
                "a construct of {place} is missing: {}",
                describe(source[index])
            )));
        } else if target_count > source_count && !is_repeatable(shape) {
            errors.push(TaggedError::new(format!(
                "a construct of {place} appears more often than in the source: {}",
                describe(source[index])
            )));
        }
    }
    for (index, shape) in target_shapes.iter().enumerate() {
        if !source_shapes.contains(shape) && !target_shapes[..index].contains(shape) {
            errors.push(TaggedError::new(format!(
                "{place} contains a construct that is not in the source: {}",
                describe(target[index])
            )));
        }
    }
    if errors.len() > before {
        return;
    }
    let formatting = |shapes: &[ProtectedNodeKind]| -> Vec<ProtectedNodeKind> {
        shapes
            .iter()
            .filter(|shape| family(shape) == Some(SemanticFamily::FormattingPresentation))
            .cloned()
            .collect()
    };
    if formatting(&source_shapes) != formatting(&target_shapes) {
        errors.push(TaggedError::new(format!(
            "formatting constructs of {place} must keep their source order"
        )));
    }
    check_branches(&source, &target, &source_shapes, &target_shapes, errors);
}

/// Compares branch contents between the n-th occurrences of each construct
/// with branches.
fn check_branches(
    source: &[&ProtectedNode],
    target: &[&ProtectedNode],
    source_shapes: &[ProtectedNodeKind],
    target_shapes: &[ProtectedNodeKind],
    errors: &mut Vec<TaggedError>,
) {
    for (index, node) in source.iter().enumerate() {
        let ProtectedNodeKind::Macro {
            name, arguments, ..
        } = &node.kind
        else {
            continue;
        };
        if !arguments.iter().any(is_branch) {
            continue;
        }
        let occurrence = source_shapes[..index]
            .iter()
            .filter(|candidate| **candidate == source_shapes[index])
            .count();
        let Some(target_node) = target_shapes
            .iter()
            .enumerate()
            .filter(|(_, candidate)| **candidate == source_shapes[index])
            .nth(occurrence)
            .map(|(position, _)| target[position])
        else {
            continue;
        };
        let ProtectedNodeKind::Macro {
            arguments: target_arguments,
            ..
        } = &target_node.kind
        else {
            continue;
        };
        for (branch, (source_argument, target_argument)) in
            arguments.iter().zip(target_arguments).enumerate()
        {
            if let (
                ProtectedExpressionKind::String {
                    nodes: source_nodes,
                },
                ProtectedExpressionKind::String {
                    nodes: target_nodes,
                },
            ) = (&source_argument.kind, &target_argument.kind)
            {
                check_nodes(
                    source_nodes,
                    target_nodes,
                    &format!("argument {} of <{}>", branch + 1, name.as_str()),
                    errors,
                );
            }
        }
    }
}

fn is_text(node: &ProtectedNode) -> bool {
    matches!(node.kind, ProtectedNodeKind::TextSlot)
}

fn is_branch(argument: &ProtectedExpression) -> bool {
    matches!(argument.kind, ProtectedExpressionKind::String { .. })
}

fn family(shape: &ProtectedNodeKind) -> Option<SemanticFamily> {
    match shape {
        ProtectedNodeKind::Macro { family, .. } => Some(*family),
        _ => None,
    }
}

fn is_repeatable(shape: &ProtectedNodeKind) -> bool {
    matches!(shape, ProtectedNodeKind::Macro { family: SemanticFamily::RuntimeContextValue, arguments, .. } if !arguments.iter().any(is_branch))
}

fn describe(node: &ProtectedNode) -> String {
    match &node.kind {
        ProtectedNodeKind::Macro { name, .. } => format!("<{}>", name.as_str()),
        ProtectedNodeKind::Opaque { .. } => "a protected construct".to_owned(),
        ProtectedNodeKind::Malformed { spelling } => format!("malformed `{spelling}`"),
        ProtectedNodeKind::TextSlot => "text".to_owned(),
    }
}

/// The comparable identity of a construct: spans are dropped and the content
/// of translatable branches is masked.
fn shape(kind: &ProtectedNodeKind) -> ProtectedNodeKind {
    match kind {
        ProtectedNodeKind::Macro {
            name,
            family,
            arguments,
        } => ProtectedNodeKind::Macro {
            name: *name,
            family: *family,
            arguments: arguments
                .iter()
                .map(|argument| shape_expression(argument, true))
                .collect(),
        },
        other => other.clone(),
    }
}

fn shape_expression(expression: &ProtectedExpression, mask_strings: bool) -> ProtectedExpression {
    let kind = match &expression.kind {
        ProtectedExpressionKind::String { .. } if mask_strings => {
            ProtectedExpressionKind::String { nodes: Vec::new() }
        }
        ProtectedExpressionKind::String { nodes } => ProtectedExpressionKind::String {
            nodes: nodes
                .iter()
                .filter(|node| !is_text(node))
                .map(|node| ProtectedNode {
                    span: Span::new(0, 0),
                    kind: shape(&node.kind),
                })
                .collect(),
        },
        ProtectedExpressionKind::RuntimeParameter { operator, operand } => {
            ProtectedExpressionKind::RuntimeParameter {
                operator: *operator,
                operand: Box::new(shape_expression(operand, false)),
            }
        }
        ProtectedExpressionKind::Comparison {
            operator,
            left,
            right,
        } => ProtectedExpressionKind::Comparison {
            operator: *operator,
            left: Box::new(shape_expression(left, false)),
            right: Box::new(shape_expression(right, false)),
        },
        other => other.clone(),
    };
    ProtectedExpression {
        span: Span::new(0, 0),
        kind,
    }
}
