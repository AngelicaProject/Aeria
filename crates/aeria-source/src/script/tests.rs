use super::lua::Op;
use super::lua::assemble::{Builder, abc, abx, asbx};
use super::*;

const SPEAKER: u32 = 2;

/// Scene functions assembled like the game's compiler output, with the
/// quest table in `r0` and an actor in `r2`.
struct Scene(Builder);

impl Scene {
    fn new() -> Self {
        Self(Builder::default())
    }

    fn pc(&self) -> i32 {
        i32::try_from(self.0.code.len()).expect("small")
    }

    fn op(&mut self, word: u32) -> &mut Self {
        self.0.code.push(word);
        self
    }

    /// `r<register> = r0.<name>`
    fn field(&mut self, register: u32, name: &str) -> &mut Self {
        let key = self.0.s(name) + 256;
        self.op(abc(Op::GetTable, register, 0, key))
    }

    /// `r3 = <object>:<method>(<fields>)`, keeping one result.
    fn call(&mut self, object: u32, method: &str, fields: &[&str]) -> &mut Self {
        let key = self.0.s(method) + 256;
        self.op(abc(Op::Method, 3, object, key));
        for (index, name) in fields.iter().enumerate() {
            self.field(5 + u32::try_from(index).expect("few"), name);
        }
        let arguments = u32::try_from(fields.len()).expect("few") + 2;
        self.op(abc(Op::Call, 3, arguments, 2))
    }

    fn talk(&mut self, key: &str) -> &mut Self {
        self.call(SPEAKER, "Talk", &[key])
    }

    /// A jump to `target`, an absolute instruction index.
    fn jump(&mut self, target: i32) -> &mut Self {
        let pc = self.pc();
        self.op(asbx(Op::Jmp, 0, target - pc - 1))
    }

    fn ret(&mut self) -> &mut Self {
        self.op(abc(Op::Return, 0, 1, 0))
    }

    fn number(&mut self, value: f64) -> u32 {
        self.0.k(Constant::Number(value)) + 256
    }
}

/// A chunk whose main function defines `Quest.OnScene<n>` for each scene.
fn chunk(scenes: Vec<(u32, Scene)>) -> Vec<u8> {
    let mut main = Builder::default();
    let table = main.s("Quest");
    for (index, (number, scene)) in scenes.into_iter().enumerate() {
        let name = main.s(&format!("OnScene{number:05}")) + 256;
        main.code.push(abx(Op::GetGlobal, 0, table));
        main.code
            .push(abx(Op::Closure, 1, u32::try_from(index).expect("few")));
        main.code.push(abc(Op::SetTable, 0, name, 1));
        main.functions.push(scene.0);
    }
    main.code.push(abc(Op::Return, 0, 1, 0));
    main.chunk()
}

fn read(scenes: Vec<(u32, Scene)>) -> QuestScript {
    QuestScript::read(&chunk(scenes)).expect("script")
}

fn line(key: &str) -> FlowNode {
    FlowNode::Line {
        key: key.to_owned(),
    }
}

fn text(key: &str) -> OptionLabel {
    OptionLabel::Text(key.to_owned())
}

fn choice(
    id: u32,
    kind: ChoiceKind,
    prompt: Option<&str>,
    options: Vec<(OptionLabel, Vec<FlowNode>)>,
) -> FlowNode {
    FlowNode::Choice(Choice {
        id,
        kind,
        prompts: prompt.map(str::to_owned).into_iter().collect(),
        options: options
            .into_iter()
            .map(|(label, then)| ChoiceOption {
                label,
                available: Availability::Always,
                then,
            })
            .collect(),
    })
}

#[test]
fn a_quest_offer_leads_to_what_accepting_and_declining_say() {
    let mut scene = Scene::new();
    scene
        .talk("TEXT_Q_ATHELYNA_000_1")
        .call(0, "QuestOffer", &[]);
    scene.op(abc(Op::Test, 3, 0, 0)).jump(13);
    scene
        .talk("TEXT_Q_ATHELYNA_000_5")
        .call(0, "QuestAccepted", &[])
        .ret();
    assert_eq!(scene.pc(), 13);
    scene
        .talk("TEXT_Q_ATHELYNA_000_2")
        .call(0, "CancelEventScene", &[])
        .ret();

    let script = read(vec![(0, scene)]);
    assert_eq!(script.scenes.len(), 1);
    assert!(script.scenes[0].traced);
    assert_eq!(
        script.scenes[0].nodes,
        [
            line("TEXT_Q_ATHELYNA_000_1"),
            choice(
                4,
                ChoiceKind::QuestOffer,
                None,
                vec![
                    (
                        OptionLabel::Accept,
                        vec![line("TEXT_Q_ATHELYNA_000_5"), FlowNode::Accepted]
                    ),
                    (
                        OptionLabel::Decline,
                        vec![line("TEXT_Q_ATHELYNA_000_2"), FlowNode::Cancelled]
                    ),
                ],
            ),
        ]
    );
}

#[test]
fn an_answer_that_asks_again_is_a_loop_with_a_repeat() {
    let mut scene = Scene::new();
    scene.op(abc(Op::LoadBool, 4, 0, 0));
    assert_eq!(scene.pc(), 1);
    scene.call(
        0,
        "YesNo",
        &["TEXT_Q_Q1_000_1", "TEXT_Q_A1_000_1", "TEXT_Q_A1_000_2"],
    );
    scene.op(abc(Op::Move, 4, 3, 0));
    let yes = scene.0.k(Constant::Boolean(true)) + 256;
    // Asked again unless the answer is yes.
    scene.op(abc(Op::Eq, 0, 4, yes)).jump(11).jump(15).jump(1);
    assert_eq!(scene.pc(), 11);
    scene.talk("TEXT_Q_SILVAIRRE_000_42").jump(1);
    assert_eq!(scene.pc(), 15);
    scene.talk("TEXT_Q_SILVAIRRE_000_43").ret();

    let nodes = &read(vec![(30, scene)]).scenes[0].nodes;
    assert_eq!(
        *nodes,
        [
            FlowNode::Loop {
                id: 1,
                body: vec![choice(
                    5,
                    ChoiceKind::YesNo,
                    Some("TEXT_Q_Q1_000_1"),
                    vec![
                        (text("TEXT_Q_A1_000_1"), vec![]),
                        (
                            text("TEXT_Q_A1_000_2"),
                            vec![
                                line("TEXT_Q_SILVAIRRE_000_42"),
                                FlowNode::Repeat { loop_id: 1 }
                            ],
                        ),
                    ],
                )],
            },
            line("TEXT_Q_SILVAIRRE_000_43"),
        ]
    );
}

#[test]
fn menu_answers_follow_an_elseif_chain_and_share_what_comes_after() {
    let mut scene = Scene::new();
    scene.call(
        0,
        "Menu",
        &[
            "TEXT_Q_Q2_000_1",
            "TEXT_Q_A2_000_1",
            "TEXT_Q_A2_000_2",
            "TEXT_Q_A2_000_3",
        ],
    );
    let one = scene.number(1.0);
    let two = scene.number(2.0);
    let start = scene.pc();
    assert_eq!(start, 6);
    // if r3 == 1 then … elseif r3 == 2 then … else … end
    scene.op(abc(Op::Eq, 0, 3, one)).jump(12);
    scene.talk("TEXT_Q_KAIN_000_1").jump(20);
    assert_eq!(scene.pc(), 12);
    scene.op(abc(Op::Eq, 0, 3, two)).jump(18);
    scene.talk("TEXT_Q_KAIN_000_2").jump(20);
    assert_eq!(scene.pc(), 18);
    scene
        .op(abc(Op::LoadBool, 6, 0, 0))
        .op(abc(Op::LoadBool, 6, 0, 0));
    assert_eq!(scene.pc(), 20);
    scene.talk("TEXT_Q_KAIN_000_9").ret();

    let nodes = &read(vec![(1, scene)]).scenes[0].nodes;
    assert_eq!(
        *nodes,
        [
            choice(
                5,
                ChoiceKind::Menu,
                Some("TEXT_Q_Q2_000_1"),
                vec![
                    (text("TEXT_Q_A2_000_1"), vec![line("TEXT_Q_KAIN_000_1")]),
                    (text("TEXT_Q_A2_000_2"), vec![line("TEXT_Q_KAIN_000_2")]),
                    (text("TEXT_Q_A2_000_3"), vec![]),
                ],
            ),
            line("TEXT_Q_KAIN_000_9"),
        ]
    );
}

#[test]
fn conditions_on_the_player_are_kept_only_when_they_change_the_text() {
    let mut scene = Scene::new();
    let female = scene.number(1.0);
    // An animation chosen by sex: nothing to read.
    scene
        .call(1, "GetSex", &[])
        .op(abc(Op::Eq, 0, 3, female))
        .jump(7);
    scene.call(
        SPEAKER,
        "PlayActionTimeline",
        &["ACTION_TIMELINE_EVENT_BOW"],
    );
    assert_eq!(scene.pc(), 7);
    // A line only a female player hears.
    scene
        .call(1, "GetSex", &[])
        .op(abc(Op::Eq, 1, 3, female))
        .jump(14);
    assert_eq!(scene.pc(), 11);
    scene
        .op(abc(Op::LoadBool, 6, 0, 0))
        .op(abc(Op::LoadBool, 6, 0, 0))
        .jump(17);
    assert_eq!(scene.pc(), 14);
    scene.talk("TEXT_Q_LUCIANE_000_7");
    assert_eq!(scene.pc(), 17);
    scene.ret();

    let nodes = &read(vec![(2, scene)]).scenes[0].nodes;
    assert_eq!(
        *nodes,
        [FlowNode::Branch {
            condition: Guard::Test(Condition {
                subject: Operand::Call {
                    function: Some("GetSex".to_owned()),
                    arguments: vec![],
                },
                test: Test::Compare(Comparison::Eq, Operand::Constant(Constant::Number(1.0))),
            }),
            then: vec![line("TEXT_Q_LUCIANE_000_7")],
            otherwise: vec![],
        }]
    );
}

#[test]
fn a_constant_on_the_left_is_read_from_the_value_side() {
    let mut scene = Scene::new();
    let three = scene.number(3.0);
    // if 3 <= GetQuestUI8AL() then … end
    scene
        .call(1, "GetQuestUI8AL", &[])
        .op(abc(Op::Le, 1, three, 3))
        .jump(5)
        .jump(8);
    assert_eq!(scene.pc(), 5);
    scene.talk("TEXT_Q_SYSTEM_000_1");
    assert_eq!(scene.pc(), 8);
    scene.ret();

    let nodes = &read(vec![(3, scene)]).scenes[0].nodes;
    let FlowNode::Branch {
        condition: Guard::Test(condition),
        then,
        ..
    } = &nodes[0]
    else {
        panic!("a branch: {nodes:?}");
    };
    assert_eq!(
        condition.test,
        Test::Compare(Comparison::Ge, Operand::Constant(Constant::Number(3.0)))
    );
    assert!(
        matches!(&condition.subject, Operand::Call { function: Some(name), .. } if name == "GetQuestUI8AL")
    );
    assert_eq!(*then, [line("TEXT_Q_SYSTEM_000_1")]);
}

#[test]
fn a_numeric_for_loop_is_traced_as_a_loop() {
    let mut scene = Scene::new();
    let one = scene.number(1.0);
    let three = scene.number(3.0);
    scene
        .op(abx(Op::LoadK, 6, one - 256))
        .op(abx(Op::LoadK, 7, three - 256))
        .op(abx(Op::LoadK, 8, one - 256));
    // FORPREP jumps to FORLOOP, which jumps back to the body.
    scene.op(asbx(Op::ForPrep, 6, 3));
    assert_eq!(scene.pc(), 4);
    scene.talk("TEXT_Q_SYSTEM_000_2");
    assert_eq!(scene.pc(), 7);
    scene.op(asbx(Op::ForLoop, 6, -4));
    scene.talk("TEXT_Q_SYSTEM_000_3").ret();

    let script = read(vec![(4, scene)]);
    assert!(script.scenes[0].traced);
    assert_eq!(
        script.scenes[0].nodes,
        [
            FlowNode::Loop {
                id: 4,
                body: vec![line("TEXT_Q_SYSTEM_000_2")],
            },
            line("TEXT_Q_SYSTEM_000_3"),
        ]
    );
}

#[test]
fn code_that_cannot_be_traced_is_listed_in_order() {
    let mut scene = Scene::new();
    scene.talk("TEXT_Q_SYSTEM_000_1");
    // A test not followed by a jump is not compiler output this reads.
    scene.op(abc(Op::Test, 3, 0, 0));
    scene.talk("TEXT_Q_SYSTEM_000_2").ret();
    let mut silent = Scene::new();
    silent.call(SPEAKER, "Wait", &[]).ret();

    let script = read(vec![(0, scene), (1, silent)]);
    assert_eq!(script.scenes.len(), 1, "a scene without text is left out");
    assert!(!script.scenes[0].traced);
    assert_eq!(
        script.scenes[0].nodes,
        [line("TEXT_Q_SYSTEM_000_1"), line("TEXT_Q_SYSTEM_000_2")]
    );
}

#[test]
fn cutscenes_and_scene_numbers_are_read() {
    let mut late = Scene::new();
    late.call(0, "PlayCutScene", &["CUT_SCENE_01"])
        .call(0, "QuestCompleted", &[])
        .ret();
    let mut early = Scene::new();
    early.talk("TEXT_Q_SYSTEM_000_1").ret();
    let script = read(vec![(12, late), (3, early)]);
    assert_eq!(
        script
            .scenes
            .iter()
            .map(|scene| (scene.scene, scene.nodes.clone()))
            .collect::<Vec<_>>(),
        [
            (Some(3), vec![line("TEXT_Q_SYSTEM_000_1")]),
            (
                Some(12),
                vec![
                    FlowNode::Cutscene {
                        name: Some("CUT_SCENE_01".to_owned()),
                        row: None,
                        path: None,
                        lines: vec![],
                        sheets: vec![],
                    },
                    FlowNode::Completed,
                ]
            ),
        ]
    );
}

#[test]
fn a_game_source_reads_the_script_of_a_quest_sheet() {
    use aeria_sqpack::testing::{FakeGame, TextSheet};

    use crate::{GameSource, SourceError, SourceLanguage};

    let mut scene = Scene::new();
    scene.talk("TEXT_MANFST004_00124_MIOUNNE_000_1").ret();
    let keyed = |key: &str| {
        TextSheet::new(2, &[0, 1])
            .keyed(0)
            .row(0, &[(0, key), (1, "Welcome")])
            .row(1, &[(0, "TEXT_OTHER"), (1, "Other")])
    };
    let folder = tempfile::tempdir().expect("folder");
    FakeGame::new("2026.09.15.0000.0000")
        .with_text(
            "quest/001/ManFst004_00124",
            &keyed("TEXT_MANFST004_00124_MIOUNNE_000_1"),
        )
        .with_text(
            "quest/001/NoScript_00001",
            &keyed("TEXT_NOSCRIPT_00001_A_000_1"),
        )
        .with_text(
            "quest/001/Broken_00002",
            &keyed("TEXT_BROKEN_00002_A_000_1"),
        )
        .with_file(
            "game_script/quest/001/ManFst004_00124.luab",
            chunk(vec![(0, scene)]),
        )
        .with_file(
            "game_script/quest/001/Broken_00002.luab",
            b"print('hi')".to_vec(),
        )
        .write(folder.path())
        .expect("game");
    let source = GameSource::open(folder.path(), SourceLanguage::English).expect("source");

    let script = source
        .quest_script("quest/001/ManFst004_00124")
        .expect("readable")
        .expect("script");
    assert_eq!(
        script.scenes[0].nodes,
        [line("TEXT_MANFST004_00124_MIOUNNE_000_1")]
    );
    assert_eq!(
        source
            .quest_script("quest/001/NoScript_00001")
            .expect("readable"),
        None
    );
    assert_eq!(
        source
            .quest_script("cut_scene/001/ManFst004_00124")
            .expect("readable"),
        None
    );
    assert!(matches!(
        source.quest_script("quest/001/Broken_00002"),
        Err(SourceError::Script { sheet, .. }) if sheet == "quest/001/Broken_00002"
    ));
}

#[test]
fn answers_tested_in_consecutive_ifs_that_ask_again_fold_into_their_options() {
    // while true do
    //   local answer = Menu(Q, A1, A2, A3)
    //   if answer == 1 then Talk(65) else if answer == 2 then Talk(66) else break end
    // end
    // Talk(70)
    let mut scene = Scene::new();
    scene.call(
        0,
        "Menu",
        &[
            "TEXT_Q_Q2_000_1",
            "TEXT_Q_A2_000_1",
            "TEXT_Q_A2_000_2",
            "TEXT_Q_A2_000_3",
        ],
    );
    let one = scene.number(1.0);
    let two = scene.number(2.0);
    assert_eq!(scene.pc(), 6);
    scene.op(abc(Op::Eq, 0, 3, one)).jump(12);
    scene.talk("TEXT_Q_SILVAIRRE_000_65").jump(0);
    assert_eq!(scene.pc(), 12);
    scene.op(abc(Op::Eq, 0, 3, two)).jump(20);
    scene
        .talk("TEXT_Q_SILVAIRRE_000_66")
        .jump(0)
        .jump(20)
        .jump(0);
    assert_eq!(scene.pc(), 20);
    scene.talk("TEXT_Q_SILVAIRRE_000_70").ret();

    let nodes = &read(vec![(13, scene)]).scenes[0].nodes;
    let again = FlowNode::Repeat { loop_id: 0 };
    assert_eq!(
        *nodes,
        [
            FlowNode::Loop {
                id: 0,
                body: vec![choice(
                    5,
                    ChoiceKind::Menu,
                    Some("TEXT_Q_Q2_000_1"),
                    vec![
                        (
                            text("TEXT_Q_A2_000_1"),
                            vec![line("TEXT_Q_SILVAIRRE_000_65"), again.clone()]
                        ),
                        (
                            text("TEXT_Q_A2_000_2"),
                            vec![line("TEXT_Q_SILVAIRRE_000_66"), again]
                        ),
                        (text("TEXT_Q_A2_000_3"), vec![]),
                    ],
                )],
            },
            line("TEXT_Q_SILVAIRRE_000_70"),
        ]
    );
}

#[test]
fn a_value_from_either_of_two_calls_names_both() {
    // if not (IsQuestAccepted(QUEST0) or IsQuestAccepted(QUEST1)) then Talk(A) end
    let mut scene = Scene::new();
    scene.call(1, "IsQuestAccepted", &["QUEST0"]);
    scene.op(abc(Op::Test, 3, 0, 1)).jump(8);
    scene.call(1, "IsQuestAccepted", &["QUEST1"]);
    assert_eq!(scene.pc(), 8);
    let no = scene.0.k(Constant::Boolean(false)) + 256;
    scene.op(abc(Op::Eq, 0, 3, no)).jump(13);
    scene.talk("TEXT_Q_A_000_1");
    assert_eq!(scene.pc(), 13);
    scene.ret();

    let nodes = &read(vec![(2, scene)]).scenes[0].nodes;
    let accepted = |quest: &str| Operand::Call {
        function: Some("IsQuestAccepted".to_owned()),
        arguments: vec![Operand::Field(quest.to_owned())],
    };
    assert_eq!(
        *nodes,
        [FlowNode::Branch {
            condition: Guard::Test(Condition {
                subject: Operand::OneOf(vec![accepted("QUEST0"), accepted("QUEST1")]),
                test: Test::Compare(Comparison::Ne, Operand::Constant(Constant::Boolean(false))),
            }),
            then: vec![],
            otherwise: vec![line("TEXT_Q_A_000_1")],
        }]
    );
}

#[test]
fn a_cutscene_lists_the_lines_its_file_names_in_row_order() {
    use aeria_sqpack::excel::{ColumnKind, Language};
    use aeria_sqpack::testing::{FakeGame, FakeRow, FakeSheet, TextSheet};

    use crate::{GameSource, SourceLanguage};

    let key = |label: &str| format!("TEXT_MANFST004_00124_{label}");
    let quest = TextSheet::new(2, &[0, 1])
        .keyed(0)
        .row(0, &[(0, &key("LUCIANE_000_0001")), (1, "Welcome back.")])
        .row(1, &[(0, &key("LUCIANE_000_0002")), (1, "Tell me.")])
        .row(
            2,
            &[(0, &key("LUCIANE_000_0099")), (1, "Not in the cutscene.")],
        );
    let voice = TextSheet::new(2, &[0, 1])
        .keyed(0)
        .row(
            0,
            &[
                (0, "TEXT_VOICEMAN_02400_000010_URIANGER"),
                (1, "Thou art come."),
            ],
        )
        .row(
            1,
            &[(0, "TEXT_VOICEMAN_02400_000020_URIANGER"), (1, "Well met.")],
        );
    // Script variables are String columns followed by their UInt32 values.
    let mut names = FakeSheet::new(vec![
        ColumnKind::String,
        ColumnKind::String,
        ColumnKind::String,
        ColumnKind::UInt32,
        ColumnKind::String,
        ColumnKind::UInt32,
    ]);
    for language in [
        Language::Japanese,
        Language::English,
        Language::German,
        Language::French,
    ] {
        names = names.with_rows(
            language,
            vec![FakeRow::new(
                7,
                vec![
                    format!("Close to Home {}", language.suffix())
                        .as_str()
                        .into(),
                    "ManFst004_00124".into(),
                    "ACTOR0".into(),
                    1_000_200.into(),
                    "CUT_SCENE_01".into(),
                    10.into(),
                ],
            )],
        );
    }
    let cutscenes = FakeSheet::new(vec![ColumnKind::String]).with_rows(
        Language::None,
        vec![FakeRow::new(
            10,
            vec!["ffxiv/test/cut00010/cut00010".into()],
        )],
    );
    // Out of order and repeated, with text that is not a key and a key of
    // another sheet.
    let cutscene = cutscene_file(&[
        key("LUCIANE_000_0002"),
        "c01".to_owned(),
        key("LUCIANE_000_0001"),
        "TEXT_VOICEMAN_02400_000010_URIANGER".to_owned(),
        key("LUCIANE_000_0002"),
    ]);
    let mut scene = Scene::new();
    scene.call(0, "PlayCutScene", &["CUT_SCENE_01"]).ret();

    let folder = tempfile::tempdir().expect("folder");
    FakeGame::new("2026.09.15.0000.0000")
        .with_text("quest/001/ManFst004_00124", &quest)
        .with_text("cut_scene/024/VoiceMan_02400", &voice)
        .with_sheet("Quest", names)
        .with_sheet("Cutscene", cutscenes)
        .with_file(
            "game_script/quest/001/ManFst004_00124.luab",
            chunk(vec![(0, scene)]),
        )
        .with_file("cut/ffxiv/test/cut00010/cut00010.cutb", cutscene)
        .write(folder.path())
        .expect("game");
    let source = GameSource::open(folder.path(), SourceLanguage::English).expect("source");

    let script = source
        .quest_script("quest/001/ManFst004_00124")
        .expect("readable")
        .expect("script");
    assert_eq!(
        script.scenes[0].nodes,
        [FlowNode::Cutscene {
            name: Some("CUT_SCENE_01".to_owned()),
            row: Some(10),
            path: Some("ffxiv/test/cut00010/cut00010".to_owned()),
            lines: vec![key("LUCIANE_000_0001"), key("LUCIANE_000_0002")],
            sheets: vec!["cut_scene/024/VoiceMan_02400".to_owned()],
        }]
    );
}

#[test]
fn a_condition_joined_with_or_is_one_branch() {
    // if Done(Q0) or Done(Q1) or Done(Q2) == true then Talk(A) else Talk(B) end
    let mut scene = Scene::new();
    let yes = scene.0.k(Constant::Boolean(true)) + 256;
    scene.call(1, "IsQuestCompleted", &["QUEST0"]);
    scene.op(abc(Op::Test, 3, 0, 1)).jump(15);
    scene.call(1, "IsQuestCompleted", &["QUEST1"]);
    scene.op(abc(Op::Test, 3, 0, 1)).jump(15);
    scene.call(1, "IsQuestCompleted", &["QUEST2"]);
    assert_eq!(scene.pc(), 13);
    scene.op(abc(Op::Eq, 0, 3, yes)).jump(19);
    assert_eq!(scene.pc(), 15);
    scene.talk("TEXT_Q_A_000_0").jump(22);
    assert_eq!(scene.pc(), 19);
    scene.talk("TEXT_Q_A_000_1");
    assert_eq!(scene.pc(), 22);
    scene.ret();

    let nodes = &read(vec![(1, scene)]).scenes[0].nodes;
    let done = |quest: &str| Operand::Call {
        function: Some("IsQuestCompleted".to_owned()),
        arguments: vec![Operand::Field(quest.to_owned())],
    };
    assert_eq!(
        *nodes,
        [FlowNode::Branch {
            condition: Guard::Any(vec![
                Guard::Test(Condition {
                    subject: done("QUEST0"),
                    test: Test::Truthy(true),
                }),
                Guard::Test(Condition {
                    subject: done("QUEST1"),
                    test: Test::Truthy(true),
                }),
                Guard::Test(Condition {
                    subject: done("QUEST2"),
                    test: Test::Compare(Comparison::Eq, Operand::Constant(Constant::Boolean(true))),
                }),
            ]),
            then: vec![line("TEXT_Q_A_000_0")],
            otherwise: vec![line("TEXT_Q_A_000_1")],
        }]
    );
}

#[test]
fn a_condition_joined_with_and_is_one_branch() {
    // if Done(Q0) and Done(Q1) then Talk(A) else Talk(B) end
    let mut scene = Scene::new();
    scene.call(1, "IsQuestCompleted", &["QUEST0"]);
    scene.op(abc(Op::Test, 3, 0, 0)).jump(14);
    scene.call(1, "IsQuestCompleted", &["QUEST1"]);
    scene.op(abc(Op::Test, 3, 0, 0)).jump(14);
    assert_eq!(scene.pc(), 10);
    scene.talk("TEXT_Q_A_000_0").jump(17);
    assert_eq!(scene.pc(), 14);
    scene.talk("TEXT_Q_A_000_1");
    assert_eq!(scene.pc(), 17);
    scene.ret();

    let nodes = &read(vec![(1, scene)]).scenes[0].nodes;
    let done = |quest: &str| {
        Guard::Test(Condition {
            subject: Operand::Call {
                function: Some("IsQuestCompleted".to_owned()),
                arguments: vec![Operand::Field(quest.to_owned())],
            },
            test: Test::Truthy(true),
        })
    };
    assert_eq!(
        *nodes,
        [FlowNode::Branch {
            condition: Guard::All(vec![done("QUEST0"), done("QUEST1")]),
            then: vec![line("TEXT_Q_A_000_0")],
            otherwise: vec![line("TEXT_Q_A_000_1")],
        }]
    );
}

#[test]
fn a_quest_variable_names_the_quest_it_holds() {
    use aeria_sqpack::excel::{ColumnKind, Language};
    use aeria_sqpack::testing::{FakeGame, FakeRow, FakeSheet, TextSheet};

    use crate::{GameSource, QuestReference, SourceLanguage};

    let keyed = |key: &str| {
        TextSheet::new(2, &[0, 1])
            .keyed(0)
            .row(0, &[(0, key), (1, "Welcome back.")])
            .row(1, &[(0, "TEXT_OTHER"), (1, "Other")])
    };
    let mut names = FakeSheet::new(vec![
        ColumnKind::String,
        ColumnKind::String,
        ColumnKind::String,
        ColumnKind::UInt32,
    ]);
    for language in [
        Language::Japanese,
        Language::English,
        Language::German,
        Language::French,
    ] {
        let suffix = language.suffix();
        names = names.with_rows(
            language,
            vec![
                FakeRow::new(
                    7,
                    vec![
                        format!("Close to Home {suffix}").as_str().into(),
                        "ManFst004_00124".into(),
                        "QUEST0".into(),
                        8.into(),
                    ],
                ),
                FakeRow::new(
                    8,
                    vec![
                        format!("Earlier {suffix}").as_str().into(),
                        "Prev_00001".into(),
                        "".into(),
                        0.into(),
                    ],
                ),
            ],
        );
    }
    // if IsQuestCompleted(QUEST0) then Talk(A) end
    let mut scene = Scene::new();
    scene.call(1, "IsQuestCompleted", &["QUEST0"]);
    scene.op(abc(Op::Test, 3, 0, 0)).jump(8);
    scene.talk("TEXT_MANFST004_00124_A_000_1");
    assert_eq!(scene.pc(), 8);
    scene.ret();

    let folder = tempfile::tempdir().expect("folder");
    FakeGame::new("2026.09.15.0000.0000")
        .with_text(
            "quest/001/ManFst004_00124",
            &keyed("TEXT_MANFST004_00124_A_000_1"),
        )
        .with_text("quest/000/Prev_00001", &keyed("TEXT_PREV_00001_A_000_1"))
        .with_sheet("Quest", names)
        .with_file(
            "game_script/quest/001/ManFst004_00124.luab",
            chunk(vec![(0, scene)]),
        )
        .write(folder.path())
        .expect("game");
    let source = GameSource::open(folder.path(), SourceLanguage::English).expect("source");

    let script = source
        .quest_script("quest/001/ManFst004_00124")
        .expect("readable")
        .expect("script");
    assert_eq!(
        script.scenes[0].nodes,
        [FlowNode::Branch {
            condition: Guard::Test(Condition {
                subject: Operand::Call {
                    function: Some("IsQuestCompleted".to_owned()),
                    arguments: vec![Operand::Quest(QuestReference {
                        variable: "QUEST0".to_owned(),
                        row: 8,
                        name: Some("Earlier en".to_owned()),
                        sheet: Some("quest/000/Prev_00001".to_owned()),
                    })],
                },
                test: Test::Truthy(true),
            }),
            then: vec![line("TEXT_MANFST004_00124_A_000_1")],
            otherwise: vec![],
        }]
    );
}

#[test]
fn a_boolean_computed_from_a_condition_is_that_condition() {
    // local done = IsQuestCompleted(QUEST0) and true or false
    // if done == false then Talk(A) end
    let mut scene = Scene::new();
    let no = scene.0.k(Constant::Boolean(false)) + 256;
    scene.call(1, "IsQuestCompleted", &["QUEST0"]);
    scene.op(abc(Op::Test, 3, 0, 0)).jump(7);
    scene.op(abc(Op::LoadBool, 7, 1, 0)).jump(8);
    assert_eq!(scene.pc(), 7);
    scene.op(abc(Op::LoadBool, 7, 0, 0));
    scene.op(abc(Op::Eq, 0, 7, no)).jump(13);
    scene.talk("TEXT_Q_A_000_1");
    assert_eq!(scene.pc(), 13);
    scene.ret();

    let nodes = &read(vec![(1, scene)]).scenes[0].nodes;
    assert_eq!(
        *nodes,
        [FlowNode::Branch {
            condition: Guard::Test(Condition {
                subject: Operand::Call {
                    function: Some("IsQuestCompleted".to_owned()),
                    arguments: vec![Operand::Field("QUEST0".to_owned())],
                },
                test: Test::Truthy(true),
            }),
            then: vec![],
            otherwise: vec![line("TEXT_Q_A_000_1")],
        }]
    );
}

/// `<function>() == <value>`
fn returns(function: &str, value: f64) -> Guard {
    Guard::Test(Condition {
        subject: Operand::Call {
            function: Some(function.to_owned()),
            arguments: vec![],
        },
        test: Test::Compare(Comparison::Eq, Operand::Constant(Constant::Number(value))),
    })
}

#[test]
fn a_menu_grayed_out_by_a_computed_flag_is_one_choice_available_when_the_flag_holds() {
    // local relic, enough            -- nil until assigned
    // if GetSex() == 1 and GetRace() == 2 or GetTribe() == 3 then
    //   relic = true; enough = HasItems() and true or false
    // else relic = false end
    // local answer
    // if enough == true then answer = Menu(Q, A1, A2)
    // else answer = GrayoutMenu(Q, A1, MENU_FLAG_DISABLE, A2, MENU_FLAG_ENABLE) end
    // if answer == 1 then
    //   if relic == true then
    //     if enough == true then Talk(OK) else Talk(LOW) end
    //   else Talk(NO) end
    // end
    let mut scene = Scene::new();
    (scene.0.parameters, scene.0.stack) = (3, 16);
    let (one, two, three) = (scene.number(1.0), scene.number(2.0), scene.number(3.0));
    let yes = scene.0.k(Constant::Boolean(true)) + 256;
    scene.call(1, "GetSex", &[]);
    scene.op(abc(Op::Eq, 0, 3, one)).jump(8);
    scene.call(1, "GetRace", &[]);
    scene.op(abc(Op::Eq, 1, 3, two)).jump(12);
    assert_eq!(scene.pc(), 8);
    scene.call(1, "GetTribe", &[]);
    scene.op(abc(Op::Eq, 0, 3, three)).jump(21);
    assert_eq!(scene.pc(), 12);
    scene.op(abc(Op::LoadBool, 10, 1, 0));
    scene.call(1, "HasItems", &[]);
    scene.op(abc(Op::Test, 3, 0, 0)).jump(19);
    scene.op(abc(Op::LoadBool, 11, 1, 0)).jump(22);
    assert_eq!(scene.pc(), 19);
    scene.op(abc(Op::LoadBool, 11, 0, 0)).jump(22);
    assert_eq!(scene.pc(), 21);
    scene.op(abc(Op::LoadBool, 10, 0, 0));
    scene.op(abc(Op::Eq, 0, 11, yes)).jump(30);
    scene
        .call(
            0,
            "Menu",
            &["TEXT_Q_Q1_000_1", "TEXT_Q_A1_000_1", "TEXT_Q_A2_000_1"],
        )
        .jump(37);
    assert_eq!(scene.pc(), 30);
    scene.call(
        0,
        "GrayoutMenu",
        &[
            "TEXT_Q_Q1_000_1",
            "TEXT_Q_A1_000_1",
            "MENU_FLAG_DISABLE",
            "TEXT_Q_A2_000_1",
            "MENU_FLAG_ENABLE",
        ],
    );
    assert_eq!(scene.pc(), 37);
    scene.op(abc(Op::Eq, 0, 3, one)).jump(54);
    scene.op(abc(Op::Eq, 0, 10, yes)).jump(51);
    scene.op(abc(Op::Eq, 0, 11, yes)).jump(47);
    scene.talk("TEXT_Q_OK_000_1").jump(54);
    assert_eq!(scene.pc(), 47);
    scene.talk("TEXT_Q_LOW_000_1").jump(54);
    assert_eq!(scene.pc(), 51);
    scene.talk("TEXT_Q_NO_000_1");
    assert_eq!(scene.pc(), 54);
    scene.ret();

    let has = Guard::Test(Condition {
        subject: Operand::Call {
            function: Some("HasItems".to_owned()),
            arguments: vec![],
        },
        test: Test::Truthy(true),
    });
    // The compiler repeats `GetTribe() == 3` on each path; it reads once.
    let relic = Guard::Any(vec![
        Guard::All(vec![returns("GetSex", 1.0), returns("GetRace", 2.0)]),
        returns("GetTribe", 3.0),
    ]);
    let enough = Guard::All(vec![relic.clone(), has.clone()]);
    let nodes = &read(vec![(1, scene)]).scenes[0].nodes;
    assert_eq!(
        *nodes,
        [FlowNode::Choice(Choice {
            id: 36,
            kind: ChoiceKind::Menu,
            prompts: vec!["TEXT_Q_Q1_000_1".to_owned()],
            options: vec![
                ChoiceOption {
                    label: text("TEXT_Q_A1_000_1"),
                    available: Availability::When(enough),
                    then: vec![FlowNode::Branch {
                        condition: relic,
                        // Inside `relic`, `enough` only adds `HasItems()`.
                        then: vec![FlowNode::Branch {
                            condition: has,
                            then: vec![line("TEXT_Q_OK_000_1")],
                            otherwise: vec![line("TEXT_Q_LOW_000_1")],
                        }],
                        otherwise: vec![line("TEXT_Q_NO_000_1")],
                    }],
                },
                ChoiceOption {
                    label: text("TEXT_Q_A2_000_1"),
                    available: Availability::Always,
                    then: vec![],
                },
            ],
        })]
    );
}

#[test]
fn a_long_elseif_ladder_of_and_conditions_is_traced() {
    // if GetSex() == 0 and GetRace() == 0 then Talk(K0)
    // elseif GetSex() == 1 and GetRace() == 1 then Talk(K1) ... (30 rungs)
    // Each rung's two tests fail to the next rung; traced path by path, the
    // rest of the ladder doubles at every rung.
    const RUNGS: i32 = 30;
    let mut scene = Scene::new();
    scene.call(1, "GetSex", &[]).op(abc(Op::Move, 10, 3, 0));
    scene.call(1, "GetRace", &[]).op(abc(Op::Move, 11, 3, 0));
    let end = scene.pc() + 8 * RUNGS;
    for rung in 0..RUNGS {
        let start = scene.pc();
        let value = scene.number(f64::from(rung));
        scene.op(abc(Op::Eq, 0, 10, value)).jump(start + 8);
        scene.op(abc(Op::Eq, 0, 11, value)).jump(start + 8);
        scene.talk(&format!("TEXT_Q_K_000_{rung}")).jump(end);
    }
    assert_eq!(scene.pc(), end);
    scene.ret();

    let flow = &read(vec![(1, scene)]).scenes[0];
    assert!(flow.traced);
    let mut rungs = 0;
    let mut nodes = flow.nodes.as_slice();
    while let [
        FlowNode::Branch {
            condition: Guard::All(tests),
            then,
            otherwise,
        },
    ] = nodes
    {
        assert_eq!(tests.len(), 2);
        assert_eq!(*then, [line(&format!("TEXT_Q_K_000_{rungs}"))]);
        rungs += 1;
        nodes = otherwise;
    }
    assert_eq!(rungs, RUNGS);
    assert!(nodes.is_empty());
}

#[test]
fn a_value_chosen_with_or_is_any_of_its_values() {
    // if (IsQuestCompleted(Q0) or IsQuestCompleted(Q1) or IsQuestCompleted(Q2)) == true
    // then Talk(A) end
    let mut scene = Scene::new();
    let yes = scene.0.k(Constant::Boolean(true)) + 256;
    scene.call(1, "IsQuestCompleted", &["QUEST0"]);
    scene.op(abc(Op::Test, 3, 0, 1)).jump(13);
    scene.call(1, "IsQuestCompleted", &["QUEST1"]);
    scene.op(abc(Op::Test, 3, 0, 1)).jump(13);
    scene.call(1, "IsQuestCompleted", &["QUEST2"]);
    assert_eq!(scene.pc(), 13);
    scene.op(abc(Op::Eq, 1, 3, yes)).jump(18);
    scene.talk("TEXT_Q_A_000_1");
    assert_eq!(scene.pc(), 18);
    scene.ret();

    let nodes = &read(vec![(1, scene)]).scenes[0].nodes;
    let completed = |quest: &str| Operand::Call {
        function: Some("IsQuestCompleted".to_owned()),
        arguments: vec![Operand::Field(quest.to_owned())],
    };
    let [
        FlowNode::Branch {
            condition: Guard::Test(condition),
            ..
        },
    ] = nodes.as_slice()
    else {
        panic!("one branch: {nodes:?}");
    };
    // Each call's value reaches the comparison when it is true, the last
    // one also when it is not.
    let Operand::OneOf(values) = &condition.subject else {
        panic!("either value: {condition:?}");
    };
    for quest in ["QUEST0", "QUEST1", "QUEST2"] {
        assert!(values.contains(&completed(quest)), "{quest} in {values:?}");
    }
}

#[test]
fn a_for_loop_variable_is_not_the_text_its_register_held_before() {
    // local key = TEXT_Q_A_000_1; Talk(key)
    // for i = 1, 1 do SetNpcTradeItem(i) end   -- `i` reuses the register
    let mut scene = Scene::new();
    let one = scene.number(1.0);
    scene.field(9, "TEXT_Q_A_000_1");
    let talk = scene.0.s("Talk") + 256;
    scene.op(abc(Op::Method, 10, SPEAKER, talk));
    scene.op(abc(Op::Move, 12, 9, 0));
    scene.op(abc(Op::Call, 10, 3, 1));
    for register in 6..=8 {
        scene.op(abx(Op::LoadK, register, one - 256));
    }
    scene.op(asbx(Op::ForPrep, 6, 3));
    let trade = scene.0.s("SetNpcTradeItem") + 256;
    scene.op(abc(Op::Method, 10, 0, trade));
    scene.op(abc(Op::Move, 12, 9, 0));
    scene.op(abc(Op::Call, 10, 3, 1));
    assert_eq!(scene.pc(), 11);
    scene.op(asbx(Op::ForLoop, 6, -4));
    scene.ret();

    let nodes = &read(vec![(1, scene)]).scenes[0].nodes;
    assert_eq!(*nodes, [line("TEXT_Q_A_000_1")]);
}

#[test]
fn the_same_choice_reached_on_two_paths_is_one_choice() {
    // if GetSex() == 1 or GetRace() == 2 then YesNo(Q, A, B) end
    let mut scene = Scene::new();
    let one = scene.number(1.0);
    let two = scene.number(2.0);
    scene.call(1, "GetSex", &[]);
    scene.op(abc(Op::Eq, 1, 3, one)).jump(8);
    scene.call(1, "GetRace", &[]);
    scene.op(abc(Op::Eq, 0, 3, two)).jump(13);
    assert_eq!(scene.pc(), 8);
    scene.call(
        0,
        "YesNo",
        &["TEXT_Q_Q1_000_1", "TEXT_Q_A1_000_1", "TEXT_Q_A1_000_2"],
    );
    assert_eq!(scene.pc(), 13);
    scene.ret();

    let nodes = &read(vec![(1, scene)]).scenes[0].nodes;
    let FlowNode::Branch {
        condition: Guard::Any(guards),
        then,
        otherwise,
    } = &nodes[0]
    else {
        panic!("one branch on either test: {nodes:?}");
    };
    assert_eq!(guards.len(), 2);
    assert!(matches!(then.as_slice(), [FlowNode::Choice(choice)] if choice.id == 12));
    assert!(otherwise.is_empty());
}

#[test]
fn a_key_loaded_into_a_register_is_still_a_line() {
    // In a function with more than 256 constants the compiler loads the key
    // first: `LOADK r5, "TEXT_…"; GETTABLE r5, r0, r5`.
    let mut scene = Scene::new();
    let talk = scene.0.s("Talk") + 256;
    let key = scene.0.s("TEXT_Q_GEROLT_000_216");
    scene
        .op(abc(Op::Method, 3, SPEAKER, talk))
        .op(abx(Op::LoadK, 5, key))
        .op(abc(Op::GetTable, 5, 0, 5))
        .op(abc(Op::Call, 3, 3, 1))
        .ret();

    let nodes = &read(vec![(6, scene)]).scenes[0].nodes;
    assert_eq!(*nodes, [line("TEXT_Q_GEROLT_000_216")]);
}

/// The `Quest` sheet of `ClsRog250_00148`, naming its battle `QUESTBATTLE0 = 5`.
fn battle_quest_names() -> aeria_sqpack::testing::FakeSheet {
    use aeria_sqpack::excel::{ColumnKind, Language};
    use aeria_sqpack::testing::{FakeRow, FakeSheet};

    let mut names = FakeSheet::new(vec![
        ColumnKind::String,
        ColumnKind::String,
        ColumnKind::String,
        ColumnKind::UInt32,
    ]);
    for language in [
        Language::Japanese,
        Language::English,
        Language::German,
        Language::French,
    ] {
        names = names.with_rows(
            language,
            vec![FakeRow::new(
                7,
                vec![
                    format!("Rogue {}", language.suffix()).as_str().into(),
                    "ClsRog250_00148".into(),
                    "QUESTBATTLE0".into(),
                    5.into(),
                ],
            )],
        );
    }
    names
}

/// A cutscene file naming `keys`.
fn cutscene_file(keys: &[String]) -> Vec<u8> {
    let body = keys.iter().fold(String::new(), |mut body, key| {
        body.push('\u{1}');
        body.push_str(key);
        body.push('\0');
        body
    });
    let mut data = b"CUTB".to_vec();
    data.extend_from_slice(&u32::try_from(body.len() + 8).expect("small").to_le_bytes());
    data.extend_from_slice(body.as_bytes());
    data
}

/// A key of the battle quest's sheet.
fn rogue(label: &str) -> String {
    format!("TEXT_CLSROG250_00148_{label}")
}

/// A game whose quest `ClsRog250_00148` has a battle script that plays
/// cutscene row 10, and whose cutscene row 20 names its lines but is not
/// played by it.
fn battle_game(folder: &std::path::Path) -> crate::GameSource {
    use aeria_sqpack::excel::{ColumnKind, Language};
    use aeria_sqpack::testing::{FakeGame, FakeRow, FakeSheet, TextSheet};

    use crate::{GameSource, SourceLanguage};

    let quest = TextSheet::new(2, &[0, 1])
        .keyed(0)
        .row(0, &[(0, &rogue("JACKE_000_1")), (1, "Opening.")])
        .row(1, &[(0, &rogue("JACKE_000_2")), (1, "In battle.")])
        .row(2, &[(0, &rogue("JACKE_000_3")), (1, "In a cutscene.")])
        .row(3, &[(0, &rogue("JACKE_000_4")), (1, "Elsewhere.")]);
    let battles = FakeSheet::new(vec![ColumnKind::String, ColumnKind::UInt32]).with_rows(
        Language::None,
        vec![FakeRow::new(5, vec!["CUT_SCENE_01".into(), 10.into()])],
    );
    let cutscenes = FakeSheet::new(vec![ColumnKind::String]).with_rows(
        Language::None,
        vec![
            FakeRow::new(10, vec!["ffxiv/test/cut10/cut10".into()]),
            FakeRow::new(20, vec!["ffxiv/test/cut20/cut20".into()]),
        ],
    );
    let mut scene = Scene::new();
    scene.talk(&rogue("JACKE_000_1")).ret();
    let mut battle = Scene::new();
    battle
        .talk(&rogue("JACKE_000_2"))
        .call(0, "PlayCutScene", &["CUT_SCENE_01"])
        .ret();
    FakeGame::new("2026.09.15.0000.0000")
        .with_text("quest/001/ClsRog250_00148", &quest)
        .with_sheet("Quest", battle_quest_names())
        .with_sheet("QuestBattle", battles)
        .with_sheet("Cutscene", cutscenes)
        .with_file(
            "game_script/quest/001/ClsRog250_00148.luab",
            chunk(vec![(0, scene)]),
        )
        .with_file(
            "game_script/quest/001/ClsRog250Btl_00148.luab",
            chunk(vec![(1, battle)]),
        )
        .with_file(
            "cut/ffxiv/test/cut10/cut10.cutb",
            cutscene_file(&[rogue("JACKE_000_3")]),
        )
        .with_file(
            "cut/ffxiv/test/cut20/cut20.cutb",
            cutscene_file(&[rogue("JACKE_000_4"), rogue("JACKE_000_3")]),
        )
        .write(folder)
        .expect("game");
    GameSource::open(folder, SourceLanguage::English).expect("source")
}

#[test]
fn a_quests_battle_script_and_every_cutscene_naming_its_lines_are_found() {
    use crate::CutsceneLines;

    let folder = tempfile::tempdir().expect("folder");
    let source = battle_game(folder.path());
    let script = source
        .quest_script("quest/001/ClsRog250_00148")
        .expect("readable")
        .expect("script");
    let scenes: Vec<(Option<u32>, Option<&str>, &[FlowNode])> = script
        .scenes
        .iter()
        .map(|scene| (scene.scene, scene.script.as_deref(), scene.nodes.as_slice()))
        .collect();
    assert_eq!(
        scenes,
        [
            (Some(0), None, [line(&rogue("JACKE_000_1"))].as_slice()),
            (
                Some(1),
                Some("ClsRog250Btl"),
                [
                    line(&rogue("JACKE_000_2")),
                    FlowNode::Cutscene {
                        name: Some("CUT_SCENE_01".to_owned()),
                        row: Some(10),
                        path: Some("ffxiv/test/cut10/cut10".to_owned()),
                        lines: vec![rogue("JACKE_000_3")],
                        sheets: vec![],
                    },
                ]
                .as_slice(),
            ),
        ]
    );
    assert_eq!(
        source
            .cutscenes_naming("quest/001/ClsRog250_00148", 2)
            .expect("readable"),
        [
            CutsceneLines {
                row: 10,
                path: "ffxiv/test/cut10/cut10".to_owned(),
                keys: vec![rogue("JACKE_000_3")],
            },
            CutsceneLines {
                row: 20,
                path: "ffxiv/test/cut20/cut20".to_owned(),
                keys: vec![rogue("JACKE_000_3"), rogue("JACKE_000_4")],
            },
        ]
    );
}

#[test]
fn a_cutscene_is_found_where_the_quests_scripts_play_it() {
    let folder = tempfile::tempdir().expect("folder");
    let source = battle_game(folder.path());
    // The battle script plays row 10; no script plays row 20.
    let plays = source.cutscene_plays(&[10, 20]).expect("readable");
    let found: Vec<(&str, Option<u32>, Option<&str>)> = plays[&10]
        .iter()
        .map(|play| (play.quest.as_str(), play.scene, play.script.as_deref()))
        .collect();
    assert_eq!(
        found,
        [("quest/001/ClsRog250_00148", Some(1), Some("ClsRog250Btl"))]
    );
    assert!(!plays.contains_key(&20));
}
