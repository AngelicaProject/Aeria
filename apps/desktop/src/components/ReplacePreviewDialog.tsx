import { useEffect, useMemo, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { useVirtualizer } from "@tanstack/react-virtual";
import { describeIssue } from "../issueText";
import { diffWords } from "../textDiff";
import type { ReplaceChangeDto, SourceBinding } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

type ReplacePreviewDialogProps = {
  /** The changes to show; the dialog is closed while null. */
  changes: readonly ReplaceChangeDto[] | null;
  onCancel: () => void;
  onApply: (changes: readonly ReplaceChangeDto[]) => void;
  onRevealBinding?: ((binding: SourceBinding) => void) | undefined;
};

function keyOf(change: ReplaceChangeDto): string {
  return `${change.path}|${change.context}`;
}

/**
 * The changes a replacement makes, before anything is written: each
 * translation before and after, with the words that change marked. Changes
 * that would break their string are shown with the reason and cannot be
 * chosen; the rest can be unchecked one by one.
 */
export function ReplacePreviewDialog({ changes, onCancel, onApply, onRevealBinding }: ReplacePreviewDialogProps) {
  const { t } = useI18n();
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set());
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    setChosen(new Set((changes ?? []).filter((change) => change.problems.length === 0).map(keyOf)));
  }, [changes]);

  const list = changes ?? [];
  const valid = useMemo(() => list.filter((change) => change.problems.length === 0), [list]);
  const virtualizer = useVirtualizer({
    count: list.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => 88,
    overscan: 8,
  });

  const toggle = (key: string) => setChosen((current) => {
    const next = new Set(current);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  });
  const allChosen = valid.length > 0 && valid.every((change) => chosen.has(keyOf(change)));

  return (
    <Dialog.Root open={changes !== null} onOpenChange={(next) => { if (!next) onCancel(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog replace-preview" aria-describedby={undefined}>
          <Dialog.Title className="dialog-title">{t("search.preview.title")}</Dialog.Title>
          <p className="dialog-description">
            {t("search.preview.summary", { count: list.length })}
            {list.length > valid.length ? ` ${t("search.preview.invalid", { count: list.length - valid.length })}` : ""}
          </p>
          <label className="checkbox replace-preview-all">
            <input type="checkbox" checked={allChosen} disabled={valid.length === 0} onChange={() => setChosen(allChosen ? new Set() : new Set(valid.map(keyOf)))} />
            {t("search.preview.chooseAll")}
          </label>
          <div className="replace-preview-list" ref={listRef}>
            <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
              {virtualizer.getVirtualItems().map((item) => {
                const change = list[item.index];
                if (!change) return null;
                const key = keyOf(change);
                const invalid = change.problems.length > 0;
                return (
                  <div key={key} ref={virtualizer.measureElement} data-index={item.index} className={invalid ? "replace-change invalid" : "replace-change"} style={{ position: "absolute", top: 0, left: 0, right: 0, transform: `translateY(${item.start}px)` }}>
                    <input type="checkbox" aria-label={t("search.preview.choose")} checked={!invalid && chosen.has(key)} disabled={invalid} onChange={() => toggle(key)} />
                    <div className="replace-change-body">
                      <button type="button" className="link-button mono replace-change-where" disabled={!change.binding} onClick={() => { if (change.binding) onRevealBinding?.(change.binding); }}>
                        {change.binding ? `${change.binding.sheetName} ${change.binding.rowId}:${change.binding.subrowId}` : change.context}
                        {change.fuzzy ? <span className="chip chip-warn">fuzzy</span> : null}
                      </button>
                      <div className="git-diff">
                        {diffWords(change.before, change.after).map((part, index) => part.kind === "same"
                          ? <span key={index}>{part.text}</span>
                          : part.kind === "added" ? <ins key={index}>{part.text}</ins> : <del key={index}>{part.text}</del>)}
                      </div>
                      {invalid ? <p className="replace-change-problems"><UiIcon icon="circleAlert" size="xs" />{change.problems.map((issue) => describeIssue(issue, t)).join("; ")}</p> : null}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
          <div className="dialog-actions">
            <Dialog.Close className="button button-secondary">{t("common.cancel")}</Dialog.Close>
            <button className="button button-primary" type="button" disabled={chosen.size === 0} onClick={() => onApply(valid.filter((change) => chosen.has(keyOf(change))))}>
              {t("search.preview.apply", { count: chosen.size })}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
