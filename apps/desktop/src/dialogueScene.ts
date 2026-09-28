import { bindingKey } from "./binding.ts";
import type { TranslationOccurrenceView } from "./translationOccurrences.ts";
import type {
  ChoiceKindDto,
  ConditionDto,
  GuardDto,
  DialogueLineDto,
  FlowNodeDto,
  AvailabilityDto,
  CutscenePlayDto,
  OptionLabelDto,
  QuestNameDto,
  SheetDialogueDto,
  SourceBinding,
} from "./types";

/**
 * Renderer-only projection of a quest or cutscene sheet as a scene: one list
 * of rows, indented by depth, for a virtualized view.
 *
 * With the scenes traced from a quest's script (`sheet_dialogue`), each
 * scene shows its lines in the order the script plays them, with the
 * player's choices, the conditions that change the text, and loops. Lines
 * no scene plays follow at the end. Without a script, as for cutscenes,
 * lines are grouped in row order. Nothing here is recorded.
 */
export type SceneLine = {
  key: string;
  binding: SourceBinding;
  /** What the row key says after the `TEXT_<ID>_` prefix. */
  rowKey: string;
  sourceMacro: string;
  /** The editor's string for the line; `null` when the cell is not translatable. */
  occurrence: TranslationOccurrenceView | null;
};

export type SceneHeading = "journal" | "objectives" | "system" | "other" | "choice" | "questOffer" | "unscripted" | "battleTalk" | "notInCutscenes";

/** What a section is: a scene (titled by what it does), another function of a script, or a cutscene file. */
export type SectionTitle = "accepting" | "completing" | "scene" | "handler" | "cutscene";

/** What a condition on a choice's answer refers to. */
export type AnswerContext = { choice: ChoiceKindDto; options: OptionLabelDto[] };

export type SceneMarker = "repeat" | "cancelled" | "accepted" | "completed";

export type SceneRow =
  | { kind: "quest"; id: string; depth: number; quest: QuestNameDto }
  | { kind: "versions"; id: string; depth: number; sheets: string[] }
  /** `name` is a handler's name or a cutscene's path; `battle` the battle script a scene or handler belongs to. */
  | { kind: "section"; id: string; depth: number; title: SectionTitle; scene: number | null; name: string | null; battle: string | null }
  | { kind: "notice"; id: string; depth: number; notice: "scriptError" | "untraced"; message: string | null }
  /** `rowOrder` marks a choice read from row order, whose answers may change what follows in ways the rows do not show. */
  | { kind: "heading"; id: string; depth: number; heading: SceneHeading; rowOrder?: boolean }
  | { kind: "speaker"; id: string; depth: number; speaker: string }
  | { kind: "line"; id: string; depth: number; line: SceneLine; style: "plain" | "prompt" | "answer" | "other" }
  | { kind: "missing"; id: string; depth: number; key: string }
  | { kind: "option"; id: string; depth: number; label: OptionLabelDto; available: AvailabilityDto; line: SceneLine | null; empty: boolean; collapsed: boolean }
  | { kind: "branch"; id: string; depth: number; condition: GuardDto; negated: boolean; answer: AnswerContext | null; collapsed: boolean }
  | { kind: "otherwise"; id: string; depth: number; collapsed: boolean }
  | { kind: "loop"; id: string; depth: number; collapsed: boolean }
  /** `path` is the cutscene's file, which the sheets it also names lines of open at. */
  | { kind: "cutscene"; id: string; depth: number; name: string | null; path: string | null; empty: boolean; collapsed: boolean }
  | { kind: "sheets"; id: string; depth: number; sheets: string[]; path: string | null }
  /** The quests' scenes that play the cutscene file of the section above. */
  | { kind: "plays"; id: string; depth: number; path: string; plays: CutscenePlayDto[] }
  | { kind: "marker"; id: string; depth: number; marker: SceneMarker; name: string | null };

/**
 * What a row is nested in, outermost first: an answer of a choice, a branch
 * or its otherwise, a loop, a cutscene, or the group of lines no scene
 * plays. The view draws one rail per level in the container's color.
 */
export type RailKind = "choice" | "branch" | "loop" | "cutscene" | "plain";

export type RowLayout = { rails: readonly RailKind[] };

export type SceneModel = {
  rows: SceneRow[];
  layout: RowLayout[];
  /** The first row of each line's binding key. */
  firstRow: ReadonlyMap<string, number>;
  /** Whether the rows follow a quest script. */
  scripted: boolean;
};

const choicePrompt = /^Q\d+$/;
const choiceOption = /^A\d+$/;

function isSystem(speaker: string): boolean {
  return speaker === "SYSTEM" || speaker.startsWith("SYSTEM_");
}

type LineGroup = "journal" | "objectives" | "system" | "other" | "choice" | `speech:${string}`;

function groupOf(line: DialogueLineDto): LineGroup {
  if (line.role === "journal") return "journal";
  if (line.role === "objective") return "objectives";
  if (line.role === "other" || line.speaker === null) return "other";
  if (isSystem(line.speaker)) return "system";
  if (choicePrompt.test(line.speaker) || choiceOption.test(line.speaker)) return "choice";
  return `speech:${line.speaker}`;
}

class RowBuilder {
  readonly rows: SceneRow[] = [];
  readonly layout: RowLayout[] = [];
  readonly firstRow = new Map<string, number>();
  private readonly occurrences = new Map<string, TranslationOccurrenceView>();

  constructor(occurrences: readonly TranslationOccurrenceView[]) {
    for (const occurrence of occurrences) this.occurrences.set(bindingKey(occurrence.binding), occurrence);
  }

  sceneLine(line: DialogueLineDto): SceneLine {
    const key = bindingKey(line.sourceBinding);
    return { key, binding: line.sourceBinding, rowKey: line.key, sourceMacro: line.sourceMacro, occurrence: this.occurrences.get(key) ?? null };
  }

  push(row: SceneRow, rails: readonly RailKind[] = []) {
    this.layout.push({ rails });
    if ((row.kind === "line" || (row.kind === "option" && row.line)) && !this.firstRow.has((row.kind === "line" ? row.line : row.line!).key)) {
      this.firstRow.set((row.kind === "line" ? row.line : row.line!).key, this.rows.length);
    }
    this.rows.push(row);
  }

  /** Lines grouped in row order: journal, objectives, speakers, system text, choices. */
  grouped(lines: readonly DialogueLineDto[], depth: number, prefix: string, rails: readonly RailKind[] = []) {
    let group: LineGroup | null = null;
    let answered = false;
    for (const dto of lines) {
      const line = this.sceneLine(dto);
      const lineGroup = groupOf(dto);
      const prompt = lineGroup === "choice" && choicePrompt.test(dto.speaker ?? "");
      // A prompt after an answer starts the next choice.
      if (group !== lineGroup || (prompt && answered)) {
        const id = `${prefix}${lineGroup}@${line.key}`;
        if (lineGroup.startsWith("speech:")) this.push({ kind: "speaker", id, depth, speaker: dto.speaker ?? "" }, rails);
        else this.push({ kind: "heading", id, depth, heading: lineGroup as SceneHeading, rowOrder: lineGroup === "choice" }, rails);
        answered = false;
      }
      group = lineGroup;
      if (lineGroup === "choice" && !prompt) answered = true;
      const style = lineGroup === "choice" ? (prompt ? "prompt" : "answer") : lineGroup === "other" ? "other" : "plain";
      this.push({ kind: "line", id: `${prefix}${line.key}`, depth, line, style }, rails);
    }
  }
}

/** Builds the rows of a sheet's scene; `collapsed` holds the ids of folded rows. */
export function buildSceneRows(
  dialogue: SheetDialogueDto,
  occurrences: readonly TranslationOccurrenceView[],
  collapsed: ReadonlySet<string> = new Set(),
): SceneModel {
  const builder = new RowBuilder(occurrences);
  if (dialogue.quest) builder.push({ kind: "quest", id: "quest", depth: 0, quest: dialogue.quest });
  if (dialogue.versions.length > 0) builder.push({ kind: "versions", id: "versions", depth: 0, sheets: dialogue.versions });
  const byKey = new Map<string, DialogueLineDto>();
  for (const line of dialogue.lines) byKey.set(line.key.toUpperCase(), line);
  const used = new Set<string>();
  const cutscenes = (dialogue.cutscenes ?? []);
  if (!dialogue.scenes && cutscenes.length === 0) {
    if (dialogue.scriptError) builder.push({ kind: "notice", id: "script-error", depth: 0, notice: "scriptError", message: dialogue.scriptError });
    builder.grouped(dialogue.lines, 0, "");
    return { rows: builder.rows, layout: builder.layout, firstRow: builder.firstRow, scripted: false };
  }
  if (dialogue.scriptError) builder.push({ kind: "notice", id: "script-error", depth: 0, notice: "scriptError", message: dialogue.scriptError });
  const scenes = dialogue.scenes ?? [];
  for (const scene of scenes) collectKeys(scene.nodes, used);

  builder.grouped(dialogue.lines.filter((line) => line.role === "journal" || line.role === "objective"), 0, "");
  scenes.forEach((scene, index) => {
    const id = `scene${index}`;
    const title: SectionTitle = scene.scene === null ? "handler" : sceneTitle(scene.nodes);
    builder.push({ kind: "section", id, depth: 0, title, scene: scene.scene, name: scene.handler, battle: scene.script });
    if (!scene.traced) builder.push({ kind: "notice", id: `${id}.untraced`, depth: 0, notice: "untraced", message: null });
    new FlowWriter(builder, byKey, collapsed).nodes(scene.nodes, 0, id);
  });
  // Cutscene files that name lines no scene plays, with those lines.
  cutscenes.forEach((cutscene, index) => {
    const lines = cutscene.lines.filter((key) => !used.has(key.toUpperCase()));
    if (lines.length === 0) return;
    const id = `cutscene${index}`;
    builder.push({ kind: "section", id, depth: 0, title: "cutscene", scene: null, name: cutscene.path, battle: null });
    builder.push({ kind: "plays", id: `${id}.plays`, depth: 0, path: cutscene.path, plays: cutscene.plays ?? [] });
    // A cutscene file names its lines, not the order its timeline plays them in: read them as row order does.
    builder.grouped(lines.flatMap((key) => byKey.get(key.toUpperCase()) ?? []), 0, `${id}:`);
    for (const key of lines) used.add(key.toUpperCase());
  });
  const rest = dialogue.lines.filter((line) => line.role !== "journal" && line.role !== "objective" && !used.has(line.key.toUpperCase()));
  // Battle talk is marked in its keys; the rest no file of the game names.
  const battleTalk = rest.filter((line) => line.key.toUpperCase().includes("BATTLETALK"));
  const unnamed = rest.filter((line) => !line.key.toUpperCase().includes("BATTLETALK"));
  if (battleTalk.length > 0) {
    builder.push({ kind: "heading", id: "battle-talk", depth: 0, heading: "battleTalk" });
    builder.grouped(battleTalk, 1, "battle-talk:", ["plain"]);
  }
  if (unnamed.length > 0) {
    builder.push({ kind: "heading", id: "unscripted", depth: 0, heading: dialogue.scenes ? "unscripted" : "notInCutscenes" });
    builder.grouped(unnamed, 1, "unscripted:", ["plain"]);
  }
  return { rows: builder.rows, layout: builder.layout, firstRow: builder.firstRow, scripted: dialogue.scenes !== null };
}

class FlowWriter {
  private readonly choices = new Map<number, AnswerContext>();
  private readonly builder: RowBuilder;
  private readonly lines: ReadonlyMap<string, DialogueLineDto>;
  private readonly collapsed: ReadonlySet<string>;

  constructor(builder: RowBuilder, lines: ReadonlyMap<string, DialogueLineDto>, collapsed: ReadonlySet<string>) {
    this.builder = builder;
    this.lines = lines;
    this.collapsed = collapsed;
  }

  private line(key: string): DialogueLineDto | undefined {
    return this.lines.get(key.toUpperCase());
  }

  nodes(nodes: readonly FlowNodeDto[], depth: number, path: string, rails: readonly RailKind[] = []) {
    const push = (row: SceneRow) => this.builder.push(row, rails);
    let speaker: string | null = null;
    nodes.forEach((node, index) => {
      const id = `${path}.${index}`;
      if (node.kind !== "line") speaker = null;
      switch (node.kind) {
        case "line": {
          const dto = this.line(node.key);
          if (!dto) {
            push({ kind: "missing", id, depth, key: node.key });
            speaker = null;
            return;
          }
          const label = dto.role === "speech" ? dto.speaker : null;
          if (label !== null && label !== speaker) push({ kind: "speaker", id: `${id}.speaker`, depth, speaker: label });
          speaker = label;
          push({ kind: "line", id, depth, line: this.builder.sceneLine(dto), style: dto.role === "other" ? "other" : "plain" });
          return;
        }
        case "choice": {
          this.choices.set(node.id, { choice: node.choice, options: node.options.map((option) => option.label) });
          push({ kind: "heading", id: `${id}.heading`, depth, heading: node.choice === "questOffer" ? "questOffer" : "choice" });
          node.prompts.forEach((key, promptIndex) => {
            const prompt = this.line(key);
            const promptId = `${id}.prompt${promptIndex}`;
            if (prompt) push({ kind: "line", id: promptId, depth, line: this.builder.sceneLine(prompt), style: "prompt" });
            else push({ kind: "missing", id: promptId, depth, key });
          });
          node.options.forEach((option, optionIndex) => {
            const optionId = `${id}.o${optionIndex}`;
            const dto = option.label.kind === "text" ? this.line(option.label.key) : undefined;
            const folded = this.collapsed.has(optionId);
            push({
              kind: "option",
              id: optionId,
              depth,
              label: option.label,
              available: option.available ?? { kind: "always" },
              line: dto ? this.builder.sceneLine(dto) : null,
              empty: option.then.length === 0,
              collapsed: folded,
            });
            if (!folded) this.nodes(option.then, depth + 1, optionId, [...rails, "choice"]);
          });
          return;
        }
        case "branch": {
          const negated = node.then.length === 0;
          const answer = node.condition.kind === "test" && node.condition.subject.kind === "answer" ? this.choices.get(node.condition.subject.choice) ?? null : null;
          const folded = this.collapsed.has(id);
          push({ kind: "branch", id, depth, condition: node.condition, negated, answer, collapsed: folded });
          if (!folded) this.nodes(negated ? node.otherwise : node.then, depth + 1, `${id}.t`, [...rails, "branch"]);
          if (!negated && node.otherwise.length > 0) {
            const otherwiseId = `${id}.e`;
            const otherwiseFolded = this.collapsed.has(otherwiseId);
            push({ kind: "otherwise", id: otherwiseId, depth, collapsed: otherwiseFolded });
            if (!otherwiseFolded) this.nodes(node.otherwise, depth + 1, otherwiseId, [...rails, "branch"]);
          }
          return;
        }
        case "loop": {
          const folded = this.collapsed.has(id);
          push({ kind: "loop", id, depth, collapsed: folded });
          if (!folded) this.nodes(node.body, depth + 1, id, [...rails, "loop"]);
          return;
        }
        case "repeat":
          push({ kind: "marker", id, depth, marker: "repeat", name: null });
          return;
        case "cutscene": {
          const folded = this.collapsed.has(id);
          const empty = node.lines.length === 0 && node.sheets.length === 0;
          const path = node.path ?? null;
          push({ kind: "cutscene", id, depth, name: node.name, path, empty, collapsed: folded });
          if (folded) return;
          const known = node.lines.flatMap((key) => this.lines.get(key.toUpperCase()) ?? []);
          this.builder.grouped(known, depth + 1, `${id}:`, [...rails, "cutscene"]);
          for (const key of node.lines.filter((key) => !this.lines.has(key.toUpperCase()))) {
            this.builder.push({ kind: "missing", id: `${id}:${key}`, depth: depth + 1, key }, [...rails, "cutscene"]);
          }
          if (node.sheets.length > 0) this.builder.push({ kind: "sheets", id: `${id}.sheets`, depth: depth + 1, sheets: node.sheets, path }, [...rails, "cutscene"]);
          return;
        }
        default:
          push({ kind: "marker", id, depth, marker: node.kind, name: null });
      }
    });
  }
}

/** Every line key a scene plays, asks, or offers as an answer, in upper case. */
function collectKeys(nodes: readonly FlowNodeDto[], keys: Set<string>) {
  for (const node of nodes) {
    switch (node.kind) {
      case "line":
        keys.add(node.key.toUpperCase());
        break;
      case "choice":
        for (const prompt of node.prompts) keys.add(prompt.toUpperCase());
        for (const option of node.options) {
          if (option.label.kind === "text") keys.add(option.label.key.toUpperCase());
          collectKeys(option.then, keys);
        }
        break;
      case "branch":
        collectKeys(node.then, keys);
        collectKeys(node.otherwise, keys);
        break;
      case "loop":
        collectKeys(node.body, keys);
        break;
      case "cutscene":
        for (const key of node.lines) keys.add(key.toUpperCase());
        break;
      default:
        break;
    }
  }
}

function contains(nodes: readonly FlowNodeDto[], found: (node: FlowNodeDto) => boolean): boolean {
  return nodes.some((node) => found(node)
    || (node.kind === "choice" && node.options.some((option) => contains(option.then, found)))
    || (node.kind === "branch" && (contains(node.then, found) || contains(node.otherwise, found)))
    || (node.kind === "loop" && contains(node.body, found)));
}

/** A scene that offers the quest accepts it; one that completes it ends it. */
function sceneTitle(nodes: readonly FlowNodeDto[]): "accepting" | "completing" | "scene" {
  if (contains(nodes, (node) => node.kind === "choice" && node.choice === "questOffer")) return "accepting";
  if (contains(nodes, (node) => node.kind === "completed")) return "completing";
  return "scene";
}

/**
 * The option a comparison of an answer selects: a menu answers with its
 * option's number, a yes/no question and a quest offer with `true` for the
 * first option and `false` for the second. `null` when it names no option.
 */
export function answerOption(answer: AnswerContext, condition: ConditionDto): number | null {
  const { test } = condition;
  if (test.kind === "truthy") return answer.choice === "menu" ? null : test.value ? 0 : 1;
  if (test.comparison !== "eq" && test.comparison !== "ne") return null;
  let index: number | null = null;
  if (test.value.kind === "number" && answer.choice === "menu") index = test.value.value - 1;
  if (test.value.kind === "boolean" && answer.choice !== "menu") index = test.value.value ? 0 : 1;
  if (index === null || !Number.isInteger(index) || index < 0 || index >= answer.options.length) return null;
  if (test.comparison === "eq") return index;
  // `≠` picks one option only when there are two.
  return answer.options.length === 2 ? 1 - index : null;
}

/**
 * A speaker label as a name: `AMHGARANJY_GEVA` reads as `Amhgaranjy Geva`.
 * Labels are the game's internal names, so the label itself stays available
 * wherever the name is shown.
 */
export function speakerName(label: string): string {
  return label
    .split("_")
    .filter((segment) => segment.length > 0)
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1).toLowerCase())
    .join(" ");
}

/**
 * The row a jump to a cutscene file lands on: the cutscene a quest's scene
 * plays, or the section of a cutscene file; `-1` when the sheet shows none.
 */
export function cutsceneRow(rows: readonly SceneRow[], path: string): number {
  return rows.findIndex((row) =>
    (row.kind === "cutscene" && row.path === path)
    || (row.kind === "section" && row.title === "cutscene" && row.name === path));
}

/** Quest and cutscene sheets can have a scene; other sheets never do. */
export function mayHaveScene(sheetName: string | null): boolean {
  return sheetName !== null && (sheetName.startsWith("quest/") || sheetName.startsWith("cut_scene/"));
}
