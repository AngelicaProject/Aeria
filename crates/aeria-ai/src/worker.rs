//! Job units of work: what the localizer gets for one job chunk, and the
//! project access it needs.
//!
//! A chunk of a quest or cutscene sheet becomes the sheet's scene: every
//! line in play order, the chunk's strings marked for translation and the
//! lines around them shown with their current translations as context. A
//! chunk of any other sheet is its strings with their row context. Every
//! line carries the game's other client languages as evidence, and the
//! unit carries the project knowledge that applies to it. The localizer
//! (see [`crate::localizer`]) translates the unit; its results are written
//! through the host's compare-and-set assisted write.

use std::collections::{BTreeMap, BTreeSet};

use crate::dialogue::{DialogueKind, LineRole, SheetDialogue};
use crate::jobs::JobUnit;
use crate::knowledge::{Domain, Knowledge, sheet_domain};
use crate::localizer::{LineKind, ScriptLine, UnitOfWork};
use crate::search::MemoryMatch;
use crate::tools::{
    ContextCell, ProjectFacts, ProjectReader, ReviewLabel, ToolError, UnitLocation, UnitState,
};

/// Lines of a scene shown before the first and after the last string of a
/// chunk, when the scene is larger than the chunk.
const SCENE_MARGIN: usize = 20;
/// Most translation memory matches shown per string.
const MEMORY_PER_LINE: usize = 2;

/// What the localizer needs to translate one string.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitContext {
    pub source: String,
    pub context: Vec<ContextCell>,
    pub current_target: Option<String>,
    pub note: Option<String>,
    /// Existing translations of similar sources, most similar first.
    pub memory: Vec<MemoryMatch>,
}

/// A chunk string as shown while a worker translates it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UnitPreview {
    /// `sheet:row:subrow:column`.
    pub address: String,
    /// The start of the source as plain text, without tags.
    pub source: String,
}

/// Most characters of a unit preview's source.
const PREVIEW_CHARS: usize = 80;

/// Macro text as short plain text: what players read, with whitespace
/// collapsed.
fn plain_preview(text: &str) -> String {
    let plain = aeria_se::parse(text).plain_text();
    let collapsed = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= PREVIEW_CHARS {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(PREVIEW_CHARS - 1).collect();
    cut.push('…');
    cut
}

fn address(location: &UnitLocation) -> String {
    format!(
        "{}:{}:{}:{}",
        location.sheet,
        location.row,
        location.subrow,
        location.column.unwrap_or(0)
    )
}

/// Why a job write did not happen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WriteFailure {
    /// The string changed after the job started.
    Conflict(String),
    Failed(String),
}

/// Project access for jobs, implemented by the desktop.
pub trait JobHost: Send + Sync {
    /// Reads one string's source and context.
    ///
    /// # Errors
    /// Returns an error when the string can no longer be read.
    fn context(&self, location: &UnitLocation) -> Result<UnitContext, ToolError>;

    /// Writes a translation with the review state `review` if the string is
    /// still in the `expected` state.
    ///
    /// # Errors
    /// Returns why nothing was written.
    fn write(
        &self,
        location: &UnitLocation,
        target: &str,
        expected: &UnitState,
        review: ReviewLabel,
    ) -> Result<(), WriteFailure>;

    /// Records an issue for the supervisor.
    fn report(&self, message: &str, location: Option<&UnitLocation>);
}

/// A job chunk made ready for the localizer.
#[derive(Clone, Debug)]
pub struct PreparedUnit {
    /// The unit of work; its script lines name the chunk strings they
    /// translate by index into the chunk.
    pub unit: UnitOfWork,
    /// Chunk strings that cannot be translated, by index, with why.
    pub failed: Vec<(usize, String)>,
    /// The chunk's strings in order, for showing what a worker is on.
    pub previews: Vec<UnitPreview>,
}

/// Prepares a job chunk for the localizer. Strings that can no longer be
/// read, or whose source is malformed, fail at once.
#[must_use]
pub fn prepare_unit(
    units: &[JobUnit],
    host: &dyn JobHost,
    reader: &dyn ProjectReader,
    knowledge: &Knowledge,
    facts: Option<&ProjectFacts>,
    instructions: &str,
) -> PreparedUnit {
    let mut failed = Vec::new();
    let mut tasks: Vec<Option<(UnitContext, Vec<String>)>> = Vec::with_capacity(units.len());
    for (index, unit) in units.iter().enumerate() {
        match host.context(&unit.location) {
            Ok(context) => {
                if let Ok(constructs) = aeria_se::constructs(&context.source) {
                    let legends = constructs.iter().map(aeria_se::Construct::legend).collect();
                    tasks.push(Some((context, legends)));
                } else {
                    failed.push((index, "the source string is malformed".to_owned()));
                    tasks.push(None);
                }
            }
            Err(error) => {
                failed.push((index, error.0));
                tasks.push(None);
            }
        }
    }
    let previews = units
        .iter()
        .zip(&tasks)
        .map(|(unit, task)| UnitPreview {
            address: address(&unit.location),
            source: task
                .as_ref()
                .map(|(context, _)| plain_preview(&context.source))
                .unwrap_or_default(),
        })
        .collect();

    // A chunk never spans sheets. The scene is context only, so a sheet
    // whose dialogue cannot be read is translated as plain strings.
    let sheet = units
        .first()
        .map(|unit| unit.location.sheet.clone())
        .unwrap_or_default();
    let dialogue = reader.dialogue(&sheet).ok().flatten();
    let mut lines = match &dialogue {
        Some(dialogue) => scene_lines(units, &tasks, dialogue, reader, &sheet),
        None => Vec::new(),
    };
    let placed: BTreeSet<usize> = lines.iter().filter_map(|line| line.task).collect();
    for (index, (unit, task)) in units.iter().zip(&tasks).enumerate() {
        if let Some((context, legends)) = task
            && !placed.contains(&index)
        {
            lines.push(task_line(LineKind::Text, unit, context, legends, index));
        }
    }
    for line in &mut lines {
        line.evidence = evidence(reader, &line.address);
    }

    let domains = unit_domains(&sheet, &lines);
    let speakers: Vec<&str> = lines
        .iter()
        .filter_map(|line| match &line.kind {
            LineKind::Speech(speaker) => Some(speaker.as_str()),
            _ => None,
        })
        .collect();
    let knowledge_text = knowledge.prompt_for(
        &domains,
        lines.iter().map(|line| line.source.as_str()),
        speakers,
        &[sheet.as_str()],
    );
    let unit = UnitOfWork {
        title: title(reader, &sheet, dialogue.as_ref()),
        source_language: facts.map_or_else(
            || "the source language".to_owned(),
            |facts| facts.source_language.clone(),
        ),
        target_language: facts
            .and_then(|facts| facts.target_language.clone())
            .unwrap_or_else(|| "the target language".to_owned()),
        knowledge: knowledge_text,
        domains,
        instructions: instructions.to_owned(),
        lines,
    };
    PreparedUnit {
        unit,
        failed,
        previews,
    }
}

fn task_line(
    kind: LineKind,
    unit: &JobUnit,
    context: &UnitContext,
    legends: &[String],
    index: usize,
) -> ScriptLine {
    ScriptLine {
        kind,
        address: address(&unit.location),
        source: context.source.clone(),
        evidence: Vec::new(),
        legends: legends.to_vec(),
        context: context.context.clone(),
        current: context.current_target.clone(),
        note: context.note.clone(),
        memory: context
            .memory
            .iter()
            .take(MEMORY_PER_LINE)
            .cloned()
            .collect(),
        task: Some(index),
    }
}

/// The scene around a dialogue chunk, in play order: the chunk's strings
/// and, as context, the sheet's other lines within [`SCENE_MARGIN`] of them.
fn scene_lines(
    units: &[JobUnit],
    tasks: &[Option<(UnitContext, Vec<String>)>],
    dialogue: &SheetDialogue,
    reader: &dyn ProjectReader,
    sheet: &str,
) -> Vec<ScriptLine> {
    let mut by_cell = BTreeMap::new();
    for (index, (unit, task)) in units.iter().zip(tasks).enumerate() {
        if task.is_some() {
            by_cell.insert(
                (
                    unit.location.row,
                    unit.location.subrow,
                    unit.location.column,
                ),
                index,
            );
        }
    }
    let positions: Vec<usize> = dialogue
        .lines
        .iter()
        .enumerate()
        .filter(|(_, line)| by_cell.contains_key(&(line.row, line.subrow, Some(line.column))))
        .map(|(position, _)| position)
        .collect();
    let (Some(first), Some(last)) = (positions.first(), positions.last()) else {
        return Vec::new();
    };
    let start = first.saturating_sub(SCENE_MARGIN);
    let end = (last + SCENE_MARGIN + 1).min(dialogue.lines.len());
    let mut rows = BTreeMap::new();
    dialogue.lines[start..end]
        .iter()
        .map(|line| {
            let kind = match &line.role {
                LineRole::Journal => LineKind::Journal,
                LineRole::Objective => LineKind::Objective,
                LineRole::Speech(speaker) => LineKind::Speech(speaker.clone()),
                LineRole::Other => LineKind::Other,
            };
            if let Some(&index) = by_cell.get(&(line.row, line.subrow, Some(line.column)))
                && let Some((context, legends)) = &tasks[index]
            {
                return task_line(kind, &units[index], context, legends, index);
            }
            let current = rows
                .entry((line.row, line.subrow))
                .or_insert_with(|| reader.row(sheet, line.row, line.subrow).ok().flatten())
                .as_ref()
                .and_then(|row| row.cells.iter().find(|cell| cell.column == line.column))
                .and_then(|cell| cell.target.clone());
            ScriptLine {
                kind,
                address: format!("{sheet}:{}:{}:{}", line.row, line.subrow, line.column),
                source: line.source.clone(),
                evidence: Vec::new(),
                legends: Vec::new(),
                context: Vec::new(),
                current,
                note: None,
                memory: Vec::new(),
                task: None,
            }
        })
        .collect()
}

/// A line in the game's other client languages, from its address.
fn evidence(reader: &dyn ProjectReader, address: &str) -> Vec<(String, String)> {
    let mut parts = address.rsplitn(4, ':');
    let (Some(column), Some(subrow), Some(row), Some(sheet)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Vec::new();
    };
    let (Ok(column), Ok(subrow), Ok(row)) = (column.parse(), subrow.parse(), row.parse()) else {
        return Vec::new();
    };
    // Evidence is a help; a language that cannot be read is left out.
    reader
        .other_languages(sheet, row, subrow, column)
        .map(|languages| {
            languages
                .into_iter()
                .filter_map(|(code, text)| {
                    text.filter(|text| !text.trim().is_empty())
                        .map(|text| (code, text))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn title(reader: &dyn ProjectReader, sheet: &str, dialogue: Option<&SheetDialogue>) -> String {
    match dialogue.map(|dialogue| (dialogue.kind, dialogue.quest)) {
        Some((DialogueKind::Quest, Some((row, subrow)))) => reader
            .row("Quest", row, subrow)
            .ok()
            .flatten()
            .and_then(|row| row.cells.into_iter().next())
            .map_or_else(
                || format!("Quest sheet {sheet}"),
                |cell| format!("Quest \"{}\" ({sheet})", cell.source),
            ),
        Some((DialogueKind::Quest, None)) => format!("Quest sheet {sheet}"),
        Some((DialogueKind::Cutscene, _)) => format!("Cutscene sheet {sheet}"),
        None => format!("Strings of the sheet {sheet}"),
    }
}

/// The kinds of text in a unit: the roles of a dialogue sheet's lines, or
/// the domain of another sheet.
fn unit_domains(sheet: &str, lines: &[ScriptLine]) -> Vec<Domain> {
    let domain = sheet_domain(sheet);
    if domain != Domain::Dialogue {
        return vec![domain];
    }
    let mut domains = Vec::new();
    for line in lines {
        let domain = match &line.kind {
            LineKind::Journal => Domain::Journal,
            LineKind::Objective => Domain::Objective,
            LineKind::Speech(speaker) if speaker.starts_with("SYSTEM") => Domain::System,
            _ => Domain::Dialogue,
        };
        if !domains.contains(&domain) {
            domains.push(domain);
        }
    }
    domains
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::dialogue::DialogueLine;
    use crate::guidance::{ProjectFile, ProjectGuide};
    use crate::jobs::UnitStatus;
    use crate::knowledge::AgentTexts;
    use crate::tools::{CellSnapshot, RowSnapshot, RowsPage, SheetSummary};

    #[test]
    fn previews_are_short_plain_text() {
        assert_eq!(
            plain_preview("Deal <i>heavy</i>  damage \\< more<num $n1>"),
            "Deal heavy damage < more"
        );
        let long = plain_preview(&"слово ".repeat(40));
        assert_eq!(long.chars().count(), PREVIEW_CHARS);
        assert!(long.ends_with('…'));
    }

    struct Host;

    impl JobHost for Host {
        fn context(&self, location: &UnitLocation) -> Result<UnitContext, ToolError> {
            let source = match location.row {
                1 => "Hi <player-name $n1>!",
                2 => "Bye",
                3 => "Aether",
                4 => "Broken <if",
                _ => return Err(ToolError::new("gone")),
            };
            Ok(UnitContext {
                source: source.to_owned(),
                context: Vec::new(),
                current_target: (location.row == 2).then(|| "Пока".to_owned()),
                note: None,
                memory: Vec::new(),
            })
        }

        fn write(
            &self,
            _: &UnitLocation,
            _: &str,
            _: &UnitState,
            _: ReviewLabel,
        ) -> Result<(), WriteFailure> {
            Ok(())
        }

        fn report(&self, _: &str, _: Option<&UnitLocation>) {}
    }

    struct Reader {
        dialogue: bool,
    }

    impl ProjectReader for Reader {
        fn facts(&self) -> Result<ProjectFacts, ToolError> {
            Err(ToolError::new("unused"))
        }
        fn other_languages(
            &self,
            _: &str,
            row: u32,
            _: u16,
            _: u32,
        ) -> Result<Vec<(String, Option<String>)>, ToolError> {
            Ok(vec![
                ("ja".to_owned(), Some(format!("行{row}"))),
                (
                    "fr".to_owned(),
                    (row == 1).then(|| "Prêt<if $gn4>e</if> ?".to_owned()),
                ),
            ])
        }
        fn sheets(&self) -> Result<Vec<SheetSummary>, ToolError> {
            Ok(Vec::new())
        }
        fn rows(&self, _: &str, _: Option<(u32, u16)>, _: u32) -> Result<RowsPage, ToolError> {
            Err(ToolError::new("unused"))
        }
        fn row(&self, _: &str, row: u32, subrow: u16) -> Result<Option<RowSnapshot>, ToolError> {
            Ok(Some(RowSnapshot {
                row,
                subrow,
                cells: vec![CellSnapshot {
                    column: 0,
                    source: "Well met.".to_owned(),
                    formatting_only: false,
                    target: (row == 0).then(|| "Приветствую.".to_owned()),
                    review_state: None,
                    note: None,
                    unit_id: None,
                    constructs: Vec::new(),
                    malformed: false,
                    glossary: Vec::new(),
                }],
                context: Vec::new(),
            }))
        }
        fn pending_changes(&self) -> Result<Value, ToolError> {
            Ok(Value::Null)
        }
        fn unit_history(&self, _: &str, _: u32) -> Result<Value, ToolError> {
            Ok(Value::Null)
        }
        fn navigate(&self, _: &UnitLocation) -> Result<(), ToolError> {
            Ok(())
        }
        fn project_file(&self, _: ProjectFile) -> Result<Option<String>, ToolError> {
            Ok(None)
        }
        fn dialogue(&self, _: &str) -> Result<Option<SheetDialogue>, ToolError> {
            let line = |row: u32, role: LineRole, source: &str| DialogueLine {
                row,
                subrow: 0,
                column: 0,
                key: format!("TEXT_{row}"),
                role,
                source: source.to_owned(),
            };
            let speech = |speaker: &str| LineRole::Speech(speaker.to_owned());
            Ok(self.dialogue.then(|| SheetDialogue {
                kind: DialogueKind::Cutscene,
                quest: None,
                lines: vec![
                    line(0, speech("URIANGER"), "Well met."),
                    line(1, speech("ALPHINAUD"), "Hi <player-name $n1>!"),
                    line(2, LineRole::Journal, "Bye"),
                    line(3, LineRole::Objective, "Aether"),
                ],
            }))
        }
        fn speakers(&self, _: &str) -> Result<Vec<(String, usize)>, ToolError> {
            Ok(Vec::new())
        }
        fn speaker_lines(
            &self,
            _: &str,
            _: usize,
            _: usize,
        ) -> Result<(usize, Vec<UnitLocation>), ToolError> {
            Ok((0, Vec::new()))
        }
    }

    fn job_unit(seq: u64, row: u32) -> JobUnit {
        JobUnit {
            seq,
            chunk: 0,
            location: UnitLocation {
                sheet: "cut_scene/000/Test".to_owned(),
                row,
                subrow: 0,
                column: Some(0),
            },
            status: UnitStatus::Running,
            attempts: 1,
            message: None,
            expected: UnitState {
                target: None,
                review_state: None,
            },
        }
    }

    fn knowledge() -> Knowledge {
        Knowledge::from_parts(
            ProjectGuide::from_files(
                Ok(Some("Be brief.".to_owned())),
                Ok(Some(
                    "term,translation\nAether,Эфир\nMoogle,Моогл\n".to_owned(),
                )),
            )
            .with_voices(Ok(Some("## ALPHINAUD\nPolite and bookish.\n".to_owned()))),
            &AgentTexts {
                style: Some("## journal\nUse вы.\n## items\nShort.".to_owned()),
                ..AgentTexts::default()
            },
        )
    }

    #[test]
    fn a_dialogue_chunk_becomes_its_scene_with_context_evidence_and_knowledge() {
        let units = [
            job_unit(10, 1),
            job_unit(11, 2),
            job_unit(12, 3),
            job_unit(13, 4),
            job_unit(14, 9),
        ];
        let prepared = prepare_unit(
            &units,
            &Host,
            &Reader { dialogue: true },
            &knowledge(),
            None,
            "Keep it short.",
        );
        assert_eq!(
            prepared.failed,
            vec![
                (3, "the source string is malformed".to_owned()),
                (4, "gone".to_owned())
            ]
        );
        let unit = &prepared.unit;
        assert_eq!(unit.title, "Cutscene sheet cut_scene/000/Test");
        let tasks: Vec<Option<usize>> = unit.lines.iter().map(|line| line.task).collect();
        assert_eq!(tasks, vec![None, Some(0), Some(1), Some(2)]);
        assert_eq!(unit.lines[0].kind, LineKind::Speech("URIANGER".to_owned()));
        assert_eq!(unit.lines[0].current.as_deref(), Some("Приветствую."));
        assert_eq!(unit.lines[2].kind, LineKind::Journal);
        assert_eq!(unit.lines[2].current.as_deref(), Some("Пока"));
        assert!(unit.lines[1].gender_marked());
        assert_eq!(
            unit.lines[1].evidence[0],
            ("ja".to_owned(), "行1".to_owned())
        );
        assert!(unit.knowledge.contains("Be brief."));
        assert!(unit.knowledge.contains("## journal\nUse вы."));
        assert!(!unit.knowledge.contains("Short."));
        assert_eq!(
            unit.domains,
            vec![Domain::Dialogue, Domain::Journal, Domain::Objective]
        );
        assert!(unit.knowledge.contains("- Aether → Эфир"));
        assert!(!unit.knowledge.contains("Moogle"));
        assert!(unit.knowledge.contains("## ALPHINAUD\nPolite and bookish."));
        assert_eq!(unit.instructions, "Keep it short.");
        assert_eq!(prepared.previews[0].source, "Hi !");
    }

    #[test]
    fn other_sheets_are_their_strings() {
        let units = [job_unit(10, 1), job_unit(11, 3)];
        let prepared = prepare_unit(
            &units,
            &Host,
            &Reader { dialogue: false },
            &Knowledge::default(),
            None,
            "",
        );
        let kinds: Vec<&LineKind> = prepared.unit.lines.iter().map(|line| &line.kind).collect();
        assert_eq!(kinds, vec![&LineKind::Text, &LineKind::Text]);
        assert_eq!(
            prepared.unit.title,
            "Strings of the sheet cut_scene/000/Test"
        );
        assert!(prepared.unit.knowledge.is_empty());
        assert_eq!(prepared.unit.domains, vec![Domain::Dialogue]);
    }
}
