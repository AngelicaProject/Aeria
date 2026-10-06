import { useEffect, useState } from "react";
import { Dialog } from "radix-ui";
import { useI18n } from "../ui/i18n";
import { useStickyState } from "./GitShared";

type FixDialogProps = {
  /** How many strings to correct; the dialog is closed while null. */
  count: number | null;
  /** The issue the search keeps, or null for every issue of the checks. */
  issue: string | null;
  /** Whether fixing the issues of the checks starts chosen. */
  issuesFirst: boolean;
  /** How many of the strings have a changed source. */
  fuzzy: number;
  onCancel: () => void;
  onFix: (issues: boolean, proofread: boolean, adapt: boolean, request: string) => void;
};

/** What a correction with AI asks of each translation: a proofreading, the issues of the checks, a translator's request, or any of them together. */
export function FixDialog({ count, issue, issuesFirst, fuzzy, onCancel, onFix }: FixDialogProps) {
  const { t } = useI18n();
  const [issues, setIssues] = useState(issuesFirst);
  // Without a checks filter the strings were chosen to be read again as a whole.
  const [proofread, setProofread] = useState(!issuesFirst);
  const [adapt, setAdapt] = useState(fuzzy > 0);
  // The last request stays while the window lives: the same correction is often asked of several results.
  const [request, setRequest] = useStickyState("search.fixRequest", "");

  useEffect(() => {
    if (count !== null) {
      setIssues(issuesFirst);
      setProofread(!issuesFirst);
      setAdapt(fuzzy > 0);
    }
  }, [count, issuesFirst, fuzzy]);

  const ready = issues || proofread || (adapt && fuzzy > 0) || request.trim() !== "";
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
              if (ready) onFix(issues, proofread, adapt && fuzzy > 0, request.trim());
            }}
          >
            {fuzzy > 0 ? (
              <label className="checkbox">
                <input type="checkbox" checked={adapt} onChange={(event) => setAdapt(event.target.checked)} />
                {t("fix.adapt", { count: fuzzy })}
              </label>
            ) : null}
            <label className="checkbox">
              <input type="checkbox" checked={proofread} onChange={(event) => setProofread(event.target.checked)} />
              {t("fix.proofread")}
            </label>
            <label className="checkbox">
              <input type="checkbox" checked={issues} onChange={(event) => setIssues(event.target.checked)} />
              {issue === null ? t("fix.issues") : t("fix.issuesOf", { issue })}
            </label>
            <label className="field">
              <span className="field-label">{t("fix.request")}</span>
              <textarea
                className="input fix-request"
                rows={3}
                value={request}
                onChange={(event) => setRequest(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && ready) {
                    event.preventDefault();
                    onFix(issues, proofread, adapt && fuzzy > 0, request.trim());
                  }
                }}
              />
            </label>
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
