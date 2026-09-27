import { useState } from "react";
import { normalizeCommandError, setProjectTargetLanguage } from "../ipc";
import { UNDETERMINED_LANGUAGE } from "../targetLanguages";
import type { CommandError, ProjectSummaryDto } from "../types";
import { useI18n } from "../ui/i18n";
import { TargetLanguagePicker } from "./TargetLanguagePicker";

/**
 * The open project's target language. Changing it rewrites only the
 * project manifest, which collaborators receive through Git; translations
 * stay as they are.
 */
export function TargetLanguageSetting({ project, onProjectChanged }: { project: ProjectSummaryDto; onProjectChanged: (project: ProjectSummaryDto) => void }) {
  const { t } = useI18n();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CommandError | null>(null);
  const current = project.targetLanguage === UNDETERMINED_LANGUAGE ? null : project.targetLanguage;

  async function choose(tag: string | null) {
    if (tag === null || tag === current) return;
    setBusy(true);
    setError(null);
    try {
      const result = await setProjectTargetLanguage(tag);
      onProjectChanged(result.project);
    } catch (caught) {
      setError(normalizeCommandError(caught));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="setting-stack">
      <TargetLanguagePicker key={project.repositoryRoot} value={current} onChange={(tag) => void choose(tag)} disabled={busy} commitTyped />
      {current === null ? <p className="field-hint is-warning">{t("project.targetLanguageMissing")}</p> : null}
      {error ? <p className="field-hint is-error" role="alert">{error.message}</p> : null}
    </div>
  );
}
