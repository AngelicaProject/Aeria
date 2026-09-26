//! Macro text for the editor: diagnostics, tags, and the game preview.
//!
//! The renderer highlights and edits macro text; this command tells it what
//! the text means. Offsets are UTF-16 code units, as the editor counts them.

use std::sync::Arc;

use aeria_se::catalog::{Form, Place, Role};
use aeria_se::preview::{ChoiceKind, Parameter, Piece, PreviewData, Style, ValueKind};
use aeria_se::{
    ExprKind, ExprSyntax, MacroString, MacroSyntax, SemanticFamily, SyntaxKind, SyntaxNode,
    Written, parse,
};
use aeria_source::GameSource;
use serde::Serialize;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroViewDto {
    pub diagnostics: Vec<MacroDiagnosticDto>,
    pub tags: Vec<MacroTagDto>,
    pub preview: Vec<PreviewPieceDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroDiagnosticDto {
    pub from: usize,
    pub to: usize,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MacroTagPart {
    Inline,
    Open,
    Close,
    Separator,
    Generic,
    Raw,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroTagDto {
    pub from: usize,
    pub to: usize,
    /// The macro's tag name, also for its separators and closing tag.
    pub name: String,
    pub part: MacroTagPart,
    pub family: Option<&'static str>,
    /// The inline arguments, on an opening or inline tag.
    pub args: Vec<MacroArgDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroArgDto {
    pub name: &'static str,
    pub role: &'static str,
    /// The argument as macro text.
    pub value: String,
    pub parameter: Option<ParameterDto>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterDto {
    pub prefix: &'static str,
    pub index: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleDto {
    /// `#rrggbbaa`.
    pub color: Option<String>,
    pub edge: Option<String>,
    pub italic: bool,
    pub bold: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ChoiceDto {
    If { condition: String },
    Switch { value: String },
    Gender,
    Myself,
    Name,
    Josa,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PreviewPieceDto {
    Text {
        text: String,
        style: StyleDto,
    },
    Break,
    #[serde(rename_all = "camelCase")]
    Value {
        value_kind: &'static str,
        source: &'static str,
        parameter: Option<ParameterDto>,
        label: String,
        style: StyleDto,
    },
    Icon {
        icon: u32,
        device: bool,
    },
    Choice {
        choice: ChoiceDto,
        branches: Vec<Vec<PreviewPieceDto>>,
    },
    Ruby {
        base: Vec<PreviewPieceDto>,
        reading: Vec<PreviewPieceDto>,
    },
    Opaque {
        spelling: String,
    },
}

/// Game data for previews, from the open project's game when there is one.
struct GameData(Option<Arc<GameSource>>);

impl PreviewData for GameData {
    fn ui_color(&self, row: u32) -> Option<u32> {
        self.0.as_ref()?.ui_color(row)
    }

    fn sheet_text(&self, sheet: &str, row: u32, column: u32) -> Option<String> {
        self.0
            .as_ref()?
            .cell_text(sheet, row, column)
            .ok()
            .flatten()
    }
}

#[tauri::command(rename_all = "camelCase")]
/// Describes macro text for the editor: its diagnostics, its tags with
/// their arguments, and a preview as the game shows it. Game data is read
/// from the open project's game; without a project, references are shown
/// as values.
///
/// # Errors
///
/// Returns a typed command error when the desktop worker fails.
pub async fn macro_view(app: tauri::AppHandle, text: String) -> CommandResult<MacroViewDto> {
    run_blocking(move || {
        let state = app.state::<DesktopState>();
        let source = state.lock_project().ok().and_then(|project| {
            project
                .as_ref()
                .map(aeria_workspace::ProjectSession::source_handle)
        });
        Ok(view(&text, &GameData(source)))
    })
    .await
}

fn view(text: &str, data: &dyn PreviewData) -> MacroViewDto {
    let document = parse(text);
    let offsets = Utf16Offsets::new(text);
    let mut tags = Vec::new();
    collect_tags(&document, document.nodes(), &offsets, &mut tags);
    MacroViewDto {
        diagnostics: document
            .diagnostics()
            .iter()
            .map(|diagnostic| MacroDiagnosticDto {
                from: offsets.at(diagnostic.span.start()),
                to: offsets.at(diagnostic.span.end()),
                message: diagnostic.message.clone(),
            })
            .collect(),
        tags,
        preview: aeria_se::preview::preview(text, data)
            .into_iter()
            .map(piece_dto)
            .collect(),
    }
}

/// UTF-16 offsets of byte offsets into one text.
struct Utf16Offsets(Vec<usize>);

impl Utf16Offsets {
    fn new(text: &str) -> Self {
        let mut offsets = vec![0; text.len() + 1];
        let mut units = 0;
        for (index, character) in text.char_indices() {
            for offset in &mut offsets[index..index + character.len_utf8()] {
                *offset = units;
            }
            units += character.len_utf16();
        }
        offsets[text.len()] = units;
        Self(offsets)
    }

    fn at(&self, byte: usize) -> usize {
        self.0
            .get(byte)
            .copied()
            .unwrap_or_else(|| self.0.last().copied().unwrap_or(0))
    }
}

const fn family_name(family: SemanticFamily) -> &'static str {
    match family {
        SemanticFamily::TranslatableText => "translatableText",
        SemanticFamily::FormattingPresentation => "formatting",
        SemanticFamily::ConditionalSelection => "condition",
        SemanticFamily::RuntimeContextValue => "runtimeValue",
        SemanticFamily::GameDataReference => "gameData",
        SemanticFamily::LayoutTextualControl => "layout",
        SemanticFamily::OpaqueProtected => "opaque",
    }
}

const fn role_name(role: Role) -> &'static str {
    match role {
        Role::Condition => "condition",
        Role::Selector => "selector",
        Role::Text => "text",
        Role::Number => "number",
        Role::Object => "object",
        Role::Sheet => "sheet",
        Role::Row => "row",
        Role::Column => "column",
        Role::Color => "color",
        Role::UiColor => "uiColor",
        Role::Icon => "icon",
        Role::Flag => "flag",
        Role::Separator => "separator",
        Role::Count => "count",
        Role::Time => "time",
        Role::Other => "other",
    }
}

fn parameter(expr: &ExprSyntax) -> Option<ParameterDto> {
    let ExprKind::Param(code, operand) = &expr.kind else {
        return None;
    };
    let ExprKind::Int(index) = operand.kind else {
        return None;
    };
    aeria_se::catalog::PARAMETERS
        .iter()
        .find(|spec| spec.code == *code)
        .map(|spec| ParameterDto {
            prefix: spec.prefix,
            index,
        })
}

fn collect_tags(
    document: &MacroString,
    nodes: &[SyntaxNode],
    offsets: &Utf16Offsets,
    tags: &mut Vec<MacroTagDto>,
) {
    for node in nodes {
        match &node.kind {
            SyntaxKind::Text(_) | SyntaxKind::Error => {}
            SyntaxKind::Raw(_) => tags.push(MacroTagDto {
                from: offsets.at(node.span.start()),
                to: offsets.at(node.span.end()),
                name: "raw".to_owned(),
                part: MacroTagPart::Raw,
                family: None,
                args: Vec::new(),
            }),
            SyntaxKind::Macro(syntax) => {
                macro_tags(document, syntax, offsets, tags);
                for arg in &syntax.args {
                    collect_expr_tags(document, arg, offsets, tags);
                }
                tags.sort_by_key(|tag| tag.from);
            }
        }
    }
}

fn collect_expr_tags(
    document: &MacroString,
    expr: &ExprSyntax,
    offsets: &Utf16Offsets,
    tags: &mut Vec<MacroTagDto>,
) {
    match &expr.kind {
        ExprKind::Str(nodes) => collect_tags(document, nodes, offsets, tags),
        ExprKind::Param(_, operand) => collect_expr_tags(document, operand, offsets, tags),
        ExprKind::Compare(_, left, right) => {
            collect_expr_tags(document, left, offsets, tags);
            collect_expr_tags(document, right, offsets, tags);
        }
        _ => {}
    }
}

fn macro_tags(
    document: &MacroString,
    syntax: &MacroSyntax,
    offsets: &Utf16Offsets,
    tags: &mut Vec<MacroTagDto>,
) {
    let name = syntax.spec.map_or_else(
        || format!("code:{:02X}", syntax.code),
        |spec| spec.name.to_owned(),
    );
    let family = syntax.spec.map(|spec| family_name(spec.family));
    let args: Vec<MacroArgDto> = syntax
        .args
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            syntax.spec.is_none_or(|spec| {
                spec.arg(*index)
                    .is_some_and(|arg| arg.place == Place::Inline)
            })
        })
        .filter(|(_, arg)| !arg.span.is_empty())
        .map(|(index, arg)| {
            let described = syntax.spec.and_then(|spec| spec.arg(index));
            MacroArgDto {
                name: described.map_or("value", |arg| arg.name),
                role: role_name(described.map_or(Role::Other, |arg| arg.role)),
                value: document.slice(arg.span).unwrap_or_default().to_owned(),
                parameter: parameter(arg),
            }
        })
        .collect();
    let count = syntax.tags.len();
    for (index, span) in syntax.tags.iter().enumerate() {
        let part = match (syntax.written, syntax.spec.map(|spec| spec.form)) {
            (Written::Generic, _) => MacroTagPart::Generic,
            (Written::Close, _) => MacroTagPart::Close,
            (Written::Open, _) => MacroTagPart::Open,
            (Written::Block, Some(Form::Block { .. })) if index == 0 => MacroTagPart::Open,
            (Written::Block, _) if index + 1 == count => MacroTagPart::Close,
            (Written::Block, _) => MacroTagPart::Separator,
            (Written::Inline, _) => MacroTagPart::Inline,
        };
        tags.push(MacroTagDto {
            from: offsets.at(span.start()),
            to: offsets.at(span.end()),
            name: name.clone(),
            part,
            family,
            args: if matches!(
                part,
                MacroTagPart::Open | MacroTagPart::Inline | MacroTagPart::Generic
            ) {
                args.iter()
                    .map(|arg| MacroArgDto {
                        name: arg.name,
                        role: arg.role,
                        value: arg.value.clone(),
                        parameter: arg.parameter,
                    })
                    .collect()
            } else {
                Vec::new()
            },
        });
    }
}

fn color(value: Option<u32>) -> Option<String> {
    value.map(|rgba| format!("#{rgba:08x}"))
}

fn style_dto(style: Style) -> StyleDto {
    StyleDto {
        color: color(style.color),
        edge: color(style.edge),
        italic: style.italic,
        bold: style.bold,
    }
}

fn parameter_dto(parameter: Parameter) -> ParameterDto {
    ParameterDto {
        prefix: parameter.prefix,
        index: parameter.index,
    }
}

const fn value_kind(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Number => "number",
        ValueKind::Text => "text",
        ValueKind::PlayerName => "playerName",
        ValueKind::GameData => "gameData",
        ValueKind::Time => "time",
        ValueKind::Other => "other",
    }
}

fn piece_dto(piece: Piece) -> PreviewPieceDto {
    match piece {
        Piece::Text { text, style } => PreviewPieceDto::Text {
            text,
            style: style_dto(style),
        },
        Piece::Break => PreviewPieceDto::Break,
        Piece::Value {
            kind,
            source,
            parameter,
            label,
            style,
        } => PreviewPieceDto::Value {
            value_kind: value_kind(kind),
            source,
            parameter: parameter.map(parameter_dto),
            label,
            style: style_dto(style),
        },
        Piece::Icon { icon, device } => PreviewPieceDto::Icon { icon, device },
        Piece::Choice { kind, branches } => PreviewPieceDto::Choice {
            choice: match kind {
                ChoiceKind::If { condition } => ChoiceDto::If { condition },
                ChoiceKind::Switch { value } => ChoiceDto::Switch { value },
                ChoiceKind::Gender => ChoiceDto::Gender,
                ChoiceKind::Myself => ChoiceDto::Myself,
                ChoiceKind::Name => ChoiceDto::Name,
                ChoiceKind::Josa => ChoiceDto::Josa,
            },
            branches: branches
                .into_iter()
                .map(|branch| branch.into_iter().map(piece_dto).collect())
                .collect(),
        },
        Piece::Ruby { base, reading } => PreviewPieceDto::Ruby {
            base: base.into_iter().map(piece_dto).collect(),
            reading: reading.into_iter().map(piece_dto).collect(),
        },
        Piece::Opaque { spelling } => PreviewPieceDto::Opaque { spelling },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_reports_tags_diagnostics_and_a_preview_in_utf16_offsets() {
        let text = "Ф<if ($n1 == 1)><i>a</i><else>b</if> <sheet Item $n1 0><nope>";
        let view = view(text, &aeria_se::preview::NoData);
        let spans: Vec<(&str, usize, usize)> = view
            .tags
            .iter()
            .map(|tag| (tag.name.as_str(), tag.from, tag.to))
            .collect();
        assert_eq!(
            spans,
            [
                ("if", 1, 16),
                ("i", 16, 19),
                ("i", 20, 24),
                ("if", 24, 30),
                ("if", 31, 36),
                ("sheet", 37, 55),
            ]
        );
        assert!(matches!(view.tags[0].part, MacroTagPart::Open));
        assert_eq!(view.tags[0].args[0].name, "condition");
        assert!(matches!(view.tags[3].part, MacroTagPart::Separator));
        assert!(matches!(view.tags[4].part, MacroTagPart::Close));
        let row = &view.tags[5].args[1];
        assert_eq!(
            (row.name, row.role, row.value.as_str()),
            ("row", "row", "$n1")
        );
        assert_eq!(row.parameter.map(|parameter| parameter.index), Some(1));
        assert_eq!(view.diagnostics.len(), 1);
        assert_eq!(view.diagnostics[0].from, 55);
        let json = serde_json::to_value(&view.preview).expect("json");
        assert_eq!(json[1]["kind"], "choice");
        assert_eq!(json[1]["choice"]["type"], "if");
        assert_eq!(json[1]["branches"][0][0]["style"]["italic"], true);
        assert_eq!(json[3]["kind"], "value");
        assert_eq!(json[3]["valueKind"], "gameData");
    }
}
