//! A preview of macro text as the game shows it.
//!
//! [`preview`] interprets parsed macro text into pieces: styled text, line
//! breaks, values, icons, and the branches conditions select. Formatting
//! follows the game's stacks: `<color>` pushes a color and `</color>`
//! restores the previous one. Game data comes from a [`PreviewData`] source,
//! such as the installed game; without it, references are shown as values.
//!
//! Strings read variables: parameters such as `$n1`, global parameters such
//! as `$gn68`, time values such as `$hour`, and properties of a character.
//! The preview lists every variable a string reads and evaluates the string
//! with one set of [`Values`], as the game does: conditions select their
//! branches, numbers and texts are filled in, and rows named by a variable
//! are looked up. A variable without a value takes a default that shows the
//! first branch of the first condition reading it.
//!
//! The preview approximates the game. It never guesses a value it cannot
//! evaluate: such a value is shown as its code, and such a condition keeps
//! every branch.

use std::collections::BTreeMap;

use crate::catalog::{self, Close, Form, GlobalSpec, PARAMETERS};
use crate::syntax::{
    ExprKind, ExprSyntax, MacroString, MacroSyntax, SyntaxKind, SyntaxNode, Written, parse,
};

/// Deepest nesting of game data text rendered inside a preview.
const MAX_REFERENCE_DEPTH: usize = 2;
/// Most rows offered for a variable whose values are rows of a sheet.
const MAX_OPTIONS: usize = 200;

/// Game data a preview reads.
pub trait PreviewData {
    /// The foreground color of a `UIColor` row as `0xRRGGBBAA`.
    fn ui_color(&self, row: u32) -> Option<u32>;
    /// The macro text of a string cell of a sheet in the source language.
    fn sheet_text(&self, sheet: &str, row: u32, column: u32) -> Option<String>;
    /// The row ids of a sheet, in order.
    fn sheet_rows(&self, _sheet: &str) -> Vec<u32> {
        Vec::new()
    }
}

/// No game data: every reference is shown as a value.
pub struct NoData;

impl PreviewData for NoData {
    fn ui_color(&self, _row: u32) -> Option<u32> {
        None
    }

    fn sheet_text(&self, _sheet: &str, _row: u32, _column: u32) -> Option<String> {
        None
    }
}

/// A color as `0xRRGGBBAA`.
pub type Rgba = u32;

/// How text looks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Style {
    pub color: Option<Rgba>,
    /// The outline color.
    pub edge: Option<Rgba>,
    pub italic: bool,
    pub bold: bool,
}

/// What a runtime value is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueKind {
    Number,
    Text,
    PlayerName,
    GameData,
    Time,
    Other,
}

/// A parameter a value reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Parameter {
    /// `n`, `s`, `gn`, or `gs`.
    pub prefix: &'static str,
    pub index: u32,
}

impl Parameter {
    /// The variable key, such as `gn68`.
    #[must_use]
    pub fn key(self) -> String {
        format!("{}{}", self.prefix, self.index)
    }

    fn is_text(self) -> bool {
        matches!(self.prefix, "s" | "gs")
    }
}

/// The value of a variable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Int(u32),
    Text(String),
}

/// Values of variables by key: a parameter such as `n1`, `gn68`, or `gs1`;
/// a time value such as `hour`; or a character property: `gender` (1 for
/// female), `self` (1 when the character is the reading player), `name` (1
/// when the name matches), or `josa` (1 after a vowel).
pub type Values = BTreeMap<String, Value>;

/// What kind of variable a string reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VariableKind {
    Number,
    Text,
    Time,
    /// A property of a character: `gender`, `self`, `name`, or `josa`.
    Character,
}

/// A variable a string reads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Variable {
    pub key: String,
    pub kind: VariableKind,
    pub parameter: Option<Parameter>,
    /// The established meaning of a global parameter.
    pub global: Option<&'static GlobalSpec>,
    /// The sheet and column the value names a row of.
    pub sheet: Option<(String, u32)>,
    /// The value that shows the first branch of the first condition reading
    /// the variable.
    pub default: Value,
    /// The value the preview used: the given one, else the default.
    pub value: Value,
    /// The text of row `value` of `sheet`.
    pub value_name: Option<String>,
    /// The rows to choose from when the value is a row of a small sheet.
    pub options: Vec<(u32, String)>,
}

/// What decides between the branches of a choice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChoiceKind {
    /// `<if>`: the condition as macro text; the second branch is "otherwise".
    If { condition: String },
    /// `<switch>`: the value as macro text; branch `n` is for value `n + 1`.
    Switch { value: String },
    /// `<if-gender>`: male, then female.
    Gender,
    /// `<if-self>`: the reading player, then another character.
    Myself,
    /// `<if-name>`: the name matches, then otherwise.
    Name,
    /// `<josa>` and `<josa-ro>`: after a final consonant, then a vowel.
    Josa,
}

/// One piece of a preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Piece {
    Text {
        text: String,
        style: Style,
    },
    Break,
    /// A value supplied at runtime, such as a parameter or a game data
    /// name.
    Value {
        kind: ValueKind,
        /// The macro name that produces it, such as `num`.
        source: &'static str,
        parameter: Option<Parameter>,
        /// A short label in macro text, such as `$n1` or `Item $n1 0`.
        label: String,
        style: Style,
        /// The value filled in from the variables; empty when it is not
        /// known, and the label is shown instead.
        shown: Vec<Piece>,
    },
    Icon {
        icon: u32,
        /// Whether the icon depends on the input device.
        device: bool,
    },
    /// A choice between branches. When the variables select a branch,
    /// `selected` names it and only that branch holds pieces; otherwise
    /// every branch is kept.
    Choice {
        kind: ChoiceKind,
        selected: Option<usize>,
        branches: Vec<Vec<Piece>>,
    },
    Ruby {
        base: Vec<Piece>,
        reading: Vec<Piece>,
    },
    /// A construct shown as its macro text, such as a key prompt or an
    /// unknown macro.
    Opaque {
        spelling: String,
    },
}

/// A preview and the variables it read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preview {
    pub pieces: Vec<Piece>,
    pub variables: Vec<Variable>,
}

/// Renders macro text as a preview with the given variable values.
/// Diagnostics do not stop the preview; the parts that could not be parsed
/// are left out.
#[must_use]
pub fn preview(text: &str, data: &dyn PreviewData, values: &Values) -> Preview {
    let document = parse(text);
    let mut variables = Variables::default();
    variables.collect(document.nodes());
    let mut renderer = Renderer {
        data,
        document: &document,
        values,
        variables: &mut variables,
        state: State::default(),
        depth: 0,
    };
    let pieces = renderer.nodes(document.nodes());
    Preview {
        pieces,
        variables: variables.finish(data, values),
    }
}

// ---------------------------------------------------------------------------
// Variables

/// A value a condition reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operand {
    Int(u32),
    Parameter(Parameter),
    GameValue(&'static str),
    Other,
}

fn operand_of(expr: &ExprSyntax) -> Operand {
    match &expr.kind {
        ExprKind::Int(value) => Operand::Int(*value),
        ExprKind::Nullary(code) => catalog::NULLARY
            .iter()
            .find(|spec| spec.code == *code)
            .map_or(Operand::Other, |spec| Operand::GameValue(spec.name)),
        ExprKind::Param(..) => parameter_of(expr).map_or(Operand::Other, Operand::Parameter),
        _ => Operand::Other,
    }
}

fn parameter_of(expr: &ExprSyntax) -> Option<Parameter> {
    let ExprKind::Param(code, operand) = &expr.kind else {
        return None;
    };
    let ExprKind::Int(index) = operand.kind else {
        return None;
    };
    PARAMETERS
        .iter()
        .find(|spec| spec.code == *code)
        .map(|spec| Parameter {
            prefix: spec.prefix,
            index,
        })
}

fn comparison(code: u8) -> Option<&'static str> {
    catalog::COMPARISONS
        .iter()
        .find(|spec| spec.code == code)
        .map(|spec| spec.operator)
}

/// The value of a variable on the left of `operator` that makes the
/// comparison with `other` hold.
fn satisfying(operator: &str, other: u32) -> u32 {
    match operator {
        ">" => other.saturating_add(1),
        "<" => other.saturating_sub(1),
        "!=" => u32::from(other == 0),
        _ => other,
    }
}

fn mirrored(operator: &'static str) -> &'static str {
    match operator {
        "<" => ">",
        "<=" => ">=",
        ">" => "<",
        ">=" => "<=",
        other => other,
    }
}

/// The value a sheet reference macro reads its row from, and its sheet and
/// column.
fn reference_parts(name: &str, args: &[ExprSyntax]) -> Option<(usize, Option<String>, u32)> {
    let row_index = match name {
        "sheet" | "sheet-sub" => 1,
        "noun-ja" | "noun-en" | "noun-de" | "noun-fr" | "noun-zh" => 2,
        _ => return None,
    };
    let sheet = args.first().and_then(|expr| match &expr.kind {
        ExprKind::Str(nodes) => match nodes.as_slice() {
            [
                SyntaxNode {
                    kind: SyntaxKind::Text(text),
                    ..
                },
            ] => Some(text.clone()),
            _ => None,
        },
        _ => None,
    });
    let column = match (name, args.get(2).map(|arg| &arg.kind)) {
        ("sheet", Some(ExprKind::Int(column))) => *column,
        _ => 0,
    };
    Some((row_index, sheet, column))
}

#[derive(Default)]
struct Variables {
    list: Vec<Variable>,
}

impl Variables {
    fn get(&self, key: &str) -> Option<&Variable> {
        self.list.iter().find(|variable| variable.key == key)
    }

    fn add(
        &mut self,
        key: String,
        kind: VariableKind,
        parameter: Option<Parameter>,
        default: Value,
    ) {
        if self.get(&key).is_some() {
            return;
        }
        self.list.push(Variable {
            key,
            kind,
            parameter,
            global: parameter
                .and_then(|parameter| catalog::global(parameter.prefix, parameter.index)),
            sheet: None,
            value: default.clone(),
            default,
            value_name: None,
            options: Vec::new(),
        });
    }

    fn add_operand(&mut self, operand: Operand, default: Option<u32>) {
        match operand {
            Operand::Parameter(parameter) if parameter.is_text() => self.add(
                parameter.key(),
                VariableKind::Text,
                Some(parameter),
                Value::Text(String::new()),
            ),
            Operand::Parameter(parameter) => self.add(
                parameter.key(),
                VariableKind::Number,
                Some(parameter),
                Value::Int(default.unwrap_or(1)),
            ),
            Operand::GameValue(name) => self.add(
                name.to_owned(),
                VariableKind::Time,
                None,
                Value::Int(default.unwrap_or(1)),
            ),
            Operand::Int(_) | Operand::Other => {}
        }
    }

    /// Records every variable of `nodes` with its default, in order of first
    /// use.
    fn collect(&mut self, nodes: &[SyntaxNode]) {
        for node in nodes {
            if let SyntaxKind::Macro(syntax) = &node.kind {
                self.collect_macro(syntax);
            }
        }
    }

    fn collect_macro(&mut self, syntax: &MacroSyntax) {
        let name = syntax.spec.map_or("", |spec| spec.name);
        let character = match name {
            "if-gender" => Some(("gender", 0)),
            "if-self" => Some(("self", 1)),
            "if-name" => Some(("name", 1)),
            "josa" | "josa-ro" => Some(("josa", 0)),
            _ => None,
        };
        if let Some((key, default)) = character {
            self.add(
                key.to_owned(),
                VariableKind::Character,
                None,
                Value::Int(default),
            );
        }
        if name == "if"
            && let Some(ExprSyntax {
                kind: ExprKind::Compare(code, left, right),
                ..
            }) = syntax.args.first()
            && let Some(operator) = comparison(*code)
        {
            let (left, right) = (operand_of(left), operand_of(right));
            if let Operand::Int(other) = right {
                self.add_operand(left, Some(satisfying(operator, other)));
            }
            if let Operand::Int(other) = left {
                self.add_operand(right, Some(satisfying(mirrored(operator), other)));
            }
        }
        for (index, arg) in syntax.args.iter().enumerate() {
            // The character an object argument names does not change the
            // preview; its properties are character variables.
            let object = syntax
                .spec
                .and_then(|spec| spec.arg(index))
                .is_some_and(|arg| arg.role == catalog::Role::Object);
            if !object {
                self.collect_expr(arg);
            }
        }
        if let Some((row_index, Some(sheet), column)) = reference_parts(name, &syntax.args)
            && let Some(parameter) = syntax.args.get(row_index).and_then(parameter_of)
            && let Some(variable) = self
                .list
                .iter_mut()
                .find(|variable| variable.key == parameter.key())
            && variable.sheet.is_none()
        {
            variable.sheet = Some((sheet, column));
        }
    }

    fn collect_expr(&mut self, expr: &ExprSyntax) {
        match &expr.kind {
            ExprKind::Str(nodes) => self.collect(nodes),
            ExprKind::Param(_, operand) => {
                self.add_operand(operand_of(expr), None);
                self.collect_expr(operand);
            }
            ExprKind::Nullary(_) => self.add_operand(operand_of(expr), None),
            ExprKind::Compare(_, left, right) => {
                self.collect_expr(left);
                self.collect_expr(right);
            }
            ExprKind::Int(_) | ExprKind::Raw(_) | ExprKind::Error => {}
        }
    }

    /// The value of `key`: the given one, else its default.
    fn value(&self, values: &Values, key: &str) -> Option<Value> {
        values
            .get(key)
            .cloned()
            .or_else(|| self.get(key).map(|variable| variable.default.clone()))
    }

    fn finish(mut self, data: &dyn PreviewData, values: &Values) -> Vec<Variable> {
        for variable in &mut self.list {
            if let Some(value) = values.get(&variable.key) {
                variable.value = value.clone();
            }
            if variable.sheet.is_none()
                && let Some(sheet) = variable.global.and_then(|global| global.sheet)
            {
                variable.sheet = Some((sheet.to_owned(), 0));
                variable.options = data
                    .sheet_rows(sheet)
                    .into_iter()
                    .take(MAX_OPTIONS)
                    .filter_map(|row| {
                        let name = row_name(data, sheet, row, 0)?;
                        (!name.is_empty()).then_some((row, name))
                    })
                    .collect();
            }
            if let (Some((sheet, column)), Value::Int(row)) = (&variable.sheet, &variable.value) {
                variable.value_name = row_name(data, sheet, *row, *column);
            }
        }
        self.list
    }
}

fn row_name(data: &dyn PreviewData, sheet: &str, row: u32, column: u32) -> Option<String> {
    data.sheet_text(sheet, row, column)
        .map(|text| parse(&text).plain_text())
}

// ---------------------------------------------------------------------------
// Rendering

#[derive(Clone, Default)]
struct State {
    colors: Vec<Option<Rgba>>,
    edges: Vec<Option<Rgba>>,
    italic: bool,
    bold: bool,
}

impl State {
    fn style(&self) -> Style {
        Style {
            color: self.colors.last().copied().flatten(),
            edge: self.edges.last().copied().flatten(),
            italic: self.italic,
            bold: self.bold,
        }
    }
}

struct Renderer<'a, 'v> {
    data: &'a dyn PreviewData,
    document: &'a MacroString,
    values: &'a Values,
    variables: &'v mut Variables,
    state: State,
    depth: usize,
}

impl Renderer<'_, '_> {
    fn nodes(&mut self, nodes: &[SyntaxNode]) -> Vec<Piece> {
        let mut pieces = Vec::new();
        for node in nodes {
            match &node.kind {
                SyntaxKind::Text(text) => push_text(&mut pieces, text, self.state.style()),
                SyntaxKind::Macro(syntax) => self.macro_node(node, syntax, &mut pieces),
                SyntaxKind::Raw(_) | SyntaxKind::Error => pieces.push(Piece::Opaque {
                    spelling: self.spelling(node),
                }),
            }
        }
        pieces
    }

    fn spelling(&self, node: &SyntaxNode) -> String {
        self.document
            .slice(node.span)
            .unwrap_or_default()
            .to_owned()
    }

    fn expr_text(&self, expr: &ExprSyntax) -> String {
        self.document
            .slice(expr.span)
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    fn operand_value(&self, operand: Operand) -> Option<Value> {
        match operand {
            Operand::Int(value) => Some(Value::Int(value)),
            Operand::Parameter(parameter) => self.variables.value(self.values, &parameter.key()),
            Operand::GameValue(name) => self.variables.value(self.values, name),
            Operand::Other => None,
        }
    }

    fn int(&self, expr: &ExprSyntax) -> Option<u32> {
        match self.operand_value(operand_of(expr))? {
            Value::Int(value) => Some(value),
            Value::Text(_) => None,
        }
    }

    /// Whether a condition holds, when the variables decide it.
    fn holds(&self, expr: &ExprSyntax) -> Option<bool> {
        if let ExprKind::Compare(code, left, right) = &expr.kind {
            let operator = comparison(*code)?;
            let left = self.operand_value(operand_of(left))?;
            let right = self.operand_value(operand_of(right))?;
            return match (left, right) {
                (Value::Int(left), Value::Int(right)) => Some(match operator {
                    "==" => left == right,
                    "!=" => left != right,
                    "<" => left < right,
                    "<=" => left <= right,
                    ">" => left > right,
                    _ => left >= right,
                }),
                (Value::Text(left), Value::Text(right)) => match operator {
                    "==" => Some(left == right),
                    "!=" => Some(left != right),
                    _ => None,
                },
                _ => None,
            };
        }
        match self.operand_value(operand_of(expr))? {
            Value::Int(value) => Some(value != 0),
            Value::Text(text) => Some(!text.is_empty()),
        }
    }

    fn character(&self, key: &str) -> Option<u32> {
        match self.variables.value(self.values, key)? {
            Value::Int(value) => Some(value),
            Value::Text(_) => None,
        }
    }

    /// Renders a branch with the current style, keeping the style for the
    /// text after the choice.
    fn branch(&mut self, expr: &ExprSyntax) -> Vec<Piece> {
        let saved = self.state.clone();
        let pieces = match &expr.kind {
            ExprKind::Str(nodes) => self.nodes(nodes),
            ExprKind::Int(value) => vec![Piece::Text {
                text: value.to_string(),
                style: self.state.style(),
            }],
            _ => vec![self.value(expr, ValueKind::Other, "")],
        };
        self.state = saved;
        pieces
    }

    fn choice(&mut self, name: &str, syntax: &MacroSyntax, pieces: &mut Vec<Piece>) {
        let spec = syntax.spec.expect("a choice macro is named");
        let arg = syntax.args.first();
        let (kind, selected) = match name {
            "if" => (
                ChoiceKind::If {
                    condition: arg.map(|expr| self.expr_text(expr)).unwrap_or_default(),
                },
                arg.and_then(|expr| self.holds(expr))
                    .map(|holds| usize::from(!holds)),
            ),
            "switch" => (
                ChoiceKind::Switch {
                    value: arg.map(|expr| self.expr_text(expr)).unwrap_or_default(),
                },
                // A value without a case shows nothing.
                arg.and_then(|expr| self.int(expr))
                    .map(|value| usize::try_from(value).unwrap_or(usize::MAX).wrapping_sub(1)),
            ),
            "if-gender" => (
                ChoiceKind::Gender,
                self.character("gender")
                    .map(|value| usize::from(value == 1)),
            ),
            "if-self" => (
                ChoiceKind::Myself,
                self.character("self").map(|value| usize::from(value != 1)),
            ),
            "if-name" => (
                ChoiceKind::Name,
                self.character("name").map(|value| usize::from(value != 1)),
            ),
            _ => (
                ChoiceKind::Josa,
                self.character("josa").map(|value| usize::from(value == 1)),
            ),
        };
        let blocks: Vec<&ExprSyntax> = syntax
            .args
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                spec.arg(*index)
                    .is_some_and(|arg| arg.place == catalog::Place::Block)
            })
            .map(|(_, expr)| expr)
            .collect();
        if let Some(selected) = selected
            && selected >= blocks.len()
        {
            return;
        }
        let branches = blocks
            .iter()
            .enumerate()
            .map(|(index, expr)| {
                if selected.is_none_or(|selected| selected == index) {
                    self.branch(expr)
                } else {
                    Vec::new()
                }
            })
            .collect();
        pieces.push(Piece::Choice {
            kind,
            selected,
            branches,
        });
    }

    fn macro_node(&mut self, node: &SyntaxNode, syntax: &MacroSyntax, pieces: &mut Vec<Piece>) {
        let Some(spec) = syntax.spec else {
            pieces.push(Piece::Opaque {
                spelling: self.spelling(node),
            });
            return;
        };
        let args = &syntax.args;
        let arg = |index: usize| args.get(index);
        if let Form::Pair { close, .. } = spec.form {
            self.pair(spec.name, close, syntax);
            return;
        }
        match spec.name {
            "br" => pieces.push(Piece::Break),
            "nbsp" => push_text(pieces, "\u{A0}", self.state.style()),
            "shy" => push_text(pieces, "\u{AD}", self.state.style()),
            "hyphen" => push_text(pieces, "\u{2011}", self.state.style()),
            "wait" | "sound" | "reset-time" | "set-time" => {}
            "icon" | "icon2" => {
                if let Some(ExprSyntax {
                    kind: ExprKind::Int(icon),
                    ..
                }) = arg(0)
                {
                    pieces.push(Piece::Icon {
                        icon: *icon,
                        device: spec.name == "icon2",
                    });
                } else {
                    pieces.push(Piece::Opaque {
                        spelling: self.spelling(node),
                    });
                }
            }
            "edge" | "shadow" | "scale" | "key" | "fixed" | "link" | "level-pos" => {
                pieces.push(Piece::Opaque {
                    spelling: self.spelling(node),
                });
            }
            "if" | "switch" | "if-gender" | "if-self" | "if-name" | "josa" | "josa-ro" => {
                self.choice(spec.name, syntax, pieces);
            }
            "ruby" => {
                let base = arg(0).map(|expr| self.branch(expr)).unwrap_or_default();
                let reading = arg(1).map(|expr| self.branch(expr)).unwrap_or_default();
                pieces.push(Piece::Ruby { base, reading });
            }
            "upper" | "capitalize" | "title-case" | "lower" | "lower-first" => {
                let mut content = arg(0).map(|expr| self.content(expr)).unwrap_or_default();
                transform(spec.name, &mut content);
                pieces.extend(content);
            }
            "split" => {
                let label = self.spelling(node);
                pieces.push(Piece::Value {
                    kind: ValueKind::Text,
                    source: spec.name,
                    parameter: None,
                    label,
                    style: self.state.style(),
                    shown: Vec::new(),
                });
            }
            "player-name" => {
                pieces.push(Piece::Value {
                    kind: ValueKind::PlayerName,
                    source: spec.name,
                    parameter: arg(0).and_then(parameter_of),
                    label: arg(0).map(|expr| self.expr_text(expr)).unwrap_or_default(),
                    style: self.state.style(),
                    shown: Vec::new(),
                });
            }
            "string" => {
                if let Some(expr) = arg(0) {
                    let content = self.content(expr);
                    pieces.extend(content);
                }
            }
            "sheet" | "sheet-sub" | "noun-ja" | "noun-en" | "noun-de" | "noun-fr" | "noun-zh"
            | "platform" => self.reference(spec, syntax, pieces),
            _ => self.number(spec, syntax, node, pieces),
        }
    }

    /// A macro that shows a number, such as `<num>`.
    fn number(
        &mut self,
        spec: &catalog::MacroSpec,
        syntax: &MacroSyntax,
        node: &SyntaxNode,
        pieces: &mut Vec<Piece>,
    ) {
        let kind = match spec.args.first().map(|arg| arg.role) {
            Some(catalog::Role::Time) => ValueKind::Time,
            _ => ValueKind::Number,
        };
        let Some(expr) = syntax.args.first() else {
            pieces.push(Piece::Opaque {
                spelling: self.spelling(node),
            });
            return;
        };
        let formatted = self
            .int(expr)
            .filter(|_| kind == ValueKind::Number)
            .map(|value| self.format_number(spec.name, value, syntax));
        match (&expr.kind, formatted) {
            (ExprKind::Int(_), Some(text)) => push_text(pieces, &text, self.state.style()),
            (_, formatted) => {
                let mut value = self.value(expr, kind, spec.name);
                if let (Piece::Value { shown, style, .. }, Some(text)) = (&mut value, formatted) {
                    *shown = vec![Piece::Text {
                        text,
                        style: *style,
                    }];
                }
                pieces.push(value);
            }
        }
    }

    fn format_number(&self, name: &str, value: u32, syntax: &MacroSyntax) -> String {
        match name {
            "num2" => format!("{value:02}"),
            "hex" => format!("{value:X}"),
            "digit" => {
                let digits = syntax
                    .args
                    .get(1)
                    .and_then(|expr| self.int(expr))
                    .unwrap_or(1);
                format!(
                    "{value:0width$}",
                    width = usize::try_from(digits).unwrap_or(1)
                )
            }
            "kilo" => {
                let separator = match syntax.args.get(1).map(|expr| &expr.kind) {
                    Some(ExprKind::Str(nodes)) => nodes
                        .iter()
                        .map(|node| match &node.kind {
                            SyntaxKind::Text(text) => text.as_str(),
                            _ => "",
                        })
                        .collect::<String>(),
                    _ => ",".to_owned(),
                };
                group_thousands(value, &separator)
            }
            _ => value.to_string(),
        }
    }

    /// Renders an argument as content: text is rendered, a value is shown
    /// as a value.
    fn content(&mut self, expr: &ExprSyntax) -> Vec<Piece> {
        match &expr.kind {
            ExprKind::Str(nodes) => self.nodes(nodes),
            _ => vec![self.value(expr, ValueKind::Text, "string")],
        }
    }

    /// A value read from an expression, filled in when the variables give
    /// it.
    fn value(&self, expr: &ExprSyntax, kind: ValueKind, source: &'static str) -> Piece {
        let parameter = parameter_of(expr);
        let kind = match (&expr.kind, kind) {
            (ExprKind::Nullary(_), ValueKind::Number) => ValueKind::Time,
            (ExprKind::Param(..), ValueKind::Other) => {
                if parameter.is_some_and(Parameter::is_text) {
                    ValueKind::Text
                } else {
                    ValueKind::Number
                }
            }
            _ => kind,
        };
        let style = self.state.style();
        let shown = match self.operand_value(operand_of(expr)) {
            Some(Value::Text(text)) if !text.is_empty() => vec![Piece::Text { text, style }],
            Some(Value::Int(value)) if kind == ValueKind::Number => vec![Piece::Text {
                text: value.to_string(),
                style,
            }],
            _ => Vec::new(),
        };
        Piece::Value {
            kind,
            source,
            parameter,
            label: self.expr_text(expr),
            style,
            shown,
        }
    }

    fn pair(&mut self, name: &str, close: Close, syntax: &MacroSyntax) {
        let value = syntax.args.first().map(|arg| &arg.kind);
        let closing = syntax.written == Written::Close
            || match (close, value) {
                (Close::Int(expected), Some(ExprKind::Int(actual))) => *actual == expected,
                (Close::StackColor, Some(ExprKind::Nullary(0xEC))) => true,
                _ => false,
            };
        match name {
            "i" => self.state.italic = !closing,
            "b" => self.state.bold = !closing,
            "color" | "ui-color" | "edge-color" | "ui-edge-color" => {
                let color = match value {
                    Some(ExprKind::Int(value)) if name.starts_with("ui-") => {
                        self.data.ui_color(*value)
                    }
                    Some(ExprKind::Int(value)) => Some(argb_to_rgba(*value)),
                    _ => None,
                };
                let stack = if name.contains("edge") {
                    &mut self.state.edges
                } else {
                    &mut self.state.colors
                };
                if closing {
                    stack.pop();
                } else {
                    stack.push(color);
                }
            }
            // Shadow colors are not shown.
            _ => {}
        }
    }

    fn reference(
        &mut self,
        spec: &catalog::MacroSpec,
        syntax: &MacroSyntax,
        pieces: &mut Vec<Piece>,
    ) {
        let args = &syntax.args;
        let (row_index, sheet, column) = reference_parts(spec.name, args).unwrap_or((1, None, 0));
        let row_expr = args.get(row_index);
        let parameter = row_expr.and_then(parameter_of);
        let row = row_expr.and_then(|expr| self.int(expr));
        let style = self.state.style();
        let mut looked_up = Vec::new();
        if let (Some(sheet), Some(row)) = (&sheet, row)
            && spec.name != "platform"
            && spec.name != "sheet-sub"
            && self.depth < MAX_REFERENCE_DEPTH
            && let Some(text) = self.data.sheet_text(sheet, row, column)
        {
            let document = parse(&text);
            // Game data text reads its own parameters, not this string's.
            let mut variables = Variables::default();
            let values = Values::new();
            let mut nested = Renderer {
                data: self.data,
                document: &document,
                values: &values,
                variables: &mut variables,
                state: self.state.clone(),
                depth: self.depth + 1,
            };
            looked_up = nested.nodes(document.nodes());
            if parameter.is_none() {
                pieces.extend(looked_up);
                return;
            }
        }
        let label = args
            .iter()
            .take(if spec.name == "sheet" {
                3
            } else {
                row_index + 1
            })
            .map(|expr| self.expr_text(expr))
            .collect::<Vec<_>>()
            .join(" ");
        pieces.push(Piece::Value {
            kind: ValueKind::GameData,
            source: spec.name,
            parameter,
            label,
            style,
            shown: looked_up,
        });
    }
}

fn push_text(pieces: &mut Vec<Piece>, text: &str, style: Style) {
    if let Some(Piece::Text {
        text: previous,
        style: previous_style,
    }) = pieces.last_mut()
        && *previous_style == style
    {
        previous.push_str(text);
        return;
    }
    pieces.push(Piece::Text {
        text: text.to_owned(),
        style,
    });
}

fn group_thousands(value: u32, separator: &str) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push_str(separator);
        }
        out.push(digit);
    }
    out
}

/// Converts a macro color `0xAARRGGBB` to `0xRRGGBBAA`.
const fn argb_to_rgba(value: u32) -> Rgba {
    value.rotate_left(8)
}

fn transform(name: &str, pieces: &mut [Piece]) {
    let mut first = true;
    for piece in pieces {
        let Piece::Text { text, .. } = piece else {
            if matches!(piece, Piece::Value { .. }) {
                first = false;
            }
            continue;
        };
        *text = match name {
            "upper" => text.to_uppercase(),
            "lower" => text.to_lowercase(),
            "title-case" => title_case(text),
            "capitalize" if first => change_first(text, true),
            "lower-first" if first => change_first(text, false),
            _ => std::mem::take(text),
        };
        if !text.is_empty() {
            first = false;
        }
    }
}

fn change_first(text: &str, upper: bool) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        let first: String = if upper {
            first.to_uppercase().collect()
        } else {
            first.to_lowercase().collect()
        };
        first + characters.as_str()
    })
}

fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for character in text.chars() {
        if start {
            out.extend(character.to_uppercase());
        } else {
            out.push(character);
        }
        start = character.is_whitespace();
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;

    struct Data;

    impl PreviewData for Data {
        fn ui_color(&self, row: u32) -> Option<u32> {
            (row == 504).then_some(0x00CC_22FF)
        }

        fn sheet_text(&self, sheet: &str, row: u32, column: u32) -> Option<String> {
            match (sheet, row, column) {
                ("Item", 5, 0) => Some("<i>Potion</i>".to_owned()),
                ("ClassJob", 19, 0) => Some("paladin".to_owned()),
                ("ClassJob", 20, 0) => Some("monk".to_owned()),
                _ => None,
            }
        }

        fn sheet_rows(&self, sheet: &str) -> Vec<u32> {
            if sheet == "ClassJob" {
                vec![0, 19, 20]
            } else {
                Vec::new()
            }
        }
    }

    fn text(text: &str, style: Style) -> Piece {
        Piece::Text {
            text: text.to_owned(),
            style,
        }
    }

    fn plain(text: &str) -> Piece {
        self::text(text, Style::default())
    }

    /// The text a preview shows, with unknown values as their labels.
    fn shown(pieces: &[Piece]) -> String {
        let mut out = String::new();
        for piece in pieces {
            match piece {
                Piece::Text { text, .. } => out.push_str(text),
                Piece::Break => out.push('\n'),
                Piece::Value {
                    label,
                    shown: values,
                    ..
                } if values.is_empty() => {
                    let _ = write!(out, "[{label}]");
                }
                Piece::Value { shown: values, .. } => out.push_str(&shown(values)),
                Piece::Choice {
                    selected: Some(index),
                    branches,
                    ..
                } => out.push_str(&shown(&branches[*index])),
                Piece::Choice { branches, .. } => {
                    let _ = write!(
                        out,
                        "{{{}}}",
                        branches
                            .iter()
                            .map(|branch| shown(branch))
                            .collect::<Vec<_>>()
                            .join("|")
                    );
                }
                Piece::Icon { icon, .. } => {
                    let _ = write!(out, "({icon})");
                }
                Piece::Ruby { base, .. } => out.push_str(&shown(base)),
                Piece::Opaque { spelling } => out.push_str(spelling),
            }
        }
        out
    }

    fn values(entries: &[(&str, Value)]) -> Values {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    const POTENCY: &str =
        "<if ($gn68 == 20)><if ($gn72 >= 94)>{320}<else>{280}</if><else>{240}</if>";

    #[test]
    fn formatting_follows_the_game_stacks() {
        let green = Style {
            color: Some(0x00CC_22FF),
            ..Style::default()
        };
        assert_eq!(
            preview(
                "<ui-color 504>Duration:</ui-color> 20s",
                &Data,
                &Values::new()
            )
            .pieces,
            [text("Duration:", green), plain(" 20s")]
        );
        let red = Style {
            color: Some(0xFF00_00FF),
            italic: true,
            ..Style::default()
        };
        assert_eq!(
            preview("<i><color #FFFF0000>a</color></i>", &Data, &Values::new()).pieces,
            [text("a", red)]
        );
    }

    #[test]
    fn nested_conditions_follow_the_values_and_default_to_the_first_branches() {
        let defaults = preview(POTENCY, &Data, &Values::new());
        assert_eq!(shown(&defaults.pieces), "320");
        let keys: Vec<(&str, &Value)> = defaults
            .variables
            .iter()
            .map(|variable| (variable.key.as_str(), &variable.default))
            .collect();
        assert_eq!(keys, [("gn68", &Value::Int(20)), ("gn72", &Value::Int(94))]);
        let level = values(&[("gn72", Value::Int(90))]);
        assert_eq!(shown(&preview(POTENCY, &Data, &level).pieces), "280");
        let other = values(&[("gn68", Value::Int(19)), ("gn72", Value::Int(100))]);
        assert_eq!(shown(&preview(POTENCY, &Data, &other).pieces), "240");
    }

    #[test]
    fn known_globals_carry_their_meaning_and_rows() {
        let preview = preview(POTENCY, &Data, &Values::new());
        let class = &preview.variables[0];
        assert_eq!(class.global.map(|global| global.name), Some("class-job"));
        assert_eq!(class.value_name.as_deref(), Some("monk"));
        assert_eq!(
            class.options,
            [(19, "paladin".to_owned()), (20, "monk".to_owned())]
        );
        assert_eq!(
            preview.variables[1].global.map(|global| global.name),
            Some("level")
        );
    }

    #[test]
    fn every_operator_and_switch_evaluates() {
        for (operator, expected) in [
            ("==", "a"),
            ("!=", "b"),
            ("<", "a"),
            ("<=", "a"),
            (">", "b"),
            (">=", "b"),
        ] {
            let text = format!("<if ($n1 {operator} 4)>b<else>a</if>");
            let five = values(&[("n1", Value::Int(5))]);
            assert_eq!(
                shown(&preview(&text, &NoData, &five).pieces),
                expected,
                "{operator}"
            );
        }
        let text = "<switch $n1><case>x<case>y</switch>!";
        assert_eq!(
            shown(&preview(text, &NoData, &values(&[("n1", Value::Int(2))])).pieces),
            "y!"
        );
        assert_eq!(
            shown(&preview(text, &NoData, &values(&[("n1", Value::Int(3))])).pieces),
            "!"
        );
        let right = preview("<if (10 < $n2)>a<else>b</if>", &NoData, &Values::new());
        assert_eq!(right.variables[0].default, Value::Int(11));
        assert_eq!(shown(&right.pieces), "a");
    }

    #[test]
    fn characters_and_texts_are_variables() {
        let text = "<if-gender $n1>He<else>She</if-gender> sees <if ($gs1 == $gs3)>your<else><string $gs3>'s</if> room.";
        assert_eq!(
            shown(&preview(text, &NoData, &Values::new()).pieces),
            "He sees your room."
        );
        let set = values(&[
            ("gender", Value::Int(1)),
            ("gs1", Value::Text("Alex".to_owned())),
            ("gs3", Value::Text("Mira".to_owned())),
        ]);
        let result = preview(text, &NoData, &set);
        assert_eq!(shown(&result.pieces), "She sees Mira's room.");
        let keys: Vec<&str> = result
            .variables
            .iter()
            .map(|variable| variable.key.as_str())
            .collect();
        assert_eq!(keys, ["gender", "gs1", "gs3"]);
        assert_eq!(
            result.variables[1].global.map(|global| global.name),
            Some("player-name")
        );
    }

    #[test]
    fn values_fill_numbers_texts_and_rows() {
        let text = "<num $n1> <kilo $n2 \",\"> <num2 $n3> <string $s1> <sheet Item $n4 0>";
        let set = values(&[
            ("n1", Value::Int(7)),
            ("n2", Value::Int(1_234_567)),
            ("n3", Value::Int(5)),
            ("n4", Value::Int(5)),
        ]);
        let result = preview(text, &Data, &set);
        assert_eq!(shown(&result.pieces), "7 1,234,567 05 [$s1] Potion");
        let row = result
            .variables
            .iter()
            .find(|variable| variable.key == "n4")
            .expect("n4");
        assert_eq!(row.sheet, Some(("Item".to_owned(), 0)));
        assert_eq!(row.value_name.as_deref(), Some("Potion"));
        assert_eq!(
            shown(&preview("<sheet Item $n1 0>", &NoData, &Values::new()).pieces),
            "[Item $n1 0]"
        );
    }

    #[test]
    fn conditions_the_values_cannot_decide_keep_every_branch() {
        let result = preview(
            "<if \"<sheet BNpcName 1 6>\">her<else>his</if>",
            &NoData,
            &Values::new(),
        );
        assert_eq!(shown(&result.pieces), "{her|his}");
    }

    #[test]
    fn transforms_apply_to_selected_text() {
        let pieces = preview("<capitalize>x y</capitalize><br>", &NoData, &Values::new()).pieces;
        assert_eq!(pieces, [plain("X y"), Piece::Break]);
    }
}
