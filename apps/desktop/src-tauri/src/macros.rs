//! Macro text for the editor: diagnostics and tags, and the game glyphs
//! and icons the editor draws.
//!
//! The renderer highlights and edits macro text; this command tells it what
//! the text means. Offsets are UTF-16 code units, as the editor counts them.

use std::sync::Arc;

use aeria_se::catalog::{Form, Place, Role};
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
    /// The color an opening color tag sets, `#rrggbbaa`, when it is known.
    pub color: Option<String>,
    /// What an opening `<if>` or `<switch>` tests, part by part.
    pub condition: Option<ConditionDto>,
}

/// A condition of an `<if>` or the value of a `<switch>`: `left` alone is
/// true when it is not zero or empty.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionDto {
    pub left: OperandDto,
    /// `==`, `!=`, `<`, `<=`, `>`, or `>=`.
    pub operator: Option<&'static str>,
    pub right: Option<OperandDto>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OperandDto {
    /// A number, with the name of its row when it is compared with a global
    /// whose values are rows of a sheet, such as a class.
    Int { value: u32, name: Option<String> },
    /// A parameter such as `$gn68`, with the established meaning of a global.
    Parameter {
        code: String,
        meaning: Option<&'static str>,
    },
    /// A time value such as `hour`.
    Time { name: &'static str },
    /// Anything else, as macro text.
    Other { text: String },
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

#[tauri::command(rename_all = "camelCase")]
/// Describes macro text for the editor: its diagnostics and its tags with
/// their arguments. The colors of `<ui-color>` tags are read from the open
/// project's game; without a project they are unknown.
///
/// # Errors
///
/// Returns a typed command error when the desktop worker fails.
pub async fn macro_view(app: tauri::AppHandle, text: String) -> CommandResult<MacroViewDto> {
    run_blocking(move || {
        let source = project_source(&app);
        Ok(view(&text, &source))
    })
    .await
}

/// A construct of several macros that reads as one value, such as the
/// player's first name.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroIdiomDto {
    pub name: &'static str,
    /// The exact macro text.
    pub text: &'static str,
    pub summary: &'static str,
}

#[tauri::command(rename_all = "camelCase")]
/// The idioms of `aeria_se::catalog`, for the editor to show each as one
/// value while the text keeps its macros.
#[must_use]
pub fn macro_idioms() -> Vec<MacroIdiomDto> {
    aeria_se::catalog::IDIOMS
        .iter()
        .map(|idiom| MacroIdiomDto {
            name: idiom.name,
            text: idiom.text,
            summary: idiom.summary,
        })
        .collect()
}

/// A macro a person can insert while translating (see
/// `aeria_se::catalog::INSERTIONS`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroInsertionDto {
    pub name: &'static str,
    pub group: &'static str,
    /// `insert`, `wrap`, or `branches`.
    pub form: &'static str,
    /// The macro text around the selection; `{row}` is one of `rows`.
    pub parts: &'static [&'static str],
    /// The rows it is offered for, with their names in the source language;
    /// empty for an insertion without rows.
    pub rows: Vec<MacroRowDto>,
    pub summary: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacroRowDto {
    pub row: u32,
    pub name: String,
}

#[tauri::command(rename_all = "camelCase")]
/// The macros a person can insert while translating, with the rows of the
/// open project's game that an insertion is offered for, such as each race.
/// Without a project, an insertion that needs rows has none.
///
/// # Errors
///
/// Returns a typed command error when the desktop worker fails.
pub async fn macro_insertions(app: tauri::AppHandle) -> CommandResult<Vec<MacroInsertionDto>> {
    run_blocking(move || {
        let source = project_source(&app);
        Ok(insertions(&source))
    })
    .await
}

fn insertions(game: &dyn Game) -> Vec<MacroInsertionDto> {
    aeria_se::catalog::INSERTIONS
        .iter()
        .map(|spec| MacroInsertionDto {
            name: spec.name,
            group: spec.group,
            form: match spec.form {
                aeria_se::catalog::InsertionForm::Insert => "insert",
                aeria_se::catalog::InsertionForm::Wrap => "wrap",
                aeria_se::catalog::InsertionForm::Branches => "branches",
            },
            parts: spec.parts,
            rows: spec.rows.map(|sheet| game.rows(sheet)).unwrap_or_default(),
            summary: spec.summary,
        })
        .collect()
}

/// The open project's game, when a project is open.
fn project_source(app: &tauri::AppHandle) -> Option<Arc<GameSource>> {
    let state = app.state::<DesktopState>();
    state.session().ok().map(|session| session.source_handle())
}

/// The family name the renderer registers the game glyph font under.
const GLYPH_FONT_FAMILY: &str = "Aeria Game Glyphs";

#[tauri::command(rename_all = "camelCase")]
/// A TrueType font of the game font's private use glyphs, such as `U+E03C`
/// (the high-quality mark), for showing game text in the interface. The
/// renderer adds it after its own fonts, so it draws only these symbols.
/// Empty without a project or when the game font cannot be read.
///
/// # Errors
///
/// Returns a typed command error when the desktop worker fails.
pub async fn game_glyph_font(app: tauri::AppHandle) -> CommandResult<tauri::ipc::Response> {
    run_blocking(move || {
        let Some(source) = project_source(&app) else {
            return Ok(tauri::ipc::Response::new(Vec::new()));
        };
        let Ok(Some(font)) = source.private_glyphs() else {
            return Ok(tauri::ipc::Response::new(Vec::new()));
        };
        let glyphs: Vec<aeria_fonts::BitmapGlyph<'_>> = font
            .glyphs
            .iter()
            .map(|glyph| aeria_fonts::BitmapGlyph {
                character: glyph.character,
                width: glyph.width,
                height: glyph.height,
                top: font.ascent - glyph.offset_y,
                advance: glyph.advance,
                alpha: &glyph.alpha,
            })
            .collect();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let size = font.size.round().max(1.0) as u32;
        Ok(tauri::ipc::Response::new(aeria_fonts::bitmap_font(
            GLYPH_FONT_FAMILY,
            size,
            font.ascent,
            font.line_height - font.ascent,
            &glyphs,
        )))
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// The inline icon an `<icon>` macro shows: its width and height as
/// little-endian 16-bit numbers, then RGBA pixels row by row. Empty without
/// a project or for an id the game has no icon for.
///
/// # Errors
///
/// Returns a typed command error when the desktop worker fails.
pub async fn game_icon(app: tauri::AppHandle, id: u32) -> CommandResult<tauri::ipc::Response> {
    run_blocking(move || {
        let Some(icon) = project_source(&app).and_then(|source| source.icon(id)) else {
            return Ok(tauri::ipc::Response::new(Vec::new()));
        };
        let mut bytes = Vec::with_capacity(4 + icon.rgba.len());
        for size in [icon.width, icon.height] {
            bytes.extend_from_slice(&u16::try_from(size).unwrap_or(0).to_le_bytes());
        }
        bytes.extend(icon.rgba);
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
}

/// What `macro_view` reads from the open project's game.
trait Game {
    /// The foreground color of a `UIColor` row as `0xRRGGBBAA`.
    fn ui_color(&self, row: u32) -> Option<u32>;
    /// The text of the first column of a sheet row, such as a class name.
    fn row_name(&self, sheet: &str, row: u32) -> Option<String>;
    /// Every row of a sheet whose first column has text, with that text.
    fn rows(&self, sheet: &str) -> Vec<MacroRowDto>;
}

impl Game for Option<Arc<GameSource>> {
    fn ui_color(&self, row: u32) -> Option<u32> {
        self.as_ref()?.ui_color(row)
    }

    fn row_name(&self, sheet: &str, row: u32) -> Option<String> {
        let text = self.as_ref()?.cell_text(sheet, row, 0).ok()??;
        Some(parse(&text).plain_text()).filter(|name| !name.is_empty())
    }

    fn rows(&self, sheet: &str) -> Vec<MacroRowDto> {
        let Some(source) = self.as_ref() else {
            return Vec::new();
        };
        let Ok(aeria_source::SheetLookup::Present(table)) = source.sheet(sheet) else {
            return Vec::new();
        };
        table
            .rows()
            .iter()
            .filter(|row| row.subrow_id == 0)
            .filter_map(|row| {
                let cell = table.cells(row).next()?;
                let name = parse(&cell.text()).plain_text();
                (!name.trim().is_empty()).then_some(MacroRowDto {
                    row: row.row_id,
                    name,
                })
            })
            .collect()
    }
}

fn view(text: &str, game: &dyn Game) -> MacroViewDto {
    let document = parse(text);
    let offsets = Utf16Offsets::new(text);
    let mut tags = Vec::new();
    collect_tags(&document, document.nodes(), &offsets, &mut tags);
    for tag in &mut tags {
        tag.color = tag_color(tag, game);
    }
    let mut conditions = Vec::new();
    collect_conditions(document.nodes(), &document, game, &mut conditions);
    for (from, condition) in conditions {
        let from = offsets.at(from);
        if let Some(tag) = tags
            .iter_mut()
            .find(|tag| tag.from == from && matches!(tag.part, MacroTagPart::Open))
        {
            tag.condition = Some(condition);
        }
    }
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
                color: None,
                condition: None,
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
            color: None,
            condition: None,
        });
    }
}

/// The color an opening `<color>`, `<edge-color>`, `<ui-color>`, or
/// `<ui-edge-color>` tag sets: its `#AARRGGBB` value, or the `UIColor` row.
fn tag_color(tag: &MacroTagDto, game: &dyn Game) -> Option<String> {
    if !matches!(tag.part, MacroTagPart::Open) {
        return None;
    }
    let value = tag.args.first()?.value.trim();
    match tag.name.as_str() {
        "color" | "edge-color" => {
            let argb = u32::from_str_radix(value.strip_prefix('#')?, 16).ok()?;
            color(Some(argb.rotate_left(8)))
        }
        "ui-color" | "ui-edge-color" => color(game.ui_color(value.parse().ok()?)),
        _ => None,
    }
}

/// The conditions of every `<if>` and `<switch>` in `nodes`, by the byte
/// offset of their opening tag.
fn collect_conditions(
    nodes: &[SyntaxNode],
    document: &MacroString,
    game: &dyn Game,
    out: &mut Vec<(usize, ConditionDto)>,
) {
    for node in nodes {
        let SyntaxKind::Macro(syntax) = &node.kind else {
            continue;
        };
        let name = syntax.spec.map_or("", |spec| spec.name);
        if matches!(name, "if" | "switch")
            && let (Some(span), Some(arg)) = (syntax.tags.first(), syntax.args.first())
        {
            out.push((span.start(), condition(arg, document, game)));
        }
        for arg in &syntax.args {
            if let ExprKind::Str(nodes) = &arg.kind {
                collect_conditions(nodes, document, game, out);
            }
        }
    }
}

fn condition(expr: &ExprSyntax, document: &MacroString, game: &dyn Game) -> ConditionDto {
    let ExprKind::Compare(code, left, right) = &expr.kind else {
        return ConditionDto {
            left: operand(expr, None, document, game),
            operator: None,
            right: None,
        };
    };
    let operator = aeria_se::catalog::COMPARISONS
        .iter()
        .find(|spec| spec.code == *code)
        .map(|spec| spec.operator);
    // A number compared with a row global, such as a class, is named by its row.
    let sheet_of = |expr: &ExprSyntax| {
        let parameter = parameter(expr)?;
        aeria_se::catalog::global(parameter.prefix, parameter.index)?.sheet
    };
    ConditionDto {
        left: operand(left, sheet_of(right), document, game),
        operator,
        right: Some(operand(right, sheet_of(left), document, game)),
    }
}

fn operand(
    expr: &ExprSyntax,
    sheet: Option<&str>,
    document: &MacroString,
    game: &dyn Game,
) -> OperandDto {
    match &expr.kind {
        ExprKind::Int(value) => OperandDto::Int {
            value: *value,
            name: sheet.and_then(|sheet| game.row_name(sheet, *value)),
        },
        ExprKind::Nullary(code) => aeria_se::catalog::NULLARY
            .iter()
            .find(|spec| spec.code == *code)
            .map_or_else(
                || OperandDto::Other {
                    text: document
                        .slice(expr.span)
                        .unwrap_or_default()
                        .trim()
                        .to_owned(),
                },
                |spec| OperandDto::Time { name: spec.name },
            ),
        _ => match parameter(expr) {
            Some(parameter) => OperandDto::Parameter {
                code: format!("${}{}", parameter.prefix, parameter.index),
                meaning: aeria_se::catalog::global(parameter.prefix, parameter.index)
                    .map(|global| global.name),
            },
            None => OperandDto::Other {
                text: document
                    .slice(expr.span)
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            },
        },
    }
}

fn color(value: Option<u32>) -> Option<String> {
    value.map(|rgba| format!("#{rgba:08x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestGame;

    impl Game for TestGame {
        fn ui_color(&self, row: u32) -> Option<u32> {
            (row == 504).then_some(0x00CC_22FF)
        }

        fn row_name(&self, sheet: &str, row: u32) -> Option<String> {
            (sheet == "ClassJob" && row == 21).then(|| "monk".to_owned())
        }

        fn rows(&self, sheet: &str) -> Vec<MacroRowDto> {
            if sheet == "Race" {
                vec![MacroRowDto {
                    row: 3,
                    name: "Lalafell".to_owned(),
                }]
            } else {
                Vec::new()
            }
        }
    }

    #[test]
    fn conditions_name_their_globals_and_rows() {
        let view = view(
            "<if ($gn68 == 21)><if ($gn72 >= 94)>a</if></if><switch $n1><case>x</switch><if $gn4>b</if>",
            &TestGame,
        );
        let conditions: Vec<serde_json::Value> = view
            .tags
            .iter()
            .filter_map(|tag| tag.condition.as_ref())
            .map(|condition| serde_json::to_value(condition).expect("json"))
            .collect();
        assert_eq!(conditions.len(), 4);
        assert_eq!(conditions[0]["left"]["meaning"], "class-job");
        assert_eq!(conditions[0]["operator"], "==");
        assert_eq!(conditions[0]["right"]["value"], 21);
        assert_eq!(conditions[0]["right"]["name"], "monk");
        assert_eq!(conditions[1]["left"]["meaning"], "level");
        assert_eq!(conditions[1]["right"]["name"], serde_json::Value::Null);
        assert_eq!(conditions[2]["left"]["code"], "$n1");
        assert_eq!(conditions[2]["operator"], serde_json::Value::Null);
        assert_eq!(conditions[3]["left"]["meaning"], "player-female");
    }

    #[test]
    fn opening_color_tags_carry_their_color() {
        let view = view(
            "<color #FF13212F>x</color><ui-color 504>y</ui-color><ui-color 9>z</ui-color>",
            &TestGame,
        );
        let colors: Vec<Option<&str>> = view.tags.iter().map(|tag| tag.color.as_deref()).collect();
        assert_eq!(
            colors,
            [Some("#13212fff"), None, Some("#00cc22ff"), None, None, None]
        );
    }

    #[test]
    fn a_view_reports_tags_and_diagnostics_in_utf16_offsets() {
        let text = "Ф<if ($n1 == 1)><i>a</i><else>b</if> <sheet Item $n1 0><nope>";
        let view = view(text, &TestGame);
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
    }

    #[test]
    fn insertions_offer_the_rows_of_their_sheet() {
        let insertions = super::insertions(&TestGame);
        let race = insertions
            .iter()
            .find(|insertion| insertion.name == "race-choice")
            .expect("race choice");
        assert_eq!(race.form, "branches");
        assert_eq!(race.parts, ["<if ($gn71 == {row})>", "<else>", "</if>"]);
        assert_eq!(
            race.rows
                .iter()
                .map(|row| (row.row, row.name.as_str()))
                .collect::<Vec<_>>(),
            [(3, "Lalafell")]
        );
        let name = insertions
            .iter()
            .find(|insertion| insertion.name == "player-full-name")
            .expect("full name");
        assert_eq!((name.form, name.parts), ("insert", &["<string $gs1>"][..]));
        assert!(name.rows.is_empty());
    }
}
