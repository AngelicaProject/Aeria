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
        Ok(view(&text, &|row| source.as_ref()?.ui_color(row)))
    })
    .await
}

/// The open project's game, when a project is open.
fn project_source(app: &tauri::AppHandle) -> Option<Arc<GameSource>> {
    let state = app.state::<DesktopState>();
    let project = state.lock_project().ok()?;
    project
        .as_ref()
        .map(aeria_workspace::ProjectSession::source_handle)
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

/// The foreground color of a `UIColor` row as `0xRRGGBBAA`.
type UiColors<'a> = &'a dyn Fn(u32) -> Option<u32>;

fn view(text: &str, ui_colors: UiColors<'_>) -> MacroViewDto {
    let document = parse(text);
    let offsets = Utf16Offsets::new(text);
    let mut tags = Vec::new();
    collect_tags(&document, document.nodes(), &offsets, &mut tags);
    for tag in &mut tags {
        tag.color = tag_color(tag, ui_colors);
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
        });
    }
}

/// The color an opening `<color>`, `<edge-color>`, `<ui-color>`, or
/// `<ui-edge-color>` tag sets: its `#AARRGGBB` value, or the `UIColor` row.
fn tag_color(tag: &MacroTagDto, ui_colors: UiColors<'_>) -> Option<String> {
    if !matches!(tag.part, MacroTagPart::Open) {
        return None;
    }
    let value = tag.args.first()?.value.trim();
    match tag.name.as_str() {
        "color" | "edge-color" => {
            let argb = u32::from_str_radix(value.strip_prefix('#')?, 16).ok()?;
            color(Some(argb.rotate_left(8)))
        }
        "ui-color" | "ui-edge-color" => color(ui_colors(value.parse().ok()?)),
        _ => None,
    }
}

fn color(value: Option<u32>) -> Option<String> {
    value.map(|rgba| format!("#{rgba:08x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_colors(_row: u32) -> Option<u32> {
        None
    }

    #[test]
    fn opening_color_tags_carry_their_color() {
        let view = view(
            "<color #FF13212F>x</color><ui-color 504>y</ui-color><ui-color 9>z</ui-color>",
            &|row| (row == 504).then_some(0x00CC_22FF),
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
        let view = view(text, &no_colors);
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
}
