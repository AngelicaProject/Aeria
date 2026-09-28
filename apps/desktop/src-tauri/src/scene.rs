//! The scene view's data: the dialogue structure of a quest or cutscene
//! sheet and the scenes traced from its quest's script. Everything here is
//! context for translators and is never recorded.

use std::collections::HashMap;

use aeria_source::{
    Availability, ChoiceKind, Comparison, Constant, Dialogue, DialogueKind, FlowNode, Guard,
    LineRole, Operand, OptionLabel, QuestScript, Test,
};
use serde::Serialize;

use crate::dto::SourceBindingDto;

/// The dialogue structure of a quest or cutscene sheet, for the editor's
/// scene view.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetDialogueDto {
    pub kind: DialogueKindDto,
    /// The quest's name from its `Quest` row, when one row names it.
    pub quest: Option<QuestNameDto>,
    /// Other quest sheets whose `Quest` row has the same name, in
    /// sheet-name order.
    pub versions: Vec<String>,
    /// Rows with text, in row order.
    pub lines: Vec<DialogueLineDto>,
    /// The scenes of the quest's script, in scene order; `None` for a
    /// cutscene, a quest without a script, or a script that cannot be read.
    pub scenes: Option<Vec<SceneFlowDto>>,
    /// Why the quest's script could not be read.
    pub script_error: Option<String>,
    /// Every cutscene file that names lines of the sheet, in `Cutscene` row
    /// order.
    pub cutscenes: Vec<IndexedCutsceneDto>,
}

/// A cutscene file and the sheet's lines it names.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedCutsceneDto {
    /// The cutscene's `Cutscene` row.
    pub row: u32,
    /// The cutscene's path, such as `ffxiv/clsarc/clsarc00110/clsarc00110`.
    pub path: String,
    /// Keys of the sheet's lines, in row order.
    pub lines: Vec<String>,
    /// The quests' scenes that play it, in sheet-name order.
    pub plays: Vec<CutscenePlayDto>,
}

/// A scene or handler of a quest's scripts that plays a cutscene.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutscenePlayDto {
    /// The quest's sheet.
    pub quest: String,
    /// The quest's name in the source language.
    pub name: Option<String>,
    pub scene: Option<u32>,
    pub handler: Option<String>,
    pub script: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogueKindDto {
    Quest,
    Cutscene,
}

/// The name of a quest and its translation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestNameDto {
    pub source_binding: SourceBindingDto,
    pub source_macro: String,
    pub target_macro: Option<String>,
}

/// One row of a dialogue sheet that has text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogueLineDto {
    pub source_binding: SourceBindingDto,
    pub role: DialogueRoleDto,
    /// The speaker label of speech, such as `URIANGER`, `SYSTEM`, or `A1`.
    pub speaker: Option<String>,
    /// What the row key says after the `TEXT_<ID>_` prefix.
    pub key: String,
    /// The source macro text, for rows the editor has no string for.
    pub source_macro: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DialogueRoleDto {
    Journal,
    Objective,
    Speech,
    Other,
}

/// One traced scene of a quest script.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneFlowDto {
    /// The number of `OnScene<number>`; `None` for another function of the
    /// script, such as an event handler.
    pub scene: Option<u32>,
    /// The name another function is assigned to, such as
    /// `GetBalloonTalkArgs`.
    pub handler: Option<String>,
    /// The quest's battle script it belongs to, such as `ClsRog250Btl`;
    /// `None` for the quest's own script.
    pub script: Option<String>,
    /// `false` when the scene lists its lines in code order, without
    /// branches.
    pub traced: bool,
    pub nodes: Vec<FlowNodeDto>,
}

/// One step of a scene. Line keys are given like [`DialogueLineDto::key`],
/// after the sheet's `TEXT_<ID>_` prefix; a key of another sheet is whole.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FlowNodeDto {
    Line {
        key: String,
    },
    Choice {
        id: u32,
        choice: ChoiceKindDto,
        /// The keys of the possible questions.
        prompts: Vec<String>,
        options: Vec<ChoiceOptionDto>,
    },
    Branch {
        condition: GuardDto,
        then: Vec<FlowNodeDto>,
        otherwise: Vec<FlowNodeDto>,
    },
    Loop {
        id: u32,
        body: Vec<FlowNodeDto>,
    },
    Repeat {
        loop_id: u32,
    },
    Cutscene {
        name: Option<String>,
        /// The cutscene's path, when its variable resolves.
        path: Option<String>,
        /// Keys of the sheet's lines the cutscene names, in row order.
        lines: Vec<String>,
        /// Other dialogue sheets with lines the cutscene names.
        sheets: Vec<String>,
    },
    Cancelled,
    Accepted,
    Completed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChoiceKindDto {
    QuestOffer,
    YesNo,
    Menu,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceOptionDto {
    pub label: OptionLabelDto,
    pub available: AvailabilityDto,
    pub then: Vec<FlowNodeDto>,
}

/// When the player can pick an answer; `never` is shown grayed out.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AvailabilityDto {
    Always,
    Never,
    When { guard: GuardDto },
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OptionLabelDto {
    Accept,
    Decline,
    Yes,
    No,
    Text {
        key: String,
    },
    /// An answer the script passes from elsewhere.
    Script,
}

/// What a branch tests: one condition, or several joined with `or` (`any`)
/// or `and` (`all`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GuardDto {
    Test { subject: OperandDto, test: TestDto },
    Any { guards: Vec<GuardDto> },
    All { guards: Vec<GuardDto> },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TestDto {
    Truthy {
        value: bool,
    },
    Compare {
        comparison: ComparisonDto,
        value: OperandDto,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ComparisonDto {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OperandDto {
    Answer {
        choice: u32,
    },
    Call {
        function: Option<String>,
        arguments: Vec<OperandDto>,
    },
    Number {
        value: f64,
    },
    Boolean {
        value: bool,
    },
    Nil,
    String {
        value: String,
    },
    Field {
        name: String,
    },
    /// A quest a script variable names.
    Quest {
        variable: String,
        row: u32,
        name: Option<String>,
        sheet: Option<String>,
    },
    Global {
        name: String,
    },
    OneOf {
        operands: Vec<OperandDto>,
    },
    Unknown,
}

impl SheetDialogueDto {
    pub(crate) fn new(
        sheet_name: &str,
        dialogue: Dialogue,
        quest: Option<QuestNameDto>,
        versions: Vec<String>,
        script: Result<Option<QuestScript>, String>,
        cutscenes: Vec<aeria_source::CutsceneLines>,
        mut plays: HashMap<u32, Vec<aeria_source::CutscenePlay>>,
    ) -> Self {
        let keys = Keys::new(sheet_name);
        let (scenes, script_error) = match script {
            Ok(script) => (
                script.map(|script| {
                    script
                        .scenes
                        .into_iter()
                        .map(|scene| SceneFlowDto {
                            scene: scene.scene,
                            handler: scene.handler,
                            script: scene.script,
                            traced: scene.traced,
                            nodes: keys.nodes(scene.nodes),
                        })
                        .collect()
                }),
                None,
            ),
            Err(error) => (None, Some(error)),
        };
        Self {
            kind: match dialogue.kind {
                DialogueKind::Quest => DialogueKindDto::Quest,
                DialogueKind::Cutscene => DialogueKindDto::Cutscene,
            },
            quest,
            versions,
            lines: dialogue
                .lines
                .into_iter()
                .map(|line| {
                    let (role, speaker) = match line.role {
                        LineRole::Journal => (DialogueRoleDto::Journal, None),
                        LineRole::Objective => (DialogueRoleDto::Objective, None),
                        LineRole::Speech { speaker } => (DialogueRoleDto::Speech, Some(speaker)),
                        LineRole::Other => (DialogueRoleDto::Other, None),
                    };
                    DialogueLineDto {
                        source_binding: SourceBindingDto {
                            sheet_name: sheet_name.to_owned(),
                            row_id: line.row_id,
                            subrow_id: line.subrow_id,
                            column_index: line.column,
                        },
                        role,
                        speaker,
                        key: keys.short(&line.key),
                        source_macro: line.text,
                    }
                })
                .collect(),
            scenes,
            script_error,
            cutscenes: cutscenes
                .into_iter()
                .map(|cutscene| IndexedCutsceneDto {
                    row: cutscene.row,
                    lines: cutscene.keys.iter().map(|key| keys.short(key)).collect(),
                    plays: plays
                        .remove(&cutscene.row)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|play| CutscenePlayDto {
                            quest: play.quest,
                            name: play.name,
                            scene: play.scene,
                            handler: play.handler,
                            script: play.script,
                        })
                        .collect(),
                    path: cutscene.path,
                })
                .collect(),
        }
    }
}

/// Shortens a sheet's row keys to what follows its `TEXT_<ID>_` prefix.
struct Keys {
    prefix: String,
}

impl Keys {
    fn new(sheet_name: &str) -> Self {
        Self {
            prefix: format!("TEXT_{}_", aeria_source::sheet_id(sheet_name)),
        }
    }

    /// The key after the prefix, compared ignoring case; a key without the
    /// prefix is kept whole.
    fn short(&self, key: &str) -> String {
        key.get(..self.prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(&self.prefix))
            .and_then(|_| key.get(self.prefix.len()..))
            .unwrap_or(key)
            .to_owned()
    }

    fn nodes(&self, nodes: Vec<FlowNode>) -> Vec<FlowNodeDto> {
        nodes.into_iter().map(|node| self.node(node)).collect()
    }

    fn node(&self, node: FlowNode) -> FlowNodeDto {
        match node {
            FlowNode::Line { key } => FlowNodeDto::Line {
                key: self.short(&key),
            },
            FlowNode::Choice(choice) => FlowNodeDto::Choice {
                id: choice.id,
                choice: match choice.kind {
                    ChoiceKind::QuestOffer => ChoiceKindDto::QuestOffer,
                    ChoiceKind::YesNo => ChoiceKindDto::YesNo,
                    ChoiceKind::Menu => ChoiceKindDto::Menu,
                },
                prompts: choice.prompts.iter().map(|key| self.short(key)).collect(),
                options: choice
                    .options
                    .into_iter()
                    .map(|option| ChoiceOptionDto {
                        label: match option.label {
                            OptionLabel::Accept => OptionLabelDto::Accept,
                            OptionLabel::Decline => OptionLabelDto::Decline,
                            OptionLabel::Yes => OptionLabelDto::Yes,
                            OptionLabel::No => OptionLabelDto::No,
                            OptionLabel::Text(key) => OptionLabelDto::Text {
                                key: self.short(&key),
                            },
                            OptionLabel::Script => OptionLabelDto::Script,
                        },
                        available: match option.available {
                            Availability::Always => AvailabilityDto::Always,
                            Availability::Never => AvailabilityDto::Never,
                            Availability::When(condition) => AvailabilityDto::When {
                                guard: guard(condition),
                            },
                            Availability::Unknown => AvailabilityDto::Unknown,
                        },
                        then: self.nodes(option.then),
                    })
                    .collect(),
            },
            FlowNode::Branch {
                condition,
                then,
                otherwise,
            } => FlowNodeDto::Branch {
                condition: guard(condition),
                then: self.nodes(then),
                otherwise: self.nodes(otherwise),
            },
            FlowNode::Loop { id, body } => FlowNodeDto::Loop {
                id,
                body: self.nodes(body),
            },
            FlowNode::Repeat { loop_id } => FlowNodeDto::Repeat { loop_id },
            FlowNode::Cutscene {
                name,
                path,
                lines,
                sheets,
                ..
            } => FlowNodeDto::Cutscene {
                name,
                path,
                lines: lines.iter().map(|key| self.short(key)).collect(),
                sheets,
            },
            FlowNode::Cancelled => FlowNodeDto::Cancelled,
            FlowNode::Accepted => FlowNodeDto::Accepted,
            FlowNode::Completed => FlowNodeDto::Completed,
        }
    }
}

fn guard(guard: Guard) -> GuardDto {
    match guard {
        Guard::Test(condition) => GuardDto::Test {
            subject: operand(condition.subject),
            test: match condition.test {
                Test::Truthy(value) => TestDto::Truthy { value },
                Test::Compare(comparison, value) => TestDto::Compare {
                    comparison: match comparison {
                        Comparison::Eq => ComparisonDto::Eq,
                        Comparison::Ne => ComparisonDto::Ne,
                        Comparison::Lt => ComparisonDto::Lt,
                        Comparison::Le => ComparisonDto::Le,
                        Comparison::Gt => ComparisonDto::Gt,
                        Comparison::Ge => ComparisonDto::Ge,
                    },
                    value: operand(value),
                },
            },
        },
        Guard::Any(guards) => GuardDto::Any {
            guards: guards.into_iter().map(self::guard).collect(),
        },
        Guard::All(guards) => GuardDto::All {
            guards: guards.into_iter().map(self::guard).collect(),
        },
    }
}

fn operand(operand: Operand) -> OperandDto {
    match operand {
        Operand::Answer(choice) => OperandDto::Answer { choice },
        Operand::Call {
            function,
            arguments,
        } => OperandDto::Call {
            function,
            arguments: arguments.into_iter().map(self::operand).collect(),
        },
        Operand::Constant(Constant::Number(value)) => OperandDto::Number { value },
        Operand::Constant(Constant::Boolean(value)) => OperandDto::Boolean { value },
        Operand::Constant(Constant::Nil) => OperandDto::Nil,
        Operand::Constant(Constant::String(value)) => OperandDto::String { value },
        Operand::Field(name) => OperandDto::Field { name },
        Operand::Quest(quest) => OperandDto::Quest {
            variable: quest.variable,
            row: quest.row,
            name: quest.name,
            sheet: quest.sheet,
        },
        Operand::Global(name) => OperandDto::Global { name },
        Operand::OneOf(operands) => OperandDto::OneOf {
            operands: operands.into_iter().map(self::operand).collect(),
        },
        Operand::Unknown => OperandDto::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use aeria_source::{
        Availability, Choice, ChoiceKind, ChoiceOption, Condition, Dialogue, DialogueKind,
        FlowNode, Guard, Operand, OptionLabel, QuestScript, SceneFlow, Test,
    };
    use std::collections::HashMap;

    use serde_json::json;

    use super::SheetDialogueDto;

    #[test]
    fn scenes_serialize_with_short_keys_and_tagged_nodes() {
        let sheet = "quest/000/ClsArc000_00021";
        let script = QuestScript {
            scenes: vec![SceneFlow {
                scene: Some(0),
                handler: None,
                script: None,
                traced: true,
                nodes: vec![
                    FlowNode::Choice(Choice {
                        id: 4,
                        kind: ChoiceKind::YesNo,
                        prompts: vec!["TEXT_CLSARC000_00021_Q1_000_1".to_owned()],
                        options: vec![ChoiceOption {
                            label: OptionLabel::Text("text_ClsArc000_00021_A1_000_1".to_owned()),
                            available: Availability::Always,
                            then: vec![FlowNode::Repeat { loop_id: 2 }],
                        }],
                    }),
                    FlowNode::Branch {
                        condition: Guard::Test(Condition {
                            subject: Operand::Call {
                                function: Some("GetSex".to_owned()),
                                arguments: vec![],
                            },
                            test: Test::Truthy(false),
                        }),
                        then: vec![FlowNode::Line {
                            key: "TEXT_OTHER_00001_A_000_1".to_owned(),
                        }],
                        otherwise: vec![FlowNode::Cutscene {
                            name: Some("CUT_SCENE_01".to_owned()),
                            row: Some(10),
                            path: Some("ffxiv/clsarc/clsarc00110/clsarc00110".to_owned()),
                            lines: vec!["TEXT_CLSARC000_00021_LUCIANE_000_0001".to_owned()],
                            sheets: vec!["cut_scene/024/VoiceMan_02400".to_owned()],
                        }],
                    },
                ],
            }],
        };
        let dialogue = Dialogue {
            kind: DialogueKind::Quest,
            lines: Vec::new(),
        };
        let dto = SheetDialogueDto::new(
            sheet,
            dialogue,
            None,
            vec!["quest/001/ClsArc998_00131".to_owned()],
            Ok(Some(script)),
            Vec::new(),
            HashMap::new(),
        );
        let value = serde_json::to_value(&dto).expect("json");
        assert_eq!(
            value["scenes"][0]["nodes"],
            json!([
                {
                    "kind": "choice",
                    "id": 4,
                    "choice": "yesNo",
                    "prompts": ["Q1_000_1"],
                    "options": [{
                        "label": { "kind": "text", "key": "A1_000_1" },
                        "available": { "kind": "always" },
                        "then": [{ "kind": "repeat", "loopId": 2 }],
                    }],
                },
                {
                    "kind": "branch",
                    "condition": {
                        "kind": "test",
                        "subject": { "kind": "call", "function": "GetSex", "arguments": [] },
                        "test": { "kind": "truthy", "value": false },
                    },
                    "then": [{ "kind": "line", "key": "TEXT_OTHER_00001_A_000_1" }],
                    "otherwise": [{
                        "kind": "cutscene",
                        "name": "CUT_SCENE_01",
                        "path": "ffxiv/clsarc/clsarc00110/clsarc00110",
                        "lines": ["LUCIANE_000_0001"],
                        "sheets": ["cut_scene/024/VoiceMan_02400"],
                    }],
                },
            ])
        );
        assert_eq!(value["scriptError"], json!(null));
        assert_eq!(value["versions"], json!(["quest/001/ClsArc998_00131"]));
    }

    #[test]
    fn an_unreadable_script_is_reported_without_scenes() {
        let sheet = "quest/000/ClsArc000_00021";
        let failed = SheetDialogueDto::new(
            sheet,
            Dialogue {
                kind: DialogueKind::Quest,
                lines: Vec::new(),
            },
            None,
            Vec::new(),
            Err("unreadable".to_owned()),
            Vec::new(),
            HashMap::new(),
        );
        assert_eq!(
            (failed.scenes, failed.script_error.as_deref()),
            (None, Some("unreadable"))
        );
    }

    #[test]
    fn cutscenes_naming_the_sheet_serialize_with_short_keys() {
        let dto = SheetDialogueDto::new(
            "quest/000/ClsArc000_00021",
            Dialogue {
                kind: DialogueKind::Quest,
                lines: Vec::new(),
            },
            None,
            Vec::new(),
            Ok(None),
            vec![aeria_source::CutsceneLines {
                row: 10,
                path: "ffxiv/clsarc/clsarc00110/clsarc00110".to_owned(),
                keys: vec!["TEXT_CLSARC000_00021_LUCIANE_000_0001".to_owned()],
            }],
            HashMap::from([(
                10,
                vec![aeria_source::CutscenePlay {
                    quest: "quest/000/ClsArc001_00046".to_owned(),
                    name: Some("Close to Home".to_owned()),
                    scene: Some(3),
                    handler: None,
                    script: None,
                }],
            )]),
        );
        assert_eq!(
            serde_json::to_value(&dto).expect("json")["cutscenes"],
            json!([{
                "row": 10,
                "path": "ffxiv/clsarc/clsarc00110/clsarc00110",
                "lines": ["LUCIANE_000_0001"],
                "plays": [{
                    "quest": "quest/000/ClsArc001_00046",
                    "name": "Close to Home",
                    "scene": 3,
                    "handler": null,
                    "script": null,
                }],
            }])
        );
    }
}
