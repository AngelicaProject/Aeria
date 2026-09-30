import { memo, useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { domKey } from "../binding";
import { answerOption, buildSceneRows, cutsceneRow, speakerName, type SceneHeading, type SceneLine, type SceneMarker, type SceneRow } from "../dialogueScene";
import type { TranslationOccurrenceView } from "../translationOccurrences";
import type { AvailabilityDto, ChangeMark, CutscenePlayDto, OptionLabelDto, SheetDialogueDto, SourceBinding } from "../types";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";
import { guardText, negate, operandCode } from "../sceneConditions";
import { MacroPreview } from "./MacroPreview";
import { ReviewDot } from "./ReviewDot";

type DialogueSceneProps = {
  dialogue: SheetDialogueDto;
  sheetName: string;
  /** Every loaded string of the sheet. */
  occurrences: readonly TranslationOccurrenceView[];
  /** Binding keys that pass the list filter; `null` when no filter is active. */
  matching: ReadonlySet<string> | null;
  selectedKey: string | null;
  disabled: boolean;
  onSelect: (occurrence: TranslationOccurrenceView) => void;
  onNavigate: (direction: 1 | -1) => void;
  onReveal: (binding: SourceBinding) => void;
  /** Opens another sheet; with a cutscene file's path, at that cutscene. */
  onOpenSheet: (sheetName: string, cutscene?: string) => void;
  changedKinds: ReadonlyMap<string, ChangeMark>;
  /** A cutscene file's path to scroll to and mark once the scene shows it. */
  target: string | null;
  /** The target was scrolled to, or the sheet does not show it. */
  onTargetShown: () => void;
  /** The scene of the sheet shown before, kept dimmed while the next one loads. */
  stale?: boolean;
};

const headingTitles: Readonly<Record<SceneHeading, MessageKey>> = {
  journal: "scene.journal",
  objectives: "scene.objectives",
  system: "scene.system",
  other: "scene.other",
  choice: "scene.choice",
  questOffer: "scene.questOffer",
  unscripted: "scene.unscripted",
  battleTalk: "scene.battleTalk",
  notInCutscenes: "scene.notInCutscenes",
};

/** Readable titles of the script functions that show text outside scenes, by name. */
const handlerTitles: ReadonlyArray<[RegExp, MessageKey]> = [
  [/BalloonTalk/, "scene.handler.balloon"],
  [/Chas|Accompany|TalkChase/, "scene.handler.escort"],
  [/MenuTextLabels/, "scene.handler.menu"],
  [/SayEvent/, "scene.handler.say"],
  [/PopEnemy/, "scene.handler.enemy"],
  [/^OnSequence\d+$/, "scene.handler.sequence"],
  [/Lcut/i, "scene.handler.localCutscene"],
  [/LetterText/, "scene.handler.letter"],
  [/Todo/, "scene.handler.objective"],
];

function handlerTitle(name: string | null): MessageKey {
  if (name === null) return "scene.handler.unnamed";
  return handlerTitles.find(([pattern]) => pattern.test(name))?.[1] ?? "scene.handler.named";
}

const markers: Readonly<Record<SceneMarker, { icon: UiIconName; label: MessageKey }>> = {
  repeat: { icon: "refreshCw", label: "scene.marker.repeat" },
  cancelled: { icon: "circleX", label: "scene.marker.cancelled" },
  accepted: { icon: "circleCheck", label: "scene.marker.accepted" },
  completed: { icon: "circleCheck", label: "scene.marker.completed" },
};

const optionLabels: Readonly<Record<Exclude<OptionLabelDto["kind"], "text">, MessageKey>> = {
  accept: "scene.option.accept",
  decline: "scene.option.decline",
  yes: "scene.option.yes",
  no: "scene.option.no",
  script: "scene.option.script",
};

/** The estimated height of a row before it is measured. */
function estimateRow(row: SceneRow): number {
  switch (row.kind) {
    case "quest": return 58;
    case "versions": return 28;
    case "section": return 44;
    case "line": return 32;
    case "option": return row.line ? 32 : 28;
    default: return 26;
  }
}

/**
 * A quest or cutscene sheet as a scene. With the quest's script, each scene
 * reads in the order the game plays it, with choices, conditions, and loops
 * that fold; otherwise lines are grouped by speaker in row order. Selecting
 * a line opens its string in the editor, as in the strings list.
 */
export const DialogueScene = memo(function DialogueScene({
  dialogue,
  sheetName,
  occurrences,
  matching,
  selectedKey,
  disabled,
  onSelect,
  onNavigate,
  onReveal,
  onOpenSheet,
  changedKinds,
  target,
  onTargetShown,
  stale = false,
}: DialogueSceneProps) {
  const { t } = useI18n();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [collapsed, setCollapsed] = useState<{ sheet: string; ids: ReadonlySet<string> }>({ sheet: sheetName, ids: new Set() });
  const collapsedIds = collapsed.sheet === sheetName ? collapsed.ids : null;
  const model = useMemo(() => buildSceneRows(dialogue, occurrences, collapsedIds ?? new Set()), [dialogue, occurrences, collapsedIds]);
  const virtualizer = useVirtualizer({
    count: model.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateRow(model.rows[index]!),
    getItemKey: (index) => model.rows[index]!.id,
    overscan: 12,
  });

  const selectedRow = selectedKey ? model.firstRow.get(selectedKey) ?? -1 : -1;
  useEffect(() => {
    if (selectedRow >= 0) virtualizer.scrollToIndex(selectedRow, { align: "auto" });
  }, [selectedRow, virtualizer]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [sheetName]);

  // A jump from another sheet lands on the cutscene it names, marked for a moment.
  const [marked, setMarked] = useState<string | null>(null);
  useEffect(() => {
    if (target === null) return;
    const index = cutsceneRow(model.rows, target);
    onTargetShown();
    if (index < 0) return;
    // A cutscene file's section starts the view; a cutscene a scene plays sits among the scene's lines.
    virtualizer.scrollToIndex(index, { align: model.rows[index]!.kind === "section" ? "start" : "center" });
    setMarked(model.rows[index]!.id);
  }, [target, model, virtualizer, onTargetShown]);
  useEffect(() => {
    if (marked === null) return;
    const timer = window.setTimeout(() => setMarked(null), 2400);
    return () => window.clearTimeout(timer);
  }, [marked]);

  function toggle(id: string) {
    setCollapsed((current) => {
      const ids = new Set(current.sheet === sheetName ? current.ids : []);
      if (ids.has(id)) ids.delete(id);
      else ids.add(id);
      return { sheet: sheetName, ids };
    });
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      onNavigate(event.key === "ArrowDown" ? 1 : -1);
    }
  }

  function lineClasses(line: SceneLine): string {
    const changeKind = changedKinds.get(line.key);
    return `${line.key === selectedKey ? " selected" : ""}${matching !== null && !matching.has(line.key) ? " dimmed" : ""}${line.occurrence === null ? " inert" : ""}${changeKind ? ` git-${changeKind}` : ""}`;
  }

  function select(line: SceneLine) {
    if (!disabled && line.occurrence) onSelect(line.occurrence);
  }

  function texts(line: SceneLine, marker?: ReactNode) {
    return (
      <>
        <span className="scene-text scene-source">
          {marker}
          <MacroPreview text={line.occurrence?.sourceMacro ?? line.sourceMacro} empty="" multiline />
        </span>
        <span className="scene-text scene-target">
          {line.occurrence ? <MacroPreview text={line.occurrence.targetMacro} empty={t("review.untranslated")} multiline /> : null}
        </span>
      </>
    );
  }

  function toggleButton(row: { id: string; collapsed: boolean }) {
    return (
      <button
        type="button"
        className="scene-toggle"
        aria-expanded={!row.collapsed}
        aria-label={t(row.collapsed ? "scene.expand" : "scene.collapse")}
        onClick={(event) => { event.stopPropagation(); toggle(row.id); }}
      >
        <UiIcon icon={row.collapsed ? "chevronRight" : "chevronDown"} size="xs" />
      </button>
    );
  }

  function optionText(label: OptionLabelDto, line: SceneLine | null): ReactNode {
    if (line) return <MacroPreview text={line.occurrence?.sourceMacro ?? line.sourceMacro} empty="" />;
    if (label.kind === "text") return <span className="mono">{label.key}</span>;
    return t(optionLabels[label.kind]);
  }

  function branchText(row: Extract<SceneRow, { kind: "branch" }>): ReactNode {
    const condition = row.negated ? negate(row.condition) : row.condition;
    if (row.answer && condition.kind === "test") {
      const index = answerOption(row.answer, condition);
      const label = index === null ? undefined : row.answer.options[index];
      if (label) {
        const line = label.kind === "text" ? dialogue.lines.find((candidate) => candidate.key.toUpperCase() === label.key.toUpperCase()) : undefined;
        return (
          <>
            <span className="scene-flow-keyword">{t("scene.ifAnswer")}</span>{" "}
            <span className="scene-answer">{line ? <MacroPreview text={line.sourceMacro} empty="" /> : optionText(label, null)}</span>
          </>
        );
      }
    }
    return <><span className="scene-flow-keyword">{t("scene.if")}</span> <span className="scene-condition-text">{guardText(t, condition)}</span></>;
  }

  function sectionTitle(row: Extract<SceneRow, { kind: "section" }>): string {
    const scene = String(row.scene ?? "");
    const title = row.title === "cutscene"
      ? t("scene.section.cutscene")
      : row.title === "handler"
        ? t(handlerTitle(row.name))
        : t(row.title === "accepting" ? "scene.section.accepting" : row.title === "completing" ? "scene.section.completing" : "scene.section.scene", { scene });
    return row.battle ? t("scene.section.battle", { title }) : title;
  }

  /** The scene number, handler name, battle script, or cutscene path beside a section's title. */
  function sectionDetail(row: Extract<SceneRow, { kind: "section" }>): ReactNode {
    const details: ReactNode[] = [];
    if (row.title === "accepting" || row.title === "completing") details.push(t("scene.section.scene", { scene: String(row.scene ?? "") }));
    if (row.title === "handler" && row.name) details.push(<span className="mono" key="name">{row.name}</span>);
    if (row.title === "cutscene") details.push(<span className="mono" key="path" title={t("scene.section.cutsceneHint")}>{row.name}</span>);
    if (row.battle) details.push(<span className="mono" key="battle" title={t("scene.section.battleHint", { script: row.battle })}>{row.battle}</span>);
    if (details.length === 0) return null;
    return <span className="scene-section-number">{details.map((detail, index) => <span key={index}>{index > 0 ? " · " : null}{detail}</span>)}</span>;
  }

  /** When a grayed-out answer can be picked, under the answer. */
  function availabilityNote(available: AvailabilityDto): ReactNode {
    switch (available.kind) {
      case "always": return null;
      case "never": return <span className="scene-answer-note">{t("scene.option.unavailable")}</span>;
      case "unknown": return <span className="scene-answer-note">{t("scene.option.maybeUnavailable")}</span>;
      case "when": return <span className="scene-answer-note"><span className="scene-flow-keyword">{t("scene.option.availableIf")}</span> {guardText(t, available.guard)}</span>;
    }
  }

  /** Where a quest plays a cutscene: its name, then its scene or handler. */
  function playLabel(play: CutscenePlayDto): string {
    const quest = play.name ?? play.quest.split("/").pop() ?? play.quest;
    const where = play.scene !== null
      ? t("scene.section.scene", { scene: String(play.scene) })
      : t(handlerTitle(play.handler));
    return `${quest} · ${play.script ? t("scene.section.battle", { title: where }) : where}`;
  }

  function renderRow(row: SceneRow, index: number): ReactNode {
    const depthStyle = { "--depth": row.depth } as CSSProperties;
    switch (row.kind) {
      case "quest":
        return (
          <div className="scene-row scene-quest" style={depthStyle}>
            <span className="scene-heading">{t("scene.quest")}</span>
            <button type="button" className="scene-quest-name" title={t("scene.openQuestName")} disabled={disabled} onClick={() => onReveal(row.quest.sourceBinding)}>
              <strong><MacroPreview text={row.quest.targetMacro ?? row.quest.sourceMacro} empty="" /></strong>
              {row.quest.targetMacro !== null ? <span className="scene-quest-source"><MacroPreview text={row.quest.sourceMacro} empty="" /></span> : null}
            </button>
          </div>
        );
      case "versions":
        return (
          <div className="scene-row scene-versions" style={depthStyle}>
            <span className="scene-heading" title={t("scene.versionsHint")}>{t("scene.versions")}</span>
            {row.sheets.map((sheet) => (
              <button key={sheet} type="button" className="scene-version mono" title={sheet} disabled={disabled} onClick={() => onOpenSheet(sheet)}>
                {sheet.slice(sheet.lastIndexOf("/") + 1)}
              </button>
            ))}
          </div>
        );
      case "section":
        return (
          <div className="scene-row scene-section" style={depthStyle}>
            <span className="scene-section-title">{sectionTitle(row)}</span>
            {sectionDetail(row)}
          </div>
        );
      case "notice":
        return (
          <div className="scene-row scene-notice" style={depthStyle} role="note">
            <UiIcon icon="triangleAlert" size="sm" />
            <span>{row.notice === "scriptError" ? t("scene.scriptError", { message: row.message ?? "" }) : t("scene.untraced")}</span>
          </div>
        );
      case "heading":
        if (row.heading === "choice" || row.heading === "questOffer") {
          return (
            <div className="scene-row scene-choice-heading" style={depthStyle}>
              <UiIcon icon={row.heading === "questOffer" ? "circleHelp" : "messageSquare"} size="xs" />
              <span>{t(headingTitles[row.heading])}</span>
              {row.rowOrder ? <span className="scene-choice-order" title={t("scene.choiceRowOrderHint")}>{t("scene.choiceRowOrder")}</span> : null}
            </div>
          );
        }
        return <div className={`scene-row scene-heading-row${row.heading === "unscripted" ? " scene-heading-major" : ""}`} style={depthStyle}><span className="scene-heading">{t(headingTitles[row.heading])}</span></div>;
      case "speaker":
        return (
          <div className="scene-row scene-speaker-row" style={depthStyle}>
            <span className="scene-speaker" title={t("scene.speakerLabel", { label: row.speaker })}>{isSystemLabel(row.speaker) ? t("scene.system") : speakerName(row.speaker)}</span>
          </div>
        );
      case "line":
        return (
          <div
            id={model.firstRow.get(row.line.key) === index ? `scene-${domKey(row.line.key)}` : undefined}
            className={`scene-row scene-line scene-line-${row.style}${lineClasses(row.line)}`}
            style={depthStyle}
            role="option"
            aria-selected={row.line.key === selectedKey}
            aria-disabled={disabled || row.line.occurrence === null || undefined}
            title={row.line.occurrence === null ? t("scene.notTranslatable") : undefined}
            onClick={() => select(row.line)}
          >
            <span className="lens-status">{row.line.occurrence ? <ReviewDot state={row.line.occurrence.state} /> : null}</span>
            {texts(row.line, row.style === "other" ? <span className="lens-tag mono">{row.line.rowKey}</span> : undefined)}
          </div>
        );
      case "missing":
        return (
          <div className="scene-row scene-line inert" style={depthStyle} title={t("scene.missingHint")}>
            <span className="lens-status" />
            <span className="scene-text scene-source"><span className="lens-tag mono">{row.key}</span>{t("scene.missing")}</span>
            <span />
          </div>
        );
      case "option":
        return (
          <div
            className={`scene-row scene-option${row.line || !row.empty ? " clickable" : ""}${row.line ? lineClasses(row.line) : ""}`}
            style={depthStyle}
            role={row.line ? "option" : undefined}
            aria-selected={row.line ? row.line.key === selectedKey : undefined}
            onClick={() => { if (row.line) select(row.line); else if (!row.empty) toggle(row.id); }}
          >
            <span className="lens-status">{row.empty ? <span className="scene-option-end" title={t("scene.optionEmpty")}><UiIcon icon="minus" size="xs" /></span> : toggleButton(row)}</span>
            <span className="scene-text scene-source">
              <span className={`scene-answer-pill${row.available.kind === "never" ? " unavailable" : ""}`}>{row.line ? <MacroPreview text={row.line.occurrence?.sourceMacro ?? row.line.sourceMacro} empty="" /> : optionText(row.label, null)}</span>
              {availabilityNote(row.available)}
            </span>
            <span className="scene-text scene-target">
              {row.line?.occurrence ? <MacroPreview text={row.line.occurrence.targetMacro} empty={t("review.untranslated")} /> : null}
            </span>
          </div>
        );
      case "branch":
        return (
          <div className="scene-row scene-branch scene-flow" style={depthStyle} onClick={() => toggle(row.id)}>
            <span className="lens-status">{toggleButton(row)}</span>
            <span className="scene-branch-text">{branchText(row)}</span>
          </div>
        );
      case "otherwise":
        return (
          <div className="scene-row scene-branch scene-flow" style={depthStyle} onClick={() => toggle(row.id)}>
            <span className="lens-status">{toggleButton(row)}</span>
            <span className="scene-branch-text"><span className="scene-flow-keyword">{t("scene.otherwise")}</span></span>
          </div>
        );
      case "loop":
        return (
          <div className="scene-row scene-branch scene-flow" style={depthStyle} onClick={() => toggle(row.id)}>
            <span className="lens-status">{toggleButton(row)}</span>
            <span className="scene-branch-text"><UiIcon icon="refreshCw" size="xs" />{t("scene.loop")}</span>
          </div>
        );
      case "cutscene":
        return (
          <div className={`scene-row scene-branch scene-cutscene${row.empty ? " inert" : ""}`} style={depthStyle} title={row.empty ? t("scene.cutsceneEmpty") : t("scene.cutsceneHint")} onClick={() => { if (!row.empty) toggle(row.id); }}>
            <span className="lens-status">{row.empty ? null : toggleButton(row)}</span>
            <span className="scene-branch-text"><UiIcon icon="play" size="xs" />{t("scene.marker.cutscene")}{row.name ? <span className="lens-tag mono scene-cutscene-name" title={row.path ? t("scene.cutscenePath", { path: row.path }) : t("scene.cutsceneName")}>{row.name}</span> : null}</span>
          </div>
        );
      case "sheets":
        return (
          <div className="scene-row scene-versions" style={depthStyle}>
            <span className="scene-heading">{t("scene.cutsceneSheets")}</span>
            {row.sheets.map((sheet) => (
              <button
                key={sheet}
                type="button"
                className="scene-version mono"
                title={row.path ? t("scene.cutsceneSheetHint", { sheet, path: row.path }) : sheet}
                disabled={disabled}
                onClick={() => onOpenSheet(sheet, row.path ?? undefined)}
              >
                {sheet}
                {row.path ? <span className="scene-version-anchor">{row.path.split("/").pop()}</span> : null}
              </button>
            ))}
          </div>
        );
      case "plays":
        return (
          <div className="scene-row scene-versions" style={depthStyle}>
            <span className="scene-heading">{t("scene.playedIn")}</span>
            {row.plays.length === 0 ? <span className="scene-plays-none">{t("scene.playedByNone")}</span> : row.plays.map((play, playIndex) => (
              <button
                key={`${play.quest}.${playIndex}`}
                type="button"
                className="scene-version"
                title={t("scene.playedInHint", { sheet: play.quest })}
                disabled={disabled}
                onClick={() => onOpenSheet(play.quest, row.path)}
              >
                {playLabel(play)}
              </button>
            ))}
          </div>
        );
      case "marker": {
        const marker = markers[row.marker];
        return (
          <div className={`scene-row scene-marker scene-marker-${row.marker}`} style={depthStyle} title={row.name ?? undefined}>
            <span className="lens-status"><UiIcon icon={marker.icon} size="xs" /></span>
            <span>{t(marker.label)}</span>
          </div>
        );
      }
    }
  }

  if (model.rows.length === 0) {
    return (
      <div className="lens-state">
        <div className="empty-state">
          <UiIcon icon="messageSquare" size="xl" />
          <strong>{t("scene.empty")}</strong>
        </div>
      </div>
    );
  }

  return (
    <div
      ref={scrollRef}
      className={`lens-scroll scene-scroll${stale ? " is-stale" : ""}`}
      role="listbox"
      tabIndex={0}
      aria-label={t("scene.labelFor", { sheet: sheetName })}
      aria-activedescendant={selectedKey && selectedRow >= 0 ? `scene-${domKey(selectedKey)}` : undefined}
      onKeyDown={handleKeyDown}
    >
      <div className="lens-spacer" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((item) => (
          <div key={item.key} className={`scene-item${model.rows[item.index]?.id === marked ? " scene-jumped" : ""}`} data-index={item.index} ref={virtualizer.measureElement} style={{ transform: `translateY(${item.start}px)` }}>
            <div className={`scene-slice scene-slice-${model.rows[item.index]!.kind}`} style={{ "--depth": model.rows[item.index]!.depth } as CSSProperties}>
              {model.layout[item.index]!.rails.map((rail, level) => (
                <span key={level} className={`scene-rail scene-rail-${rail}`} style={{ "--level": level } as CSSProperties} aria-hidden="true" />
              ))}
              {renderRow(model.rows[item.index]!, item.index)}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
});

function isSystemLabel(label: string): boolean {
  return label === "SYSTEM" || label.startsWith("SYSTEM_");
}
