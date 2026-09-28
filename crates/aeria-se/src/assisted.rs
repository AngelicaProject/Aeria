//! Macro text written by the translation agent, and the structure policy it
//! must satisfy.
//!
//! The agent reads and writes macro text, as people do in the editor's code
//! mode, with every construct of the source explained ([`constructs`]). It
//! may restructure a translation as its language needs; the policy checked
//! here keeps what the game fills in:
//!
//! - every piece of game data of the source stays: runtime values such as
//!   `<num $n1>` or `<string $gs1>`, game data references such as `<sheet …>`,
//!   icons, sounds, constructs Aeria does not understand, and raw bytes. They
//!   may move and repeat, and the translation adds none the source lacks;
//! - the source's formatting stays, as many times as in the source, in any
//!   order;
//! - conditions such as `<if>`, `<switch>`, and `<if-gender>` may be added,
//!   dropped, and restructured, as long as they test only values the source
//!   uses or globals whose meaning is established (`catalog::GLOBALS`);
//! - line breaks, spaces, and text transforms such as `<capitalize>` are free;
//! - a speaker name (see [`crate::speaker`]) stays exactly when the source has
//!   one.

use std::collections::HashSet;
use std::fmt::Write as _;

use crate::bytes::Expr;
use crate::catalog::{self, MacroSpec, NULLARY, PARAMETERS, Role, SemanticFamily};
use crate::semantic::family;
use crate::speaker::{SPEAKER_CLOSE, SPEAKER_OPEN, speaker_name};
use crate::syntax::{
    ExprKind, ExprSyntax, MacroString, MacroSyntax, Span, SyntaxKind, SyntaxNode, Written, parse,
};

/// Longest construct spelling shown in a legend.
const LEGEND_SPELLING_CHARS: usize = 160;

/// Layout macros a translation may add or drop.
const FREE_LAYOUT: &[&str] = &["br", "nbsp", "shy", "hyphen"];

/// A reason a translation's structure was refused, written for the model that
/// produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureError {
    pub message: String,
}

impl StructureError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// What a translation may do with a construct of the source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstructRule {
    /// Game data: it stays, and may move or repeat.
    Keep,
    /// Formatting: it stays as often as in the source, in any order.
    Formatting,
    /// A condition: it may be reworded, restructured, added, or dropped.
    Condition,
    /// Layout or a text transform: it may be added or dropped.
    Free,
}

/// One construct of a source string, explained for the model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Construct {
    /// The construct's macro text.
    pub spelling: String,
    /// What it does, with its argument values.
    pub description: String,
    pub family: Option<SemanticFamily>,
    pub rule: ConstructRule,
}

impl Construct {
    /// A one-line explanation for the model.
    #[must_use]
    pub fn legend(&self) -> String {
        let mut spelling: String = self.spelling.chars().take(LEGEND_SPELLING_CHARS).collect();
        if spelling.len() < self.spelling.len() {
            spelling.push('…');
        }
        let rule = match self.rule {
            ConstructRule::Keep => "game data: keep it; it may move or repeat",
            ConstructRule::Formatting => "formatting: keep it as often as the source has it",
            ConstructRule::Condition => {
                "condition: may be reworded, restructured, added, or dropped"
            }
            ConstructRule::Free => "may be added or dropped",
        };
        format!("{spelling} — {} ({rule})", self.description)
    }
}

/// How a translation may treat a macro.
fn rule(syntax: &MacroSyntax) -> ConstructRule {
    let Some(spec) = syntax.spec else {
        return ConstructRule::Keep;
    };
    match spec.family {
        SemanticFamily::ConditionalSelection => ConstructRule::Condition,
        SemanticFamily::FormattingPresentation => ConstructRule::Formatting,
        SemanticFamily::LayoutTextualControl if FREE_LAYOUT.contains(&spec.name) => {
            ConstructRule::Free
        }
        // `<capitalize>…</capitalize>` only changes its text; `<string $gs1>`
        // shows a value.
        SemanticFamily::TranslatableText
            if (0..syntax.args.len()).all(|index| spec.is_translatable_arg(index)) =>
        {
            ConstructRule::Free
        }
        _ => ConstructRule::Keep,
    }
}

/// Every construct of `source`, each spelling once, in source order.
///
/// # Errors
///
/// Returns an error for a malformed source, which cannot be translated with
/// assistance.
pub fn constructs(source: &str) -> Result<Vec<Construct>, StructureError> {
    let document = parse(source);
    if !document.is_well_formed() {
        return Err(StructureError::new(
            "the source string is malformed and cannot be translated with assistance",
        ));
    }
    let mut found = Vec::new();
    list(&document, document.nodes(), &mut found);
    let mut seen = HashSet::new();
    found.retain(|construct: &Construct| seen.insert(construct.spelling.clone()));
    Ok(found)
}

fn list(document: &MacroString, nodes: &[SyntaxNode], found: &mut Vec<Construct>) {
    for node in nodes {
        let SyntaxKind::Macro(syntax) = &node.kind else {
            continue;
        };
        let spelling = document.slice(node.span).unwrap_or_default();
        found.push(Construct {
            spelling: spelling.to_owned(),
            description: catalog::idiom(spelling)
                .map_or_else(|| describe(syntax), |idiom| idiom.summary.to_owned()),
            family: family(node),
            rule: rule(syntax),
        });
        for arg in &syntax.args {
            if let ExprKind::Str(inner) = &arg.kind {
                list(document, inner, found);
            }
        }
    }
}

/// The syntax of conditions and the globals whose meaning is established,
/// for the model's instructions.
#[must_use]
pub fn authoring_reference() -> String {
    let mut reference = String::from(
        "Macro text: text with tags. A condition is <if (left op right)>then<else>otherwise</if> \
with ==, !=, <, <=, >, or >=, or <if $value>…<else>…</if> for a value that is not zero or empty; \
<switch $value><case>for 1<case>for 2</switch> chooses by position. A branch that is a number \
is written in braces, such as {240}. Write \\< \\{ \\\\ for a literal <, {, or \\. \
Values a condition may test besides those the source uses:",
    );
    for global in catalog::GLOBALS {
        let _ = write!(
            reference,
            " ${}{} is {};",
            global.prefix, global.index, global.summary
        );
    }
    reference.pop();
    reference.push('.');
    reference
}

// ---------------------------------------------------------------------------
// Descriptions

/// What a macro does, with its argument values, for people and the model.
#[must_use]
pub fn describe(syntax: &MacroSyntax) -> String {
    let Some(spec) = syntax.spec else {
        return format!(
            "macro code {:#04X}, which Aeria does not know; keep it exactly",
            syntax.code
        );
    };
    if syntax.written == Written::Close {
        return format!("end of: {}", spec.summary);
    }
    let details: Vec<String> = syntax
        .args
        .iter()
        .enumerate()
        .filter(|(index, _)| !spec.is_translatable_arg(*index))
        .filter(|_| syntax.written != Written::Open || !is_implied(spec, &syntax.args))
        .filter_map(|(index, arg)| {
            let described = spec.arg(index)?;
            Some(format!(
                "{} = {}",
                described.name,
                describe_value(arg, described.role)
            ))
        })
        .collect();
    if details.is_empty() {
        spec.summary.to_owned()
    } else {
        format!("{}; {}", spec.summary, details.join(", "))
    }
}

fn is_implied(spec: &MacroSpec, args: &[ExprSyntax]) -> bool {
    matches!(
        (spec.form, args),
        (catalog::Form::Pair { implied: Some(value), .. }, [ExprSyntax { kind: ExprKind::Int(actual), .. }])
            if *actual == value
    )
}

fn describe_value(expr: &ExprSyntax, role: Role) -> String {
    match &expr.kind {
        ExprKind::Int(value) if role == Role::Color => format!("#{value:08X}"),
        ExprKind::Int(value) => value.to_string(),
        ExprKind::Str(nodes) => match nodes.as_slice() {
            [] => "empty text".to_owned(),
            [
                SyntaxNode {
                    kind: SyntaxKind::Text(text),
                    ..
                },
            ] => text.clone(),
            _ => "text with macros".to_owned(),
        },
        ExprKind::Nullary(kind) => NULLARY
            .iter()
            .find(|spec| spec.code == *kind)
            .map_or_else(|| "a game value".to_owned(), |spec| spec.summary.to_owned()),
        ExprKind::Param(kind, operand) => {
            let parameter = PARAMETERS.iter().find(|spec| spec.code == *kind);
            let summary = parameter.map_or("parameter", |spec| spec.summary);
            match operand.kind {
                ExprKind::Int(number) => {
                    let meaning = parameter
                        .and_then(|spec| catalog::global(spec.prefix, number))
                        .map(|global| format!(" ({})", global.summary))
                        .unwrap_or_default();
                    format!("{summary} {number}{meaning}")
                }
                _ => format!("{summary} ({})", describe_value(operand, Role::Other)),
            }
        }
        ExprKind::Compare(kind, left, right) => {
            let operator = catalog::COMPARISONS
                .iter()
                .find(|spec| spec.code == *kind)
                .map_or("?", |spec| spec.operator);
            format!(
                "{} {operator} {}",
                describe_value(left, Role::Other),
                describe_value(right, Role::Other)
            )
        }
        ExprKind::Raw(_) | ExprKind::Error => "an invalid value".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Structure policy

/// The comparable identity of a construct: its code and arguments, with the
/// content of translatable branches masked.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Shape {
    Macro { code: u8, args: Vec<Option<Expr>> },
    Raw(Vec<u8>),
}

fn shape(syntax: &MacroSyntax) -> Shape {
    Shape::Macro {
        code: syntax.code,
        args: syntax
            .args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                let translatable = syntax
                    .spec
                    .is_some_and(|spec| spec.is_translatable_arg(index));
                if translatable && matches!(arg.kind, ExprKind::Str(_)) {
                    None
                } else {
                    crate::syntax::expr_to_model(arg)
                }
            })
            .collect(),
    }
}

/// What the policy compares in one string.
#[derive(Default)]
struct Facts<'a> {
    data: Vec<(Shape, &'a str)>,
    formatting: Vec<(Shape, &'a str)>,
    /// Conditions, with the parameters their own arguments test.
    conditions: Vec<(&'a str, Vec<(u8, u32)>)>,
    /// Every parameter the string uses, as `(type byte, index)`.
    parameters: HashSet<(u8, u32)>,
}

fn collect<'a>(document: &'a MacroString, nodes: &'a [SyntaxNode], facts: &mut Facts<'a>) {
    for node in nodes {
        let spelling = document.slice(node.span).unwrap_or_default();
        match &node.kind {
            SyntaxKind::Text(_) | SyntaxKind::Error => {}
            SyntaxKind::Raw(bytes) => facts.data.push((Shape::Raw(bytes.clone()), spelling)),
            SyntaxKind::Macro(syntax) => {
                let mut tested = Vec::new();
                for (index, arg) in syntax.args.iter().enumerate() {
                    let translatable = syntax
                        .spec
                        .is_some_and(|spec| spec.is_translatable_arg(index));
                    if !(translatable && matches!(arg.kind, ExprKind::Str(_))) {
                        parameters(arg, &mut tested);
                    }
                }
                facts.parameters.extend(tested.iter().copied());
                match rule(syntax) {
                    ConstructRule::Keep => facts.data.push((shape(syntax), spelling)),
                    ConstructRule::Formatting => facts.formatting.push((shape(syntax), spelling)),
                    ConstructRule::Condition => facts.conditions.push((spelling, tested)),
                    ConstructRule::Free => {}
                }
                // Game data inside a kept construct is part of its shape;
                // anything else nested is compared on its own.
                for (index, arg) in syntax.args.iter().enumerate() {
                    let translatable = syntax
                        .spec
                        .is_some_and(|spec| spec.is_translatable_arg(index));
                    if let ExprKind::Str(inner) = &arg.kind
                        && (translatable || rule(syntax) != ConstructRule::Keep)
                    {
                        collect(document, inner, facts);
                    }
                }
            }
        }
    }
}

/// The parameters an expression tests, outside macros nested in its strings.
fn parameters(expr: &ExprSyntax, found: &mut Vec<(u8, u32)>) {
    match &expr.kind {
        ExprKind::Param(kind, operand) => match operand.kind {
            ExprKind::Int(index) => found.push((*kind, index)),
            _ => parameters(operand, found),
        },
        ExprKind::Compare(_, left, right) => {
            parameters(left, found);
            parameters(right, found);
        }
        _ => {}
    }
}

fn parameter_text(kind: u8, index: u32) -> String {
    let prefix = PARAMETERS
        .iter()
        .find(|spec| spec.code == kind)
        .map_or("?", |spec| spec.prefix);
    format!("${prefix}{index}")
}

fn is_known_global(kind: u8, index: u32) -> bool {
    PARAMETERS
        .iter()
        .find(|spec| spec.code == kind)
        .is_some_and(|spec| catalog::global(spec.prefix, index).is_some())
}

/// Checks macro text the agent wrote for `source` against the structure
/// policy.
///
/// # Errors
///
/// Returns every violated rule, written for the model.
pub fn check_assisted_structure(source: &str, target: &str) -> Result<(), Vec<StructureError>> {
    let source_document = parse(source);
    let target_document = parse(target);
    if !target_document.is_well_formed() {
        let mut errors = vec![StructureError::new(
            "the translation is not well-formed macro text",
        )];
        errors.extend(
            target_document
                .diagnostics()
                .iter()
                .map(|diagnostic| StructureError::new(diagnostic.message.clone())),
        );
        return Err(errors);
    }
    if !source_document.is_well_formed() {
        return Err(vec![StructureError::new("the source string is malformed")]);
    }
    let mut source_facts = Facts::default();
    collect(&source_document, source_document.nodes(), &mut source_facts);
    let mut target_facts = Facts::default();
    collect(&target_document, target_document.nodes(), &mut target_facts);

    let mut errors = Vec::new();
    check_speaker_name(&source_document, &target_document, &mut errors);
    for (index, (shape, spelling)) in source_facts.data.iter().enumerate() {
        let first = !source_facts.data[..index]
            .iter()
            .any(|(earlier, _)| earlier == shape);
        if first
            && !target_facts
                .data
                .iter()
                .any(|(candidate, _)| candidate == shape)
        {
            errors.push(StructureError::new(format!(
                "{spelling} of the source is missing; every value the game fills in must stay, though it may move or repeat"
            )));
        }
    }
    for (index, (shape, spelling)) in target_facts.data.iter().enumerate() {
        let first = !target_facts.data[..index]
            .iter()
            .any(|(earlier, _)| earlier == shape);
        if first
            && !source_facts
                .data
                .iter()
                .any(|(candidate, _)| candidate == shape)
        {
            errors.push(StructureError::new(format!(
                "{spelling} is not in the source; a translation may not add game data, only reuse the source's"
            )));
        }
    }
    let count = |facts: &Facts<'_>, shape: &Shape| {
        facts
            .formatting
            .iter()
            .filter(|(candidate, _)| candidate == shape)
            .count()
    };
    for (index, (shape, spelling)) in source_facts
        .formatting
        .iter()
        .chain(&target_facts.formatting)
        .enumerate()
    {
        let earlier = source_facts
            .formatting
            .iter()
            .chain(&target_facts.formatting)
            .take(index)
            .any(|(candidate, _)| candidate == shape);
        let (expected, found) = (count(&source_facts, shape), count(&target_facts, shape));
        if !earlier && expected != found {
            errors.push(StructureError::new(format!(
                "{spelling} appears {expected}× in the source and {found}× in the translation; keep the source's formatting, in any order"
            )));
        }
    }
    for (spelling, tested) in &target_facts.conditions {
        for (kind, index) in tested {
            if !source_facts.parameters.contains(&(*kind, *index))
                && !is_known_global(*kind, *index)
            {
                errors.push(StructureError::new(format!(
                    "{spelling} tests {}, which the source does not use and whose meaning is not known; a condition may test the source's values or the known globals",
                    parameter_text(*kind, *index)
                )));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// A translation starts with a speaker name exactly when the source does,
/// and a name the source gives is not left empty.
fn check_speaker_name(
    source: &MacroString,
    target: &MacroString,
    errors: &mut Vec<StructureError>,
) {
    match (speaker_name(source), speaker_name(target)) {
        (Some(expected), None) => errors.push(StructureError::new(format!(
            "the translation must start with a speaker name written {SPEAKER_OPEN}name{SPEAKER_CLOSE}, as the source does: {}",
            source
                .slice(Span::new(expected.open.start(), expected.close.end()))
                .unwrap_or_default()
        ))),
        (None, Some(found)) => errors.push(StructureError::new(format!(
            "the translation starts with {}, which the game shows as a speaker name, but the source has none",
            target
                .slice(Span::new(found.open.start(), found.close.end()))
                .unwrap_or_default()
        ))),
        (Some(expected), Some(found)) if !expected.name.is_empty() && found.name.is_empty() => {
            errors.push(StructureError::new(format!(
                "the speaker name between {SPEAKER_OPEN} and {SPEAKER_CLOSE} is empty; the source's is {}",
                source.slice(expected.name).unwrap_or_default()
            )));
        }
        _ => {}
    }
}
