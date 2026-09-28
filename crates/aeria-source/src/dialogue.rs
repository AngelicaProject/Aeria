//! The dialogue structure of quest and cutscene sheets, read from row keys.
//!
//! Quest sheets (`quest/…/<Id>`) and cutscene sheets (`cut_scene/…/<Id>`)
//! key every row as `TEXT_<ID>_<rest>`, where `<ID>` is the sheet's own
//! name. The rest names what the row holds: a journal entry, an objective,
//! or a line with a speaker label. Keys are read conservatively: a key that
//! matches no rule is [`LineRole::Other`], never a guess. See
//! `docs/architecture/source.md`.

use crate::sheet::SourceSheet;

/// The kinds of sheet whose row keys describe dialogue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DialogueKind {
    /// `quest/…`: a quest's journal, objectives, and dialogue.
    Quest,
    /// `cut_scene/…`: the lines of cutscenes.
    Cutscene,
}

impl DialogueKind {
    /// The kind of a sheet, by its name; `None` for other sheets.
    #[must_use]
    pub fn of(sheet: &str) -> Option<Self> {
        if sheet.starts_with("quest/") {
            Some(Self::Quest)
        } else if sheet.starts_with("cut_scene/") {
            Some(Self::Cutscene)
        } else {
            None
        }
    }
}

/// What one row of a dialogue sheet holds, according to its key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineRole {
    /// `SEQ_<n>`: the quest journal entry of one quest step.
    Journal,
    /// `TODO_<n>`: an objective.
    Objective,
    /// A line with a speaker label, such as `URIANGER`, `SYSTEM`, or `A1`.
    /// The label is the game's internal name; it is not always a character.
    Speech { speaker: String },
    /// Any other key, such as battle talk or gauge names.
    Other,
}

/// One row of a dialogue sheet that has text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DialogueLine {
    pub row_id: u32,
    pub subrow_id: u16,
    /// The column of `text`.
    pub column: u32,
    pub key: String,
    pub role: LineRole,
    /// The macro text of the row's first non-empty String cell other than
    /// the key.
    pub text: String,
}

/// The rows of a dialogue sheet that have text, in row order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Dialogue {
    pub kind: DialogueKind,
    pub lines: Vec<DialogueLine>,
}

impl Dialogue {
    /// Reads the dialogue of a keyed quest or cutscene sheet; `None` for any
    /// other sheet. A row whose String cells other than the key are all
    /// empty is left out.
    #[must_use]
    pub fn of(sheet: &SourceSheet) -> Option<Self> {
        let kind = DialogueKind::of(sheet.name())?;
        let keys = sheet.row_keys()?;
        let lines = sheet
            .rows()
            .iter()
            .filter_map(|row| {
                let cell = sheet
                    .cells(row)
                    .find(|cell| cell.column != keys.column() && !cell.bytes.is_empty())?;
                let key = keys.key_of(row.row_id, row.subrow_id)?;
                Some(DialogueLine {
                    row_id: row.row_id,
                    subrow_id: row.subrow_id,
                    column: cell.column,
                    key: key.to_owned(),
                    role: line_role(sheet.name(), key),
                    text: cell.text(),
                })
            })
            .collect();
        Some(Self { kind, lines })
    }
}

/// The last segment of a sheet name: `ManFst004_00124` for
/// `quest/001/ManFst004_00124`.
#[must_use]
pub fn sheet_id(sheet: &str) -> &str {
    sheet.rsplit('/').next().unwrap_or(sheet)
}

/// What a row key of a dialogue sheet names.
///
/// After the `TEXT_<ID>_` prefix, which is compared ignoring ASCII case:
///
/// - `SEQ_<n>` is a journal entry and `TODO_<n>` an objective;
/// - `<label>_<n>_<n>` and `<n>_<label>` are speech by `<label>`;
/// - anything else, or a key without the prefix, is [`LineRole::Other`].
///
/// A label is one or more `_`-separated segments of ASCII letters and
/// digits, none of them only digits.
#[must_use]
pub fn line_role(sheet: &str, key: &str) -> LineRole {
    let prefix = format!("TEXT_{}_", sheet_id(sheet));
    let Some(rest) = key
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(&prefix))
        .and_then(|_| key.get(prefix.len()..))
    else {
        return LineRole::Other;
    };
    let segments: Vec<&str> = rest.split('_').collect();
    match segments.as_slice() {
        ["SEQ", step] if is_number(step) => return LineRole::Journal,
        ["TODO", step] if is_number(step) => return LineRole::Objective,
        _ => {}
    }
    let speaker = match segments.as_slice() {
        [label @ .., first, second] if is_number(first) && is_number(second) => label,
        // Unvoiced choices in voiced cutscenes: `Q1_000_001_NONE_VOICE`.
        [label @ .., first, second, "NONE", "VOICE"] if is_number(first) && is_number(second) => {
            label
        }
        [first, label @ ..] if is_number(first) => label,
        _ => return LineRole::Other,
    };
    if speaker.is_empty() || !speaker.iter().all(|segment| is_label(segment)) {
        return LineRole::Other;
    }
    LineRole::Speech {
        speaker: speaker.join("_"),
    }
}

fn is_number(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_label(segment: &str) -> bool {
    !segment.is_empty()
        && segment.bytes().all(|byte| byte.is_ascii_alphanumeric())
        && !is_number(segment)
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUEST: &str = "quest/001/ManFst004_00124";
    const CUTSCENE: &str = "cut_scene/024/VoiceMan_02400";

    fn speech(speaker: &str) -> LineRole {
        LineRole::Speech {
            speaker: speaker.to_owned(),
        }
    }

    #[test]
    fn quest_keys_name_journal_objectives_and_speakers() {
        let role = |key: &str| line_role(QUEST, key);
        assert_eq!(role("TEXT_MANFST004_00124_SEQ_00"), LineRole::Journal);
        assert_eq!(role("TEXT_MANFST004_00124_TODO_12"), LineRole::Objective);
        assert_eq!(
            role("TEXT_MANFST004_00124_MIOUNNE_000_1"),
            speech("MIOUNNE")
        );
        assert_eq!(role("TEXT_MANFST004_00124_SYSTEM_000_48"), speech("SYSTEM"));
        assert_eq!(role("TEXT_MANFST004_00124_Q1_000_000"), speech("Q1"));
        assert_eq!(
            role("TEXT_MANFST004_00124_AMHGARANJY_GEVA_000_110"),
            speech("AMHGARANJY_GEVA")
        );
        assert_eq!(
            role("TEXT_MANFST004_00124_900190_KANESENNA"),
            speech("KANESENNA")
        );
        assert_eq!(
            role("TEXT_ManFst004_00124_GEROLT_000_146"),
            speech("GEROLT"),
            "the prefix ignores case"
        );
    }

    #[test]
    fn cutscene_keys_put_the_speaker_after_the_line_number() {
        let role = |key: &str| line_role(CUTSCENE, key);
        assert_eq!(role("TEXT_VOICEMAN_02400_000010_ILBERD"), speech("ILBERD"));
        assert_eq!(
            role("TEXT_VOICEMAN_02400_001550_SYSTEM_NONE_VOICE"),
            speech("SYSTEM_NONE_VOICE")
        );
        assert_eq!(role("TEXT_VOICEMAN_02400_A1_000_003"), speech("A1"));
        assert_eq!(
            role("TEXT_VOICEMAN_02400_Q4_000_001_NONE_VOICE"),
            speech("Q4")
        );
        assert_eq!(
            role("TEXT_VOICEMAN_02400_A4_000_002_NONE_VOICE"),
            speech("A4")
        );
    }

    #[test]
    fn other_keys_are_never_guessed() {
        for key in [
            "TEXT_MANFST004_00124_QIB_001_XRHUNTIA_BATTLETALK_16",
            "TEXT_MANFST004_00124_QIB_PRINCIPIA_BATTLETALK_05",
            "TEXT_MANFST004_00124_POP_MESSAGE",
            "TEXT_MANFST004_00124_TANSUI__BATTLETALK_007_002",
            "TEXT_MANFST004_00124_04214_Q5_000_000",
            "TEXT_MANFST004_00124_SEQ_ONE",
            "TEXT_MANFST004_00124_000_000",
            "TEXT_OTHER_00001_MIOUNNE_000_1",
            "TEXT",
        ] {
            assert_eq!(line_role(QUEST, key), LineRole::Other, "{key}");
        }
        assert_eq!(
            line_role(CUTSCENE, "TEXT_VOICEMAN_02400_E00200_MIDGARDSORMR"),
            LineRole::Other
        );
        assert_eq!(
            line_role(CUTSCENE, "TEXT_VOICEMAN_02400_001561_SYSTEM_060"),
            LineRole::Other
        );
    }

    #[test]
    fn only_quest_and_cutscene_sheets_are_dialogue() {
        assert_eq!(DialogueKind::of(QUEST), Some(DialogueKind::Quest));
        assert_eq!(DialogueKind::of(CUTSCENE), Some(DialogueKind::Cutscene));
        assert_eq!(DialogueKind::of("custom/000/CmnDefCabinet_00082"), None);
        assert_eq!(DialogueKind::of("Quest"), None);
        assert_eq!(sheet_id(QUEST), "ManFst004_00124");
    }
}
