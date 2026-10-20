import { useEffect, useState } from "react";
import { Dialog } from "radix-ui";
import { instructions, normalizeCommandError } from "../ipc";
import type { InstructionDto } from "../types";
import { useI18n } from "../ui/i18n";
import { Select } from "../ui/primitives/Select";
import { useStickyState } from "./GitShared";

type FixDialogProps = {
  /** How many strings to correct; the dialog is closed while null. */
  count: number | null;
  /** The issue the search keeps, or null for every issue; the issues go when the search filters by them. */
  issue: string | null;
  /** The search keeps strings with issues of the checks: those are fixed too. */
  withIssues: boolean;
  /** How many of the strings have a changed source. */
  fuzzy: number;
  onCancel: () => void;
  onFix: (instruction: string | null, adapt: boolean) => void;
};

/** What a correction with AI asks of each found translation: one of the project's instructions, and the issues and adaptation the strings need. */
export function FixDialog({ count, issue, withIssues, fuzzy, onCancel, onFix }: FixDialogProps) {
  const { t } = useI18n();
  const [adapt, setAdapt] = useState(fuzzy > 0);
  // The last instruction stays while the window lives: the same one is often asked of several results.
  const [instruction, setInstruction] = useStickyState("search.fixInstruction", "");
  const [list, setList] = useState<InstructionDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (count === null) return;
    setAdapt(fuzzy > 0);
    setList(null);
    setError(null);
    void instructions()
      .then((found) => setList(found.instructions))
      .catch((reason: unknown) => setError(normalizeCommandError(reason).message));
  }, [count, fuzzy]);

  const chosen = list?.some((item) => item.name === instruction) ? instruction : "";
  const ready = chosen !== "" || withIssues || (adapt && fuzzy > 0);
  return (
    <Dialog.Root open={count !== null} onOpenChange={(next) => { if (!next) onCancel(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog fix-dialog" aria-describedby={undefined}>
          <Dialog.Title className="dialog-title">{t("fix.title")}</Dialog.Title>
          <p className="dialog-description">{t("fix.summary", { count: count ?? 0 })}</p>
          <form
            className="fix-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (ready) onFix(chosen || null, adapt && fuzzy > 0);
            }}
          >
            <div className="field">
              <span className="field-label">{t("fix.instruction")}</span>
              <Select
                value={chosen}
                label={t("fix.instruction")}
                disabled={list === null}
                options={[{ value: "", label: t("fix.noInstruction") }, ...(list ?? []).map((item) => ({ value: item.name, label: item.name }))]}
                onChange={setInstruction}
              />
            </div>
            {list !== null && list.length === 0 ? <p className="field-hint">{t("fix.noInstructions")}</p> : null}
            {error ? <p className="glossary-problem">{error}</p> : null}
            {withIssues ? <p className="field-hint">{issue === null ? t("fix.issues") : t("fix.issuesOf", { issue })}</p> : null}
            {fuzzy > 0 ? (
              <label className="checkbox">
                <input type="checkbox" checked={adapt} onChange={(event) => setAdapt(event.target.checked)} />
                {t("fix.adapt", { count: fuzzy })}
              </label>
            ) : null}
            <div className="dialog-actions">
              <button className="button button-secondary" type="button" onClick={onCancel}>{t("common.cancel")}</button>
              <button className="button button-primary" type="submit" disabled={!ready}>{t("fix.action")}</button>
            </div>
          </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
