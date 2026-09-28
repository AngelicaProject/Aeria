import assert from "node:assert/strict";
import test from "node:test";

import { answerOption, buildSceneRows, cutsceneRow, mayHaveScene, speakerName } from "../src/dialogueScene.ts";
import { flattenTranslationRows } from "../src/translationOccurrences.ts";

const SHEET = "quest/000/ClsArc000_00021";
const binding = (rowId) => ({ sheetName: SHEET, rowId, subrowId: 0, columnIndex: 1 });
const line = (rowId, role, speaker, key) => ({ sourceBinding: binding(rowId), role, speaker, key, sourceMacro: `line ${rowId}` });
const quest = { sourceBinding: { sheetName: "Quest", rowId: 7, subrowId: 0, columnIndex: 0 }, sourceMacro: "Close to Home", targetMacro: null };

const lines = [
  line(0, "journal", null, "SEQ_00"),
  line(1, "objective", null, "TODO_00"),
  line(2, "speech", "ATHELYNA", "ATHELYNA_000_1"),
  line(3, "speech", "ATHELYNA", "ATHELYNA_000_2"),
  line(4, "speech", "ATHELYNA", "ATHELYNA_000_5"),
  line(5, "speech", "LUCIANE", "LUCIANE_000_10"),
  line(6, "speech", "Q1", "Q1_000_1"),
  line(7, "speech", "A1", "A1_000_1"),
  line(8, "speech", "A1", "A1_000_2"),
  line(9, "speech", "LUCIANE", "LUCIANE_000_25"),
  line(10, "speech", "SYSTEM_NONE_VOICE", "000100_SYSTEM_NONE_VOICE"),
  line(11, "other", null, "POP_MESSAGE"),
  line(12, "speech", "LUCIANE", "LUCIANE_000_99"),
];

/** A compact picture of the rows: kind, depth, and what they show. */
function shape(model) {
  return model.rows.map((row) => {
    const pad = "  ".repeat(row.depth);
    switch (row.kind) {
      case "line": return `${pad}${row.style === "plain" ? "" : `${row.style} `}${row.line.binding.rowId}`;
      case "speaker": return `${pad}@${row.speaker}`;
      case "heading": return `${pad}#${row.heading}`;
      case "option": return `${pad}> ${row.line ? row.line.binding.rowId : row.label.kind}${row.collapsed ? " (folded)" : ""}`;
      case "branch": return `${pad}if${row.negated ? " not" : ""}`;
      case "marker": return `${pad}[${row.marker}]`;
      case "section": return `== ${row.title} ${row.scene}`;
      case "versions": return `versions ${row.sheets.join(" ")}`;
      default: return `${pad}${row.kind}`;
    }
  });
}

test("without a script, lines are grouped by role and consecutive speaker in row order", () => {
  const model = buildSceneRows({ kind: "quest", quest, versions: [], lines, scenes: null, scriptError: null }, []);
  assert.equal(model.scripted, false);
  assert.deepEqual(shape(model), [
    "quest",
    "#journal", "0",
    "#objectives", "1",
    "@ATHELYNA", "2", "3", "4",
    "@LUCIANE", "5",
    "#choice", "prompt 6", "answer 7", "answer 8",
    "@LUCIANE", "9",
    "#system", "10",
    "#other", "other 11",
    "@LUCIANE", "12",
  ]);
});

const scenes = [
  {
    scene: 0,
    handler: null,
    script: null,
    traced: true,
    nodes: [
      { kind: "line", key: "ATHELYNA_000_1" },
      {
        kind: "choice",
        id: 4,
        choice: "questOffer",
        prompts: [],
        options: [
          { label: { kind: "accept" }, then: [{ kind: "line", key: "ATHELYNA_000_5" }, { kind: "accepted" }] },
          { label: { kind: "decline" }, then: [{ kind: "line", key: "ATHELYNA_000_2" }, { kind: "cancelled" }] },
        ],
      },
    ],
  },
  {
    scene: 1,
    handler: null,
    script: null,
    traced: true,
    nodes: [
      { kind: "line", key: "LUCIANE_000_10" },
      {
        kind: "loop",
        id: 3,
        body: [
          {
            kind: "choice",
            id: 50,
            choice: "yesNo",
            prompts: ["Q1_000_1"],
            options: [
              { label: { kind: "text", key: "A1_000_1" }, then: [] },
              { label: { kind: "text", key: "a1_000_2" }, then: [{ kind: "line", key: "LUCIANE_000_25" }, { kind: "repeat", loopId: 3 }] },
            ],
          },
        ],
      },
      {
        kind: "branch",
        condition: { kind: "test", subject: { kind: "call", function: "GetSex", arguments: [] }, test: { kind: "compare", comparison: "eq", value: { kind: "number", value: 1 } } },
        then: [],
        otherwise: [{ kind: "line", key: "000100_SYSTEM_NONE_VOICE" }, { kind: "line", key: "TEXT_OTHER_00001_A_000_1" }],
      },
      { kind: "cutscene", name: "CUT_SCENE_01", lines: ["LUCIANE_000_99"], sheets: ["cut_scene/024/VoiceMan_02400"] },
    ],
  },
];

test("a quest script reads scene by scene with choices, loops, conditions, and unplayed lines last", () => {
  const dialogue = { kind: "quest", quest, versions: ["quest/001/ClsArc998_00131"], lines, scenes, scriptError: null };
  const model = buildSceneRows(dialogue, []);
  assert.equal(model.scripted, true);
  assert.deepEqual(shape(model), [
    "quest",
    "versions quest/001/ClsArc998_00131",
    "#journal", "0",
    "#objectives", "1",
    "== accepting 0",
    "@ATHELYNA", "2",
    "#questOffer",
    "> accept", "  @ATHELYNA", "  4", "  [accepted]",
    "> decline", "  @ATHELYNA", "  3", "  [cancelled]",
    "== scene 1",
    "@LUCIANE", "5",
    "loop",
    "  #choice", "  prompt 6",
    "  > 7",
    "  > 8", "    @LUCIANE", "    9", "    [repeat]",
    "if not", "  @SYSTEM_NONE_VOICE", "  10", "  missing",
    "cutscene", "  @LUCIANE", "  12", "  sheets",
    "#unscripted", "  #other", "  other 11",
  ]);
  const second = model.rows.find((row) => row.kind === "line" && row.line.binding.rowId === 8);
  assert.equal(second, undefined, "an answer is its option's row, not a separate line");
  const optionEight = model.rows.findIndex((row) => row.kind === "option" && row.line?.binding.rowId === 8);
  assert.equal(model.firstRow.get(model.rows[optionEight].line.key), optionEight);
});

test("folded options hide what they lead to", () => {
  const dialogue = { kind: "quest", quest: null, versions: [], lines, scenes, scriptError: null };
  const rows = buildSceneRows(dialogue, [], new Set(["scene0.1.o0"])).rows;
  const accept = rows.findIndex((row) => row.kind === "option" && row.label.kind === "accept");
  assert.equal(rows[accept].collapsed, true);
  assert.equal(rows[accept + 1].kind, "option", "the decline option follows at once");
});

test("scene lines join the loaded strings", () => {
  const occurrences = flattenTranslationRows([
    {
      sheetName: SHEET,
      rowId: 5,
      subrowId: 0,
      context: [],
      cells: [{ sourceBinding: binding(5), sourceMacro: "line 5", formattingOnly: false, translation: { translationUnitId: "tu", targetMacro: "строка 5", reviewState: "draft", translatorNote: null } }],
    },
  ]);
  const rows = buildSceneRows({ kind: "quest", quest: null, versions: [], lines, scenes, scriptError: null }, occurrences).rows;
  const luciane = rows.find((row) => row.kind === "line" && row.line.binding.rowId === 5);
  assert.equal(luciane.line.occurrence.targetMacro, "строка 5");
});

test("an unreadable script is reported above the grouped lines", () => {
  const rows = buildSceneRows({ kind: "quest", quest: null, versions: [], lines, scenes: null, scriptError: "broken" }, []).rows;
  assert.deepEqual(rows[0], { kind: "notice", id: "script-error", depth: 0, notice: "scriptError", message: "broken" });
});

test("a condition on an answer names the option it selects", () => {
  const yesNo = { choice: "yesNo", options: [{ kind: "yes" }, { kind: "no" }] };
  const menu = { choice: "menu", options: [{ kind: "text", key: "A" }, { kind: "text", key: "B" }, { kind: "text", key: "C" }] };
  const compare = (comparison, value) => ({ subject: { kind: "answer", choice: 1 }, test: { kind: "compare", comparison, value } });
  assert.equal(answerOption(yesNo, compare("eq", { kind: "boolean", value: true })), 0);
  assert.equal(answerOption(yesNo, compare("ne", { kind: "boolean", value: true })), 1);
  assert.equal(answerOption(yesNo, { subject: { kind: "answer", choice: 1 }, test: { kind: "truthy", value: false } }), 1);
  assert.equal(answerOption(menu, compare("eq", { kind: "number", value: 3 })), 2);
  assert.equal(answerOption(menu, compare("ne", { kind: "number", value: 3 })), null, "≠ in a menu of three names two options");
  assert.equal(answerOption(menu, compare("lt", { kind: "number", value: 3 })), null);
  assert.equal(answerOption(menu, compare("eq", { kind: "number", value: 4 })), null);
});

test("speaker labels read as names", () => {
  assert.equal(speakerName("KETENRAMM"), "Ketenramm");
  assert.equal(speakerName("AMHGARANJY_GEVA"), "Amhgaranjy Geva");
  assert.equal(mayHaveScene(SHEET), true);
  assert.equal(mayHaveScene("cut_scene/024/VoiceMan_02400"), true);
  assert.equal(mayHaveScene("Quest"), false);
  assert.equal(mayHaveScene(null), false);
});

test("nested rows carry a rail for each container they are in", () => {
  const dialogue = { kind: "quest", quest, versions: [], lines, scenes, scriptError: null };
  const model = buildSceneRows(dialogue, []);
  const picture = model.rows.map((row, index) => `${shape({ rows: [row] })[0].trim()} [${model.layout[index].rails.join(",")}]`);
  assert.equal(model.layout.length, model.rows.length);
  assert.ok(picture.includes("> accept []"));
  assert.ok(picture.includes("4 [choice]"), "a line an answer leads to sits on a choice rail");
  assert.ok(picture.includes("9 [loop,choice]"), "inside a loop and an answer");
  assert.ok(picture.includes("[repeat] [loop,choice]"));
  assert.ok(picture.includes("10 [branch]"));
  assert.ok(picture.includes("12 [cutscene]"));
  assert.ok(picture.includes("other 11 [plain]"), "lines no scene plays sit on a plain rail");
  model.rows.forEach((row, index) => assert.equal(model.layout[index].rails.length, row.depth, row.id));
});

test("handlers, cutscene files, and battle talk each get a section", () => {
  const extra = [
    ...lines,
    line(13, "speech", "GEROLT", "GEROLT_000_216"),
    line(14, "speech", "ALPHINAUD_BATTLETALK", "ALPHINAUD_BATTLETALK_000_1"),
    line(15, "speech", "LUCIANE", "LUCIANE_000_0001"),
  ];
  const handlerScenes = [
    ...scenes,
    { scene: null, handler: "GetBalloonTalkArgs", script: null, traced: true, nodes: [{ kind: "line", key: "GEROLT_000_216" }] },
  ];
  const dialogue = {
    kind: "quest", quest: null, versions: [], lines: extra, scenes: handlerScenes, scriptError: null,
    cutscenes: [{ row: 10, path: "ffxiv/clsarc/clsarc00110/clsarc00110", lines: ["LUCIANE_000_0001", "LUCIANE_000_10"] }],
  };
  const picture = shape(buildSceneRows(dialogue, []));
  const from = picture.indexOf("== handler null");
  assert.deepEqual(picture.slice(from), [
    "== handler null", "@GEROLT", "13",
    "== cutscene null", "plays", "@LUCIANE", "15",
    "#battleTalk", "  @ALPHINAUD_BATTLETALK", "  14",
    "#unscripted", "  #other", "  other 11",
  ]);
  const cutscene = buildSceneRows(dialogue, []).rows.find((row) => row.kind === "section" && row.title === "cutscene");
  assert.equal(cutscene.name, "ffxiv/clsarc/clsarc00110/clsarc00110", "a line a scene plays is not repeated under its cutscene");
});

test("a cutscene sheet reads cutscene by cutscene", () => {
  const cutLines = [
    line(0, "speech", "URIANGER", "000010_URIANGER"),
    line(1, "speech", "ILBERD", "000020_ILBERD"),
    line(2, "speech", "ILBERD", "000030_ILBERD"),
  ];
  const play = { quest: "quest/047/AktKmm103_04753", name: "The Coming Dawn", scene: 17, handler: null, script: null };
  const dialogue = { kind: "cutscene", quest: null, versions: [], lines: cutLines, scenes: null, scriptError: null, cutscenes: [{ row: 5, path: "ffxiv/voiceman/a", lines: ["000010_URIANGER", "000020_ILBERD"], plays: [play] }] };
  const model = buildSceneRows(dialogue, []);
  assert.equal(model.scripted, false);
  assert.deepEqual(shape(model), ["== cutscene null", "plays", "@URIANGER", "0", "@ILBERD", "1", "#notInCutscenes", "  @ILBERD", "  2"]);
  assert.deepEqual(model.rows[1].plays, [play], "the section says which quest's scene plays the cutscene");
});

test("a jump to a cutscene file lands on its section or on the cutscene a scene plays", () => {
  const cutLines = [line(0, "speech", "URIANGER", "000010_URIANGER")];
  const sheet = { kind: "cutscene", quest: null, versions: [], lines: cutLines, scenes: null, scriptError: null, cutscenes: [{ row: 5, path: "ffxiv/voiceman/a", lines: ["000010_URIANGER"], plays: [] }] };
  const rows = buildSceneRows(sheet, []).rows;
  assert.equal(cutsceneRow(rows, "ffxiv/voiceman/a"), 0);
  assert.equal(cutsceneRow(rows, "ffxiv/voiceman/b"), -1);
  const played = { kind: "cutscene", name: "CUT_SCENE_01", path: "ffxiv/voiceman/a", lines: [], sheets: ["cut_scene/024/VoiceMan_02400"] };
  const quest = { kind: "quest", quest: null, versions: [], lines, scenes: [{ scene: 3, handler: null, script: null, traced: true, nodes: [played] }], scriptError: null, cutscenes: [] };
  const questRows = buildSceneRows(quest, []).rows;
  const index = cutsceneRow(questRows, "ffxiv/voiceman/a");
  assert.equal(questRows[index].kind, "cutscene");
  const sheets = questRows.find((row) => row.kind === "sheets");
  assert.equal(sheets.path, "ffxiv/voiceman/a", "the link to the other sheet carries the cutscene");
});

test("an answer keeps when it can be picked", () => {
  const holds = { kind: "test", subject: { kind: "field", name: "READY" }, test: { kind: "truthy", value: true } };
  const menu = {
    kind: "choice", id: 1, choice: "menu", prompts: ["Q1_000_1"],
    options: [
      { label: { kind: "text", key: "A1_000_1" }, available: { kind: "when", guard: holds }, then: [] },
      { label: { kind: "text", key: "A1_000_2" }, available: { kind: "never" }, then: [] },
    ],
  };
  const dialogue = { kind: "quest", quest: null, versions: [], lines, scenes: [{ scene: 1, handler: null, script: null, traced: true, nodes: [menu] }], scriptError: null, cutscenes: [] };
  const options = buildSceneRows(dialogue, []).rows.filter((row) => row.kind === "option");
  assert.deepEqual(options.map((row) => row.available), [{ kind: "when", guard: holds }, { kind: "never" }]);
});

test("a choice in a cutscene file reads as a choice in row order", () => {
  const cutLines = [
    line(0, "speech", "EMETSELCH", "002050_EMETSELCH"),
    line(1, "speech", "Q4", "Q4_000_001_NONE_VOICE"),
    line(2, "speech", "A4", "A4_000_001_NONE_VOICE"),
    line(3, "speech", "A4", "A4_000_002_NONE_VOICE"),
    line(4, "speech", "EMETSELCH", "002060_EMETSELCH"),
  ];
  const dialogue = { kind: "cutscene", quest: null, versions: [], lines: cutLines, scenes: null, scriptError: null, cutscenes: [{ row: 5, path: "ffxiv/a", lines: cutLines.map((entry) => entry.key), plays: [] }] };
  const model = buildSceneRows(dialogue, []);
  assert.deepEqual(shape(model), ["== cutscene null", "plays", "@EMETSELCH", "0", "#choice", "prompt 1", "answer 2", "answer 3", "@EMETSELCH", "4"]);
  assert.equal(model.rows.find((row) => row.kind === "heading" && row.heading === "choice").rowOrder, true, "its answers may change what follows");
});
