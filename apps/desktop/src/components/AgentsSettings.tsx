import { useEffect, useState } from "react";
import { agentsConnect, agentsStatus, normalizeCommandError } from "../ipc";
import type { AgentsStatusDto, CommandError } from "../types";
import { ErrorBanner } from "./ErrorBanner";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

/** One line of the status list: done or not, with what it is. */
function StatusLine({ done, label, detail }: { done: boolean; label: string; detail?: string | null }) {
  return (
    <li className={done ? "agents-status-line done" : "agents-status-line"}>
      <UiIcon icon={done ? "circleCheck" : "circleAlert"} size="sm" />
      <span>{label}</span>
      {detail ? <small className="mono muted">{detail}</small> : null}
    </li>
  );
}

/**
 * Connects agent harnesses (Claude Code, Codex, Hermes Agent) to Aeria:
 * the `aeria` command on PATH, the skill, and AGENTS.md in the open project.
 */
export function AgentsSettings() {
  const { t } = useI18n();
  const [status, setStatus] = useState<AgentsStatusDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [busy, setBusy] = useState(false);
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    let active = true;
    agentsStatus()
      .then((next) => { if (active) setStatus(next); })
      .catch((reason: unknown) => { if (active) setError(normalizeCommandError(reason)); });
    return () => { active = false; };
  }, []);

  async function connect() {
    setBusy(true);
    setError(null);
    try {
      setStatus(await agentsConnect());
      setConnected(true);
    } catch (reason) {
      setError(normalizeCommandError(reason));
    } finally {
      setBusy(false);
    }
  }

  const complete = status !== null
    && status.commandCurrent
    && status.onPath
    && status.skills.every((skill) => skill.current)
    && status.projectFiles !== false;

  return (
    <div className="agents-settings">
      {error ? <ErrorBanner title={t("settings.agents.title")} error={error} onDismiss={() => setError(null)} /> : null}
      {status === null ? (
        error ? null : <p className="muted">{t("common.loading")}</p>
      ) : (
        <>
          <ul className="agents-status">
            <StatusLine done={status.commandCurrent && status.onPath} label={t("settings.agents.command")} detail={status.commandPath} />
            {status.skills.map((skill) => (
              <StatusLine key={skill.harness} done={skill.current} label={t("settings.agents.skill", { harness: skill.harness })} detail={skill.path} />
            ))}
            {status.skills.length === 0 ? <li className="agents-status-line muted">{t("settings.agents.noHarness")}</li> : null}
            {status.projectFiles === null
              ? <li className="agents-status-line muted">{t("settings.agents.noProject")}</li>
              : <StatusLine done={status.projectFiles} label={t("settings.agents.projectFiles")} />}
          </ul>
          {status.commandMissing ? <p className="field-hint">{t("settings.agents.commandMissing")}</p> : null}
          <div className="agents-actions">
            <button className="button button-primary" type="button" disabled={busy || status.commandMissing} onClick={() => void connect()}>
              <UiIcon icon="sparkles" size="sm" />
              {complete ? t("settings.agents.reconnect") : t("settings.agents.connect")}
            </button>
          </div>
          {connected ? <p className="field-hint">{t("settings.agents.connected")}</p> : null}
        </>
      )}
    </div>
  );
}
