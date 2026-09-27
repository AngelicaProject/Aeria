//! Dialogue context: the scene around a quest or cutscene line, the quest it
//! belongs to, and the voice profiles of its speakers.
//!
//! The host reads a sheet's dialogue structure from the game's row keys
//! (`aeria_source::Dialogue`, see `docs/architecture/source.md`); this module
//! shapes it for Angelica's tools, worker chunks, and one-string drafts.
//! Everything here is context for the model and is never recorded.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chat::ToolDefinition;
use crate::guidance::{ProjectFile, ProjectGuide};
use crate::tools::{
    ProjectReader, ProjectWriter, ProposalOutcome, ReviewLabel, ToolError, bound_text,
};
use crate::voices::{MAX_PROFILE_SPEAKERS, MAX_PROFILES_PER_CHANGE, VoiceProfile, change_voices};

/// Most lines `dialogue_context` returns on each side of a line.
pub const MAX_NEIGHBOUR_LINES: usize = 40;
/// Most journal entries or objectives `dialogue_context` returns.
pub const MAX_QUEST_ENTRIES: usize = 24;
/// Most lines one `speaker_lines` call returns.
pub const MAX_SPEAKER_LINES: usize = 30;
/// Most speakers one `list_speakers` call returns.
pub const MAX_LISTED_SPEAKERS: usize = 200;
/// Longest line text in a scene summary.
const BRIEF_LINE_CHARS: usize = 300;
/// Most journal entries or objectives in a scene summary.
const BRIEF_QUEST_ENTRIES: usize = 12;
/// Most voice profiles in a scene summary.
const BRIEF_PROFILES: usize = 8;

/// The kind of a dialogue sheet.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogueKind {
    Quest,
    Cutscene,
}

/// What one row of a dialogue sheet holds, according to its key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineRole {
    Journal,
    Objective,
    /// A line with a speaker label, such as `URIANGER`, `SYSTEM`, or `A1`.
    Speech(String),
    Other,
}

/// One row of a dialogue sheet that has text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogueLine {
    pub row: u32,
    pub subrow: u16,
    pub column: u32,
    pub key: String,
    pub role: LineRole,
    /// The source macro text.
    pub source: String,
}

impl DialogueLine {
    #[must_use]
    pub fn speaker(&self) -> Option<&str> {
        match &self.role {
            LineRole::Speech(speaker) => Some(speaker),
            _ => None,
        }
    }

    /// Speech and other lines; journal entries and objectives are not.
    #[must_use]
    pub const fn is_spoken(&self) -> bool {
        matches!(self.role, LineRole::Speech(_) | LineRole::Other)
    }
}

/// The dialogue structure of one quest or cutscene sheet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetDialogue {
    pub kind: DialogueKind,
    /// The `Quest` row that names the quest, when one row does.
    pub quest: Option<(u32, u16)>,
    /// Rows with text, in row order.
    pub lines: Vec<DialogueLine>,
}

impl SheetDialogue {
    /// The index of one row's line.
    #[must_use]
    pub fn position(&self, row: u32, subrow: u16) -> Option<usize> {
        self.lines
            .iter()
            .position(|line| line.row == row && line.subrow == subrow)
    }

    /// The speaker label of one row, when it is speech.
    #[must_use]
    pub fn speaker(&self, row: u32, subrow: u16) -> Option<&str> {
        self.lines[self.position(row, subrow)?].speaker()
    }

    /// Up to `count` spoken lines before line `index`, in order.
    fn spoken_before(&self, index: usize, count: usize) -> Vec<&DialogueLine> {
        let mut lines: Vec<&DialogueLine> = self.lines[..index]
            .iter()
            .rev()
            .filter(|line| line.is_spoken())
            .take(count)
            .collect();
        lines.reverse();
        lines
    }

    fn entries(&self, role: &LineRole) -> impl Iterator<Item = &DialogueLine> {
        self.lines.iter().filter(move |line| line.role == *role)
    }
}

/// The read tools for dialogue and voice profiles.
#[must_use]
pub fn dialogue_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "dialogue_context",
            description: "The scene around a line of a quest or cutscene sheet (quest/… or cut_scene/…): the quest's name, journal, and objectives, the spoken lines before and after it in sheet order with their speaker labels and translations, and the voice profiles of those speakers. Speaker labels come from the row keys and are the game's internal names; SYSTEM is system text, and labels such as Q1 or A1 are usually a player choice's question and answers. Sheet order follows the script but not its branches, and who is addressed is not recorded: infer it and say so. Set other_languages to see the line in the game's other client languages, which often settles an ambiguous source.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "sheet": { "type": "string" },
                    "row": { "type": "integer", "minimum": 0 },
                    "subrow": { "type": "integer", "minimum": 0, "description": "Defaults to 0." },
                    "before": { "type": "integer", "minimum": 0, "maximum": MAX_NEIGHBOUR_LINES, "description": "Spoken lines before; defaults to 8." },
                    "after": { "type": "integer", "minimum": 0, "maximum": MAX_NEIGHBOUR_LINES, "description": "Spoken lines after; defaults to 4." },
                    "other_languages": { "type": "boolean", "description": "Add the line in the game's other client languages." },
                },
                "required": ["sheet", "row"],
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "list_speakers",
            description: "Speaker labels of quest and cutscene speech, the most lines first, with whether each has a voice profile: to find the characters that matter most, for example to write voice profiles. Labels are the game's internal names; SYSTEM, Q1, A1, and labels with a number are rarely characters. The first call reads every dialogue sheet and can take a few seconds.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Only labels that contain this text." },
                    "without_profile": { "type": "boolean", "description": "Only speakers without a voice profile." },
                    "offset": { "type": "integer", "minimum": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_LISTED_SPEAKERS, "description": "Defaults to 50." },
                },
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "speaker_lines",
            description: "Lines of one speaker label across every quest and cutscene, in sheet order, with their translations and the speaker's voice profile: to learn how a character speaks and how the project has translated them. Set spread to sample lines evenly across the whole game instead of reading a page, since a character's voice can change over the story. An unknown label returns similar labels. The first call reads every dialogue sheet and can take a few seconds.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "speaker": { "type": "string", "description": "A speaker label, such as URIANGER." },
                    "offset": { "type": "integer", "minimum": 0, "description": "Not used with spread." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_SPEAKER_LINES, "description": "Defaults to 15." },
                    "spread": { "type": "boolean", "description": "Sample lines evenly across all of the speaker's lines." },
                },
                "required": ["speaker"],
                "additionalProperties": false,
            }),
        },
        ToolDefinition {
            name: "get_voices",
            description: "Character voice profiles from aeria-voices.md: the profiles of the given speaker labels, or the list of every speaker with a profile.",
            parameters: json!({
                "type": "object",
                "properties": { "speakers": { "type": "array", "maxItems": 50, "items": { "type": "string" } } },
                "additionalProperties": false,
            }),
        },
    ]
}

/// The write tool for voice profiles.
#[must_use]
pub fn voice_change_definition() -> ToolDefinition {
    ToolDefinition {
        name: "propose_voice_profile",
        description: "Proposes character voice profiles in aeria-voices.md as one change, and can remove speakers from profiles. A profile names a character by the speaker labels of the game's dialogue keys and says in Markdown how the character speaks in the target language: register, forms of address, pronouns, archaisms, verbal tics, with short examples. Each profile replaces the one profile that names any of its speakers, or is added. The user always approves voice profile changes, and a change applies only to the file it was proposed against: put every profile of a turn in one call, and wait for the user to apply it before proposing more. Base a profile on the character's lines (speaker_lines with spread) and existing translations.",
        parameters: json!({
            "type": "object",
            "properties": {
                "profiles": {
                    "type": "array",
                    "maxItems": MAX_PROFILES_PER_CHANGE,
                    "items": {
                        "type": "object",
                        "properties": {
                            "speakers": { "type": "array", "minItems": 1, "maxItems": MAX_PROFILE_SPEAKERS, "items": { "type": "string" }, "description": "Speaker labels of one character, such as [\"URIANGER\"]." },
                            "profile": { "type": "string", "description": "The complete profile text in Markdown." },
                        },
                        "required": ["speakers", "profile"],
                        "additionalProperties": false,
                    },
                },
                "remove": { "type": "array", "maxItems": MAX_PROFILE_SPEAKERS, "items": { "type": "string" }, "description": "Speaker labels to remove from their profiles." },
            },
            "additionalProperties": false,
        }),
    }
}

fn plain_cut(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    let mut cut: String = text.chars().take(limit - 1).collect();
    cut.push('…');
    cut
}

/// A line's current translation: target and review state.
fn translation(
    reader: &dyn ProjectReader,
    sheet: &str,
    line: &DialogueLine,
) -> Result<(Option<String>, Option<ReviewLabel>), ToolError> {
    Ok(reader
        .row(sheet, line.row, line.subrow)?
        .and_then(|row| {
            row.cells
                .into_iter()
                .find(|cell| cell.column == line.column)
        })
        .map(|cell| (cell.target, cell.review_state))
        .unwrap_or_default())
}

fn line_value(
    reader: &dyn ProjectReader,
    sheet: &str,
    line: &DialogueLine,
) -> Result<Value, ToolError> {
    let mut source = line.source.clone();
    bound_text(&mut source);
    let mut value = json!({
        "row": line.row,
        "subrow": line.subrow,
        "column": line.column,
        "key": line.key,
        "source": source,
    });
    if let Some(speaker) = line.speaker() {
        value["speaker"] = json!(speaker);
    }
    let (target, review_state) = translation(reader, sheet, line)?;
    if let Some(mut target) = target {
        bound_text(&mut target);
        value["target"] = json!(target);
        value["reviewState"] = json!(review_state);
    }
    Ok(value)
}

/// The quest's name and its translation, from its `Quest` row.
fn quest_value(reader: &dyn ProjectReader, quest: (u32, u16)) -> Result<Value, ToolError> {
    let mut value = json!({ "sheet": "Quest", "row": quest.0, "subrow": quest.1 });
    if let Some(cell) = reader
        .row("Quest", quest.0, quest.1)?
        .and_then(|row| row.cells.into_iter().next())
    {
        value["column"] = json!(cell.column);
        value["name"] = json!(cell.source);
        if let Some(target) = cell.target {
            value["translation"] = json!(target);
        }
    }
    Ok(value)
}

fn profile_value(profile: &VoiceProfile) -> Value {
    json!({ "speakers": profile.speakers, "profile": profile.text_for_prompt() })
}

/// Voice profiles of the speakers, and the speakers without one.
fn voices_value<'a>(guide: &ProjectGuide, speakers: impl IntoIterator<Item = &'a str>) -> Value {
    let mut unique: Vec<&str> = Vec::new();
    for speaker in speakers {
        if !unique.contains(&speaker) {
            unique.push(speaker);
        }
    }
    let profiles = guide
        .voices
        .as_ref()
        .map(|voices| voices.for_speakers(unique.iter().copied()))
        .unwrap_or_default();
    let missing: Vec<&str> = unique
        .into_iter()
        .filter(|speaker| {
            !profiles
                .iter()
                .any(|profile| profile.speakers.iter().any(|known| known == speaker))
        })
        .collect();
    json!({
        "profiles": profiles.into_iter().map(profile_value).collect::<Vec<_>>(),
        "speakersWithoutProfile": missing,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DialogueArgs {
    sheet: String,
    row: u32,
    subrow: Option<u16>,
    before: Option<usize>,
    after: Option<usize>,
    #[serde(default)]
    other_languages: bool,
}

fn not_dialogue(sheet: &str) -> ToolError {
    ToolError::new(format!(
        "{sheet} is not a quest or cutscene sheet with row keys; dialogue context exists only for quest/… and cut_scene/… sheets"
    ))
}

/// `dialogue_context`: the scene around one line.
pub(crate) fn dialogue_context(
    reader: &dyn ProjectReader,
    guide: &ProjectGuide,
    args: &DialogueArgs,
) -> Result<Value, ToolError> {
    let subrow = args.subrow.unwrap_or(0);
    let dialogue = reader
        .dialogue(&args.sheet)?
        .ok_or_else(|| not_dialogue(&args.sheet))?;
    let index = dialogue.position(args.row, subrow).ok_or_else(|| {
        ToolError::new(format!(
            "{}:{}:{subrow} is not a line with text in this sheet",
            args.sheet, args.row
        ))
    })?;
    let line = &dialogue.lines[index];
    let mut result = json!({
        "sheet": args.sheet,
        "kind": dialogue.kind,
        "line": line_value(reader, &args.sheet, line)?,
    });
    if let Some(quest) = dialogue.quest {
        result["quest"] = quest_value(reader, quest)?;
    }
    if dialogue.kind == DialogueKind::Quest {
        for (field, role) in [
            ("journal", LineRole::Journal),
            ("objectives", LineRole::Objective),
        ] {
            let entries = dialogue
                .entries(&role)
                .take(MAX_QUEST_ENTRIES)
                .map(|entry| line_value(reader, &args.sheet, entry))
                .collect::<Result<Vec<_>, _>>()?;
            result[field] = json!(entries);
        }
    }
    let mut speakers: Vec<&str> = line.speaker().into_iter().collect();
    if line.is_spoken() {
        let before =
            dialogue.spoken_before(index, args.before.unwrap_or(8).min(MAX_NEIGHBOUR_LINES));
        let after: Vec<&DialogueLine> = dialogue.lines[index + 1..]
            .iter()
            .filter(|line| line.is_spoken())
            .take(args.after.unwrap_or(4).min(MAX_NEIGHBOUR_LINES))
            .collect();
        speakers.extend(
            before
                .iter()
                .chain(&after)
                .filter_map(|line| line.speaker()),
        );
        result["before"] = json!(
            before
                .iter()
                .map(|line| line_value(reader, &args.sheet, line))
                .collect::<Result<Vec<_>, _>>()?
        );
        result["after"] = json!(
            after
                .iter()
                .map(|line| line_value(reader, &args.sheet, line))
                .collect::<Result<Vec<_>, _>>()?
        );
    }
    if args.other_languages {
        let languages: serde_json::Map<String, Value> = reader
            .other_languages(&args.sheet, line.row, line.subrow, line.column)?
            .into_iter()
            .map(|(language, text)| {
                let text = text.map_or(Value::Null, |mut text| {
                    bound_text(&mut text);
                    Value::String(text)
                });
                (language, text)
            })
            .collect();
        result["line"]["otherLanguages"] = Value::Object(languages);
    }
    result["voices"] = voices_value(guide, speakers);
    Ok(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpeakerLinesArgs {
    speaker: String,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
    #[serde(default)]
    spread: bool,
}

/// Positions of `count` lines spread evenly over `total`, first included.
fn spread_positions(total: usize, count: usize) -> Vec<usize> {
    let count = count.min(total);
    (0..count).map(|index| index * total / count).collect()
}

/// `speaker_lines`: one speaker's lines across quests and cutscenes.
pub(crate) fn speaker_lines(
    reader: &dyn ProjectReader,
    guide: &ProjectGuide,
    args: &SpeakerLinesArgs,
) -> Result<Value, ToolError> {
    let speaker = args.speaker.trim().to_ascii_uppercase();
    if speaker.is_empty() {
        return Err(ToolError::new("name a speaker label, such as URIANGER"));
    }
    let limit = args.limit.unwrap_or(15).clamp(1, MAX_SPEAKER_LINES);
    let (total, located) = if args.spread {
        let total = reader.speaker_lines(&speaker, 0, 0)?.0;
        let mut located = Vec::new();
        for position in spread_positions(total, limit) {
            located.extend(
                reader
                    .speaker_lines(&speaker, position, 1)?
                    .1
                    .into_iter()
                    .map(|location| (position, location)),
            );
        }
        (total, located)
    } else {
        let (total, page) = reader.speaker_lines(&speaker, args.offset, limit)?;
        (total, (args.offset..).zip(page).collect())
    };
    if total == 0 {
        // Labels that start with the query first, then the most lines.
        let mut similar = reader.speakers(&speaker)?;
        similar.sort_by(|(a, a_lines), (b, b_lines)| {
            (!a.starts_with(&speaker), std::cmp::Reverse(a_lines), a).cmp(&(
                !b.starts_with(&speaker),
                std::cmp::Reverse(b_lines),
                b,
            ))
        });
        let similar: Vec<Value> = similar
            .into_iter()
            .take(20)
            .map(|(label, lines)| json!({ "speaker": label, "lines": lines }))
            .collect();
        return Ok(json!({ "speaker": speaker, "total": 0, "similar": similar }));
    }
    let mut lines = Vec::with_capacity(located.len());
    let mut current: Option<(String, Option<SheetDialogue>)> = None;
    for (position, location) in located {
        if current
            .as_ref()
            .is_none_or(|(sheet, _)| *sheet != location.sheet)
        {
            current = Some((location.sheet.clone(), reader.dialogue(&location.sheet)?));
        }
        let Some((sheet, Some(dialogue))) = &current else {
            continue;
        };
        let Some(index) = dialogue.position(location.row, location.subrow) else {
            continue;
        };
        let mut value = line_value(reader, sheet, &dialogue.lines[index])?;
        value["sheet"] = json!(sheet);
        value["index"] = json!(position);
        lines.push(value);
    }
    let mut result = json!({ "speaker": speaker, "total": total, "lines": lines });
    let next = args.offset + limit;
    if !args.spread && next < total {
        result["nextOffset"] = json!(next);
    }
    if let Some(profile) = guide
        .voices
        .as_ref()
        .and_then(|voices| voices.find(&speaker))
    {
        result["voice"] = profile_value(profile);
    }
    Ok(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListSpeakersArgs {
    #[serde(default)]
    query: String,
    #[serde(default)]
    without_profile: bool,
    #[serde(default)]
    offset: usize,
    limit: Option<usize>,
}

/// `list_speakers`: speaker labels, the most lines first.
pub(crate) fn list_speakers(
    reader: &dyn ProjectReader,
    guide: &ProjectGuide,
    args: &ListSpeakersArgs,
) -> Result<Value, ToolError> {
    let profiled = |speaker: &str| {
        guide
            .voices
            .as_ref()
            .is_some_and(|voices| voices.find(speaker).is_some())
    };
    let mut speakers: Vec<(String, usize)> = reader
        .speakers(args.query.trim())?
        .into_iter()
        .filter(|(speaker, _)| !args.without_profile || !profiled(speaker))
        .collect();
    speakers.sort_by(|(a, a_lines), (b, b_lines)| b_lines.cmp(a_lines).then_with(|| a.cmp(b)));
    let limit = args.limit.unwrap_or(50).clamp(1, MAX_LISTED_SPEAKERS);
    let total = speakers.len();
    let page: Vec<Value> = speakers
        .into_iter()
        .skip(args.offset)
        .take(limit)
        .map(|(speaker, lines)| {
            json!({ "speaker": speaker, "lines": lines, "hasProfile": profiled(&speaker) })
        })
        .collect();
    let next = args.offset + page.len();
    Ok(json!({
        "total": total,
        "speakers": page,
        "nextOffset": (next < total).then_some(next),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VoicesArgs {
    #[serde(default)]
    speakers: Vec<String>,
}

/// `get_voices`: profiles of some speakers, or every profiled speaker.
pub(crate) fn get_voices(guide: &ProjectGuide, args: &VoicesArgs) -> Value {
    let voices = guide.voices.as_ref();
    let mut result = if args.speakers.is_empty() {
        json!({ "speakers": voices.map(|voices| voices.speakers()).unwrap_or_default() })
    } else {
        voices_value(guide, args.speakers.iter().map(String::as_str))
    };
    if voices.is_none() {
        result["note"] = json!("the project has no voice profiles yet");
    }
    if !guide.problems.is_empty() {
        result["problems"] = json!(guide.problems);
    }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VoiceChangeArgs {
    #[serde(default)]
    profiles: Vec<ProfileArgs>,
    #[serde(default)]
    remove: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileArgs {
    speakers: Vec<String>,
    profile: String,
}

/// `propose_voice_profile`: sets profiles and removes speakers as one
/// change, for the user's approval.
pub(crate) fn propose_voice_profile(
    reader: &dyn ProjectReader,
    writer: &dyn ProjectWriter,
    args: &VoiceChangeArgs,
) -> Result<Value, ToolError> {
    if args.profiles.is_empty() && args.remove.is_empty() {
        return Err(ToolError::new("set profiles or remove speakers"));
    }
    if args.profiles.len() > MAX_PROFILES_PER_CHANGE {
        return Err(ToolError::new(format!(
            "propose at most {MAX_PROFILES_PER_CHANGE} profiles at once"
        )));
    }
    if args.remove.len() > MAX_PROFILE_SPEAKERS {
        return Err(ToolError::new(format!(
            "remove at most {MAX_PROFILE_SPEAKERS} speakers at once"
        )));
    }
    if args
        .profiles
        .iter()
        .any(|profile| profile.speakers.is_empty())
    {
        return Err(ToolError::new("each profile names at least one speaker"));
    }
    let set: Vec<(Vec<String>, String)> = args
        .profiles
        .iter()
        .map(|profile| (profile.speakers.clone(), profile.profile.clone()))
        .collect();
    let before = reader.project_file(ProjectFile::Voices)?;
    let after = change_voices(before.as_deref(), &set, &args.remove).map_err(ToolError::new)?;
    if after.len() as u64 > ProjectFile::Voices.max_bytes() {
        return Err(ToolError::new("the voice profiles would be too long"));
    }
    if before.as_deref() == Some(after.as_str()) {
        return Err(ToolError::new("aeria-voices.md would not change"));
    }
    let file = ProjectFile::Voices.file_name();
    Ok(
        match writer.propose_file_change(crate::tools::FileChange {
            file: ProjectFile::Voices,
            before,
            after,
        })? {
            ProposalOutcome::Pending { proposal_id } => json!({
                "status": "awaitingApproval",
                "proposalId": proposal_id,
                "file": file,
                "profiles": set.len(),
                "note": "Wait until the user applies this change before proposing more voice profiles; a later change made before then would conflict with it.",
            }),
            ProposalOutcome::Applied => json!({ "status": "applied", "file": file }),
            ProposalOutcome::Conflict { message } | ProposalOutcome::Failed { message } => {
                json!({ "status": "failed", "errors": [message] })
            }
        },
    )
}

/// A plain-text summary of the scene before some lines of a dialogue
/// sheet, for requests without tools: the quest name, journal, and
/// objectives, then up to `before` spoken lines before the first of `rows`
/// with their speakers and current translations, then the voice profiles
/// of the speakers of those lines and of `rows`.
///
/// # Errors
///
/// Returns an error when the project cannot be read.
pub fn scene_brief(
    reader: &dyn ProjectReader,
    guide: &ProjectGuide,
    sheet: &str,
    dialogue: &SheetDialogue,
    rows: &[(u32, u16)],
    before: usize,
) -> Result<String, ToolError> {
    let mut brief = String::new();
    match (dialogue.kind, dialogue.quest) {
        (DialogueKind::Quest, Some(quest)) => {
            let value = quest_value(reader, quest)?;
            let name = value["name"].as_str().unwrap_or("unknown");
            let _ = write!(brief, "Quest \"{name}\"");
            if let Some(translation) = value["translation"].as_str() {
                let _ = write!(brief, " (translated as \"{translation}\")");
            }
            let _ = writeln!(brief, ", sheet {sheet}.");
        }
        (DialogueKind::Quest, None) => {
            let _ = writeln!(brief, "Quest sheet {sheet}.");
        }
        (DialogueKind::Cutscene, _) => {
            let _ = writeln!(brief, "Cutscene sheet {sheet}.");
        }
    }
    for (title, role) in [
        ("Quest journal", LineRole::Journal),
        ("Objectives", LineRole::Objective),
    ] {
        let entries: Vec<&DialogueLine> =
            dialogue.entries(&role).take(BRIEF_QUEST_ENTRIES).collect();
        if !entries.is_empty() {
            let _ = writeln!(brief, "{title}:");
            for entry in entries {
                let _ = writeln!(brief, "- {}", plain_cut(&entry.source, BRIEF_LINE_CHARS));
            }
        }
    }
    let mut speakers: Vec<&str> = rows
        .iter()
        .filter_map(|&(row, subrow)| dialogue.speaker(row, subrow))
        .collect();
    let first = rows
        .iter()
        .filter_map(|&(row, subrow)| dialogue.position(row, subrow))
        .min();
    if let Some(first) = first {
        let earlier = dialogue.spoken_before(first, before);
        if !earlier.is_empty() {
            let _ = writeln!(brief, "Lines before, in sheet order:");
            for line in earlier {
                let text = plain_cut(&line.source, BRIEF_LINE_CHARS);
                let _ = write!(brief, "- {}: {text}", line.speaker().unwrap_or(&line.key));
                if let (Some(target), _) = translation(reader, sheet, line)? {
                    let _ = write!(brief, " → {}", plain_cut(&target, BRIEF_LINE_CHARS));
                }
                brief.push('\n');
                speakers.extend(line.speaker());
            }
        }
    }
    if let Some(voices) = &guide.voices {
        let profiles = voices.for_speakers(speakers);
        if !profiles.is_empty() {
            brief.push_str("Voice profiles, to follow for how these characters speak:\n");
            for profile in profiles.into_iter().take(BRIEF_PROFILES) {
                brief.push_str(&profile.prompt_block());
                brief.push('\n');
            }
        }
    }
    Ok(brief)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::tools::{
        CellSnapshot, FileChange, ProjectFacts, Proposal, ReadTools, ReviewBatch, RowSnapshot,
        RowsPage, SheetSummary, TranslatableUnit, UnitLocation,
    };

    const SHEET: &str = "quest/001/ManFst004_00124";
    const VOICES: &str =
        "## URIANGER\nАрхаичная речь, обращается на «вы».\n\n## MIOUNNE\nТёплая, деловая.\n";

    fn line(row: u32, key: &str, role: LineRole, source: &str) -> DialogueLine {
        DialogueLine {
            row,
            subrow: 0,
            column: 1,
            key: key.to_owned(),
            role,
            source: source.to_owned(),
        }
    }

    fn speech(speaker: &str) -> LineRole {
        LineRole::Speech(speaker.to_owned())
    }

    fn quest_dialogue() -> SheetDialogue {
        SheetDialogue {
            kind: DialogueKind::Quest,
            quest: Some((7, 0)),
            lines: vec![
                line(0, "TEXT_X_SEQ_00", LineRole::Journal, "Miounne has tasks."),
                line(
                    24,
                    "TEXT_X_TODO_00",
                    LineRole::Objective,
                    "Visit the guild.",
                ),
                line(
                    48,
                    "TEXT_X_MIOUNNE_000_1",
                    speech("MIOUNNE"),
                    "Let us begin.",
                ),
                line(49, "TEXT_X_SYSTEM_000_2", speech("SYSTEM"), "You nod."),
                line(50, "TEXT_X_QIB_BATTLETALK_05", LineRole::Other, "Hah!"),
                line(
                    51,
                    "TEXT_X_URIANGER_000_3",
                    speech("URIANGER"),
                    "You again??",
                ),
                line(52, "TEXT_X_MIOUNNE_000_4", speech("MIOUNNE"), "Indeed."),
            ],
        }
    }

    #[derive(Default)]
    struct Reader {
        changes: Mutex<Vec<FileChange>>,
    }

    fn cell(column: u32, source: &str, target: Option<&str>) -> CellSnapshot {
        CellSnapshot {
            column,
            source: source.to_owned(),
            formatting_only: false,
            target: target.map(str::to_owned),
            review_state: target.map(|_| ReviewLabel::Draft),
            note: None,
            unit_id: None,
            tagged: None,
            tags: Vec::new(),
            untaggable: false,
            glossary: Vec::new(),
        }
    }

    impl ProjectReader for Reader {
        fn facts(&self) -> Result<ProjectFacts, ToolError> {
            Err(ToolError::new("unused"))
        }

        fn sheets(&self) -> Result<Vec<SheetSummary>, ToolError> {
            Ok(Vec::new())
        }

        fn rows(&self, _: &str, _: Option<(u32, u16)>, _: u32) -> Result<RowsPage, ToolError> {
            Err(ToolError::new("unused"))
        }

        fn row(
            &self,
            sheet: &str,
            row: u32,
            subrow: u16,
        ) -> Result<Option<RowSnapshot>, ToolError> {
            let cells = match (sheet, row) {
                ("Quest", 7) => vec![cell(0, "Close to Home", Some("Как дома"))],
                (SHEET, 48) => vec![cell(1, "Let us begin.", Some("Начнём же."))],
                (SHEET, 49 | 51 | 52) => vec![cell(1, "…", None)],
                _ => return Ok(None),
            };
            Ok(Some(RowSnapshot {
                row,
                subrow,
                cells,
                context: Vec::new(),
            }))
        }

        fn other_languages(
            &self,
            _: &str,
            _: u32,
            _: u16,
            _: u32,
        ) -> Result<Vec<(String, Option<String>)>, ToolError> {
            Ok(vec![
                ("ja".to_owned(), Some("またお前か？".to_owned())),
                ("de".to_owned(), Some("Ihr schon wieder?".to_owned())),
                ("fr".to_owned(), None),
            ])
        }

        fn pending_changes(&self) -> Result<Value, ToolError> {
            Ok(json!([]))
        }

        fn unit_history(&self, _: &str, _: u32) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        fn navigate(&self, _: &UnitLocation) -> Result<(), ToolError> {
            Ok(())
        }

        fn project_file(&self, file: ProjectFile) -> Result<Option<String>, ToolError> {
            Ok((file == ProjectFile::Voices).then(|| VOICES.to_owned()))
        }

        fn dialogue(&self, sheet: &str) -> Result<Option<SheetDialogue>, ToolError> {
            Ok((sheet == SHEET).then(quest_dialogue))
        }

        fn speakers(&self, query: &str) -> Result<Vec<(String, usize)>, ToolError> {
            Ok([
                ("AURIAUNE", 21),
                ("MIOUNNE", 2),
                ("URIANGER", 2077),
                ("URIBOY", 5),
            ]
            .into_iter()
            .filter(|(label, _)| label.contains(query))
            .map(|(label, lines)| (label.to_owned(), lines))
            .collect())
        }

        fn speaker_lines(
            &self,
            speaker: &str,
            offset: usize,
            limit: usize,
        ) -> Result<(usize, Vec<UnitLocation>), ToolError> {
            let rows: &[u32] = match speaker {
                "MIOUNNE" => &[48, 52],
                "URIANGER" => &[51; 10],
                _ => &[],
            };
            Ok((
                rows.len(),
                rows.iter()
                    .skip(offset)
                    .take(limit)
                    .map(|&row| UnitLocation {
                        sheet: SHEET.to_owned(),
                        row,
                        subrow: 0,
                        column: None,
                    })
                    .collect(),
            ))
        }
    }

    impl ProjectWriter for Reader {
        fn translatable_unit(&self, _: &UnitLocation) -> Result<TranslatableUnit, ToolError> {
            Err(ToolError::new("unused"))
        }

        fn submit(&self, _: Vec<Proposal>) -> Result<Vec<ProposalOutcome>, ToolError> {
            Ok(Vec::new())
        }

        fn propose_file_change(&self, change: FileChange) -> Result<ProposalOutcome, ToolError> {
            self.changes.lock().expect("lock").push(change);
            Ok(ProposalOutcome::Pending {
                proposal_id: "p1".to_owned(),
            })
        }

        fn propose_review(&self, _: ReviewBatch) -> Result<ProposalOutcome, ToolError> {
            Err(ToolError::new("unused"))
        }
    }

    fn run(tools: &ReadTools<'_>, name: &str, arguments: &Value) -> Value {
        let output = tools.execute(name, &arguments.to_string());
        let value: Value = serde_json::from_str(&output.content).expect("json");
        assert!(!output.is_error, "{value}");
        value
    }

    #[test]
    fn a_line_comes_with_its_quest_scene_speakers_and_voices() {
        let reader = Reader::default();
        let tools = ReadTools::new(&reader);
        let scene = run(
            &tools,
            "dialogue_context",
            &json!({ "sheet": SHEET, "row": 51, "before": 2, "other_languages": true }),
        );
        assert_eq!(scene["kind"], "quest");
        assert_eq!(scene["quest"]["name"], "Close to Home");
        assert_eq!(scene["quest"]["translation"], "Как дома");
        assert_eq!(scene["journal"][0]["source"], "Miounne has tasks.");
        assert_eq!(scene["objectives"][0]["row"], 24);
        assert_eq!(scene["line"]["speaker"], "URIANGER");
        assert_eq!(scene["line"]["otherLanguages"]["de"], "Ihr schon wieder?");
        let before: Vec<u64> = scene["before"]
            .as_array()
            .expect("before")
            .iter()
            .map(|line| line["row"].as_u64().expect("row"))
            .collect();
        assert_eq!(before, [49, 50], "journal and objectives are not spoken");
        assert_eq!(scene["before"][1]["key"], "TEXT_X_QIB_BATTLETALK_05");
        assert!(scene["before"][1].get("speaker").is_none());
        assert_eq!(scene["after"][0]["speaker"], "MIOUNNE");
        let profiled: Vec<&str> = scene["voices"]["profiles"]
            .as_array()
            .expect("profiles")
            .iter()
            .map(|profile| profile["speakers"][0].as_str().expect("speaker"))
            .collect();
        assert_eq!(profiled, ["URIANGER", "MIOUNNE"]);
        assert_eq!(scene["voices"]["speakersWithoutProfile"], json!(["SYSTEM"]));

        let first = run(
            &tools,
            "dialogue_context",
            &json!({ "sheet": SHEET, "row": 48 }),
        );
        assert_eq!(first["line"]["target"], "Начнём же.");
        assert_eq!(first["before"], json!([]));

        let journal = run(
            &tools,
            "dialogue_context",
            &json!({ "sheet": SHEET, "row": 0 }),
        );
        assert!(
            journal.get("before").is_none(),
            "a journal entry has no neighbours"
        );

        for arguments in [
            json!({ "sheet": "Item", "row": 1 }),
            json!({ "sheet": SHEET, "row": 30 }),
        ] {
            let output = tools.execute("dialogue_context", &arguments.to_string());
            assert!(output.is_error, "{arguments}");
        }
    }

    #[test]
    fn speaker_lines_show_translations_and_suggest_labels() {
        let reader = Reader::default();
        let tools = ReadTools::new(&reader);
        let lines = run(
            &tools,
            "speaker_lines",
            &json!({ "speaker": "miounne", "limit": 1 }),
        );
        assert_eq!(lines["total"], 2);
        assert_eq!(lines["lines"][0]["sheet"], SHEET);
        assert_eq!(lines["lines"][0]["target"], "Начнём же.");
        assert_eq!(lines["nextOffset"], 1);
        assert_eq!(lines["voice"]["profile"], "Тёплая, деловая.");
        let unknown = run(&tools, "speaker_lines", &json!({ "speaker": "URI" }));
        assert_eq!(unknown["total"], 0);
        let similar: Vec<&str> = unknown["similar"]
            .as_array()
            .expect("similar")
            .iter()
            .map(|label| label["speaker"].as_str().expect("label"))
            .collect();
        assert_eq!(similar, ["URIANGER", "URIBOY", "AURIAUNE"]);

        let spread = run(
            &tools,
            "speaker_lines",
            &json!({ "speaker": "URIANGER", "limit": 3, "spread": true }),
        );
        let positions: Vec<u64> = spread["lines"]
            .as_array()
            .expect("lines")
            .iter()
            .map(|line| line["index"].as_u64().expect("index"))
            .collect();
        assert_eq!(positions, [0, 3, 6]);
        assert!(spread.get("nextOffset").is_none());
        assert_eq!(spread_positions(2, 30), [0, 1]);

        let listed = run(&tools, "list_speakers", &json!({ "limit": 2 }));
        assert_eq!(listed["total"], 4);
        assert_eq!(
            listed["speakers"][0],
            json!({ "speaker": "URIANGER", "lines": 2077, "hasProfile": true })
        );
        assert_eq!(listed["speakers"][1]["speaker"], "AURIAUNE");
        assert_eq!(listed["nextOffset"], 2);
        let missing = run(
            &tools,
            "list_speakers",
            &json!({ "without_profile": true, "query": "URI" }),
        );
        let labels: Vec<&str> = missing["speakers"]
            .as_array()
            .expect("speakers")
            .iter()
            .map(|speaker| speaker["speaker"].as_str().expect("label"))
            .collect();
        assert_eq!(labels, ["AURIAUNE", "URIBOY"]);

        let all = run(&tools, "get_voices", &json!({}));
        assert_eq!(all["speakers"], json!(["URIANGER", "MIOUNNE"]));
    }

    #[test]
    fn voice_profiles_are_proposed_as_file_changes() {
        let reader = Reader::default();
        let tools = ReadTools::with_writer(&reader, &reader);
        let proposed = run(
            &tools,
            "propose_voice_profile",
            &json!({ "profiles": [
                { "speakers": ["thancred"], "profile": "Ироничный, на «ты»." },
                { "speakers": ["URIANGER"], "profile": "Высокий стиль." },
            ] }),
        );
        assert_eq!(proposed["status"], "awaitingApproval");
        assert_eq!(proposed["profiles"], 2);
        let changes = reader.changes.lock().expect("lock");
        assert_eq!(changes.len(), 1, "one change for every profile");
        assert_eq!(changes[0].file, ProjectFile::Voices);
        assert_eq!(changes[0].before.as_deref(), Some(VOICES));
        assert!(
            changes[0]
                .after
                .starts_with("## URIANGER\n\nВысокий стиль.\n")
        );
        assert!(
            changes[0]
                .after
                .ends_with("## THANCRED\n\nИроничный, на «ты».\n")
        );
        drop(changes);
        for arguments in [
            json!({ "profiles": [{ "speakers": ["URIANGER"], "profile": " " }] }),
            json!({ "profiles": [{ "speakers": [], "profile": "Text." }] }),
            json!({}),
        ] {
            let output = tools.execute("propose_voice_profile", &arguments.to_string());
            assert!(output.is_error, "{arguments}");
        }
        let chat = ReadTools::new(&reader).execute(
            "propose_voice_profile",
            &json!({ "remove": ["URIANGER"] }).to_string(),
        );
        assert!(chat.is_error, "Chat mode cannot change the project");
    }

    #[test]
    fn a_scene_brief_leads_into_a_chunk() {
        let reader = Reader::default();
        let guide = ProjectGuide::default().with_voices(Ok(Some(VOICES.to_owned())));
        let brief =
            scene_brief(&reader, &guide, SHEET, &quest_dialogue(), &[(51, 0)], 2).expect("brief");
        assert!(brief.starts_with(
            "Quest \"Close to Home\" (translated as \"Как дома\"), sheet quest/001/ManFst004_00124.\n"
        ));
        assert!(brief.contains("Quest journal:\n- Miounne has tasks.\n"));
        assert!(brief.contains(
            "Lines before, in sheet order:\n- SYSTEM: You nod.\n- TEXT_X_QIB_BATTLETALK_05: Hah!\n"
        ));
        assert!(brief.contains("<voice speakers=\"URIANGER\">"));
        assert!(
            !brief.contains("MIOUNNE\">"),
            "only speakers of the scene are shown"
        );
    }
}
