import { useEffect, useState } from "react";
import { Dialog } from "radix-ui";
import type { GlossaryRow } from "../glossaryModel";
import type { GlossaryCategory } from "../types";
import { useI18n } from "../ui/i18n";

export function NewCategoryDialog({ open, categories, onClose, onCreate }: { open: boolean; categories: readonly GlossaryCategory[]; onClose: () => void; onCreate: (name: string) => void }) {
  const { t } = useI18n();
  const [name, setName] = useState("");
  useEffect(() => { if (open) setName(""); }, [open]);
  const exists = categories.some((c) => c.name.trim().toLowerCase() === name.trim().toLowerCase());
  return <Dialog.Root open={open} onOpenChange={(next) => { if (!next) onClose(); }}><Dialog.Portal>
    <Dialog.Overlay className="dialog-overlay" />
    <Dialog.Content className="dialog confirm-dialog">
      <Dialog.Title className="dialog-title">{t("glossary.category.add")}</Dialog.Title>
      <Dialog.Description className="dialog-description">{t("glossary.category.independent")}</Dialog.Description>
      <form onSubmit={(event) => { event.preventDefault(); if (name.trim() && !exists) onCreate(name.trim()); }}>
        <label className="glossary-field">{t("glossary.category.name")}<input className="input" autoFocus value={name} onChange={(event) => setName(event.target.value)} /></label>
        {exists ? <p className="glossary-problem">{t("glossary.category.exists")}</p> : null}
        <div className="dialog-actions"><button type="button" className="button button-secondary" onClick={onClose}>{t("common.cancel")}</button><button type="submit" className="button button-primary" disabled={!name.trim() || exists}>{t("glossary.category.add")}</button></div>
      </form>
    </Dialog.Content>
  </Dialog.Portal></Dialog.Root>;
}

export type CategoryMergeDraft = { source: GlossaryCategory; target: GlossaryCategory; pairs: { source: GlossaryRow; target: GlossaryRow }[] };

function sameContent(a: GlossaryRow, b: GlossaryRow): boolean {
  return a.translation === b.translation && a.note === b.note && a.matchCase === b.matchCase
    && JSON.stringify(a.forms) === JSON.stringify(b.forms);
}

export function MergeCategoriesDialog({ draft, onClose, onMerge }: { draft: CategoryMergeDraft | null; onClose: () => void; onMerge: (choices: ReadonlyMap<number, number>) => void }) {
  const { t } = useI18n();
  const [choices, setChoices] = useState<ReadonlyMap<number, number>>(new Map());
  useEffect(() => { setChoices(new Map()); }, [draft]);
  const conflicts = draft?.pairs.filter((pair) => !sameContent(pair.source, pair.target)) ?? [];
  const ready = conflicts.every((pair) => choices.has(pair.source.key));
  return <Dialog.Root open={draft !== null} onOpenChange={(next) => { if (!next) onClose(); }}><Dialog.Portal>
    <Dialog.Overlay className="dialog-overlay" />
    <Dialog.Content className="dialog glossary-merge-dialog">
      <Dialog.Title className="dialog-title">{t("glossary.category.merge")}</Dialog.Title>
      <Dialog.Description className="dialog-description">{draft ? t("glossary.category.mergeDescription", { source: draft.source.name, target: draft.target.name }) : ""}</Dialog.Description>
      <div className="glossary-merge-conflicts">
        {conflicts.map((pair) => <fieldset key={pair.source.key} className="glossary-field">
          <legend>{pair.source.term}</legend>
          <p className="muted">{t("glossary.category.chooseDefinition")}</p>
          {[pair.source, pair.target].map((row) => <label key={row.key} className="glossary-merge-choice">
            <input type="radio" name={`merge-${pair.source.key}`} checked={choices.get(pair.source.key) === row.key} onChange={() => setChoices((current) => new Map(current).set(pair.source.key, row.key))} />
            <span><strong>{row.translation}</strong> · {row.category}<br />{row.note}<br />{row.forms.join("; ")} {row.matchCase ? t("glossary.field.matchCase") : ""}</span>
          </label>)}
        </fieldset>)}
      </div>
      <div className="dialog-actions"><button type="button" className="button button-secondary" onClick={onClose}>{t("common.cancel")}</button><button type="button" className="button button-primary" disabled={!ready} onClick={() => onMerge(choices)}>{t("glossary.category.merge")}</button></div>
    </Dialog.Content>
  </Dialog.Portal></Dialog.Root>;
}
