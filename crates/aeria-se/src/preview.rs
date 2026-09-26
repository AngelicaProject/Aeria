//! A preview of macro text as the game shows it.
//!
//! [`preview`] interprets parsed macro text into pieces: styled text, line
//! breaks, runtime values, icons, and conditions whose branches can be
//! chosen. Formatting follows the game's stacks: `<color>` pushes a color
//! and `</color>` restores the previous one. Game data comes from a
//! [`PreviewData`] source, such as the installed game; without it,
//! references are shown as values.
//!
//! The preview approximates the game. Values supplied at runtime, such as
//! parameters, are shown as labeled values, never guessed.

use crate::catalog::{self, Close, Form, PARAMETERS, Role};
use crate::syntax::{
    ExprKind, ExprSyntax, MacroString, MacroSyntax, SyntaxKind, SyntaxNode, Written, parse,
};

/// Deepest nesting of game data text rendered inside a preview.
const MAX_REFERENCE_DEPTH: usize = 2;

/// Game data a preview reads.
pub trait PreviewData {
    /// The foreground color of a `UIColor` row as `0xRRGGBBAA`.
    fn ui_color(&self, row: u32) -> Option<u32>;
    /// The macro text of a string cell of a sheet in the source language.
    fn sheet_text(&self, sheet: &str, row: u32, column: u32) -> Option<String>;
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

/// A parameter the value reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Parameter {
    /// `n`, `s`, `gn`, or `gs`.
    pub prefix: &'static str,
    pub index: u32,
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
    /// A value supplied at runtime, such as a parameter or a game data name
    /// that cannot be looked up.
    Value {
        kind: ValueKind,
        /// The macro name that produces it, such as `num`.
        source: &'static str,
        parameter: Option<Parameter>,
        /// A short label in macro text, such as `$n1` or `Item $n1 0`.
        label: String,
        style: Style,
    },
    Icon {
        icon: u32,
        /// Whether the icon depends on the input device.
        device: bool,
    },
    Choice {
        kind: ChoiceKind,
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

/// Renders macro text as a preview. Diagnostics do not stop the preview;
/// the parts that could not be parsed are left out.
#[must_use]
pub fn preview(text: &str, data: &dyn PreviewData) -> Vec<Piece> {
    let document = parse(text);
    let mut renderer = Renderer {
        data,
        document: &document,
        state: State::default(),
        depth: 0,
    };
    renderer.nodes(document.nodes())
}

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

struct Renderer<'a> {
    data: &'a dyn PreviewData,
    document: &'a MacroString,
    state: State,
    depth: usize,
}

impl Renderer<'_> {
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

    #[allow(clippy::too_many_lines)]
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
                let kind = match spec.name {
                    "if" => ChoiceKind::If {
                        condition: arg(0).map(|expr| self.expr_text(expr)).unwrap_or_default(),
                    },
                    "switch" => ChoiceKind::Switch {
                        value: arg(0).map(|expr| self.expr_text(expr)).unwrap_or_default(),
                    },
                    "if-gender" => ChoiceKind::Gender,
                    "if-self" => ChoiceKind::Myself,
                    "if-name" => ChoiceKind::Name,
                    _ => ChoiceKind::Josa,
                };
                let branches = args
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        spec.arg(*index)
                            .is_some_and(|arg| arg.place == catalog::Place::Block)
                    })
                    .map(|(_, expr)| self.branch(expr))
                    .collect();
                pieces.push(Piece::Choice { kind, branches });
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
                });
            }
            "player-name" => {
                let parameter = arg(0).and_then(parameter_of);
                pieces.push(Piece::Value {
                    kind: ValueKind::PlayerName,
                    source: spec.name,
                    parameter,
                    label: arg(0).map(|expr| self.expr_text(expr)).unwrap_or_default(),
                    style: self.state.style(),
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
            _ => {
                let role = spec.args.first().map_or(Role::Other, |arg| arg.role);
                let kind = match role {
                    Role::Time => ValueKind::Time,
                    _ => ValueKind::Number,
                };
                match arg(0) {
                    Some(ExprSyntax {
                        kind: ExprKind::Int(value),
                        ..
                    }) if spec.name == "num" => {
                        push_text(pieces, &value.to_string(), self.state.style());
                    }
                    Some(expr) => {
                        let value = self.value(expr, kind, spec.name);
                        pieces.push(value);
                    }
                    None => pieces.push(Piece::Opaque {
                        spelling: self.spelling(node),
                    }),
                }
            }
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

    fn value(&self, expr: &ExprSyntax, kind: ValueKind, source: &'static str) -> Piece {
        let kind = match (&expr.kind, kind) {
            (ExprKind::Nullary(_), ValueKind::Number) => ValueKind::Time,
            (ExprKind::Param(code, _), ValueKind::Other) => {
                if matches!(*code, 0xEA | 0xEB) {
                    ValueKind::Text
                } else {
                    ValueKind::Number
                }
            }
            _ => kind,
        };
        Piece::Value {
            kind,
            source,
            parameter: parameter_of(expr),
            label: self.expr_text(expr),
            style: self.state.style(),
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
        let row_index = if spec.name.starts_with("noun") { 2 } else { 1 };
        let int = |index: usize| match args.get(index).map(|arg| &arg.kind) {
            Some(ExprKind::Int(value)) => Some(*value),
            _ => None,
        };
        let column = if spec.name == "sheet" {
            int(2).unwrap_or(0)
        } else {
            0
        };
        if let (Some(sheet), Some(row)) = (&sheet, int(row_index))
            && spec.name != "platform"
            && spec.name != "sheet-sub"
            && self.depth < MAX_REFERENCE_DEPTH
            && let Some(text) = self.data.sheet_text(sheet, row, column)
        {
            let document = parse(&text);
            let mut nested = Renderer {
                data: self.data,
                document: &document,
                state: self.state.clone(),
                depth: self.depth + 1,
            };
            pieces.extend(nested.nodes(document.nodes()));
            return;
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
            parameter: args.get(row_index).and_then(parameter_of),
            label,
            style: self.state.style(),
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
    use super::*;

    struct Data;

    impl PreviewData for Data {
        fn ui_color(&self, row: u32) -> Option<u32> {
            (row == 504).then_some(0x00CC_22FF)
        }

        fn sheet_text(&self, sheet: &str, row: u32, column: u32) -> Option<String> {
            (sheet == "Item" && row == 5 && column == 0).then(|| "<i>Potion</i>".to_owned())
        }
    }

    fn text(text: &str, style: Style) -> Piece {
        Piece::Text {
            text: text.to_owned(),
            style,
        }
    }

    #[test]
    fn formatting_follows_the_game_stacks() {
        let green = Style {
            color: Some(0x00CC_22FF),
            ..Style::default()
        };
        assert_eq!(
            preview("<ui-color 504>Duration:</ui-color> 20s", &Data),
            [text("Duration:", green), text(" 20s", Style::default())]
        );
        let red = Style {
            color: Some(0xFF00_00FF),
            italic: true,
            ..Style::default()
        };
        assert_eq!(
            preview("<i><color #FFFF0000>a</color></i>", &Data),
            [text("a", red)]
        );
    }

    #[test]
    fn references_read_game_data_and_values_stay_labeled() {
        let italic = Style {
            italic: true,
            ..Style::default()
        };
        assert_eq!(
            preview("Take <sheet Item 5 0>.", &Data),
            [
                text("Take ", Style::default()),
                text("Potion", italic),
                text(".", Style::default())
            ]
        );
        assert_eq!(
            preview("<sheet Item $n1 0>", &NoData),
            [Piece::Value {
                kind: ValueKind::GameData,
                source: "sheet",
                parameter: Some(Parameter {
                    prefix: "n",
                    index: 1
                }),
                label: "Item $n1 0".to_owned(),
                style: Style::default(),
            }]
        );
    }

    #[test]
    fn choices_keep_every_branch_and_transforms_apply() {
        assert_eq!(
            preview("<if $gn1>{320}<else>{240}</if>", &NoData),
            [Piece::Choice {
                kind: ChoiceKind::If {
                    condition: "$gn1".to_owned()
                },
                branches: vec![
                    vec![text("320", Style::default())],
                    vec![text("240", Style::default())]
                ],
            }]
        );
        let pieces = preview(
            "<if ($n1 == 1)>him<else>her</if><capitalize>x y</capitalize><br>",
            &NoData,
        );
        assert_eq!(
            pieces[0],
            Piece::Choice {
                kind: ChoiceKind::If {
                    condition: "($n1 == 1)".to_owned()
                },
                branches: vec![
                    vec![text("him", Style::default())],
                    vec![text("her", Style::default())]
                ],
            }
        );
        assert_eq!(pieces[1], text("X y", Style::default()));
        assert_eq!(pieces[2], Piece::Break);
    }
}
