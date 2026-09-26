import { useEffect, useState } from "react";
import { open as openNativeDialog } from "@tauri-apps/plugin-dialog";
import { gameSettings, normalizeCommandError, setGamePath } from "../ipc";
import type { CommandError, GameInstallationDto, GameOrigin, GameSettingsDto } from "../types";
import { formatRelativeTime } from "../timeDisplay";
import { ConfirmDialog } from "./ConfirmDialog";
import { ErrorBanner } from "./ErrorBanner";
import { displayPath } from "../pathDisplay";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n, type Translate } from "../ui/i18n";
import type { MessageKey } from "../i18n/translate";

const originLabels: Readonly<Record<GameOrigin, MessageKey>> = {
  settings: "game.origin.settings",
  squareEnix: "game.origin.squareEnix",
  steam: "game.origin.steam",
  xivLauncher: "game.origin.xivLauncher",
  defaultLocation: "game.origin.defaultLocation",
};

/** "Version … · origin" for one installation. */
export function gameInstallationFacts(installation: GameInstallationDto, t: Translate): string {
  return t("game.versionOrigin", { version: installation.gameVersion, origin: t(originLabels[installation.origin]) });
}

export function GameSettings() {
  const { t } = useI18n();
  const [settings, setSettings] = useState<GameSettingsDto | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    gameSettings()
      .then((next) => { if (active) setSettings(next); })
      .catch((reason: unknown) => { if (active) setError(normalizeCommandError(reason)); });
    return () => { active = false; };
  }, []);

  async function apply(path: string | null) {
    setBusy(true);
    setError(null);
    try { setSettings(await setGamePath(path)); }
    catch (reason) { setError(normalizeCommandError(reason)); }
    finally { setBusy(false); }
  }

  async function choose() {
    try {
      const current = settings?.active?.path;
      const selection = await openNativeDialog(current ? { directory: true, multiple: false, defaultPath: current } : { directory: true, multiple: false });
      if (typeof selection === "string") await apply(selection);
    } catch (reason) {
      setError({ code: "pathPicker", message: reason instanceof Error ? reason.message : t("launcher.pathPickerFailed") });
    }
  }

  const active = settings?.active ?? null;
  return (
    <div className="game-settings">
      {error ? <ErrorBanner title={t("settings.game.title")} error={error} onDismiss={() => setError(null)} /> : null}
      {settings === null ? (
        error ? null : <p className="muted">{t("game.loading")}</p>
      ) : (
        <>
          <div className={active ? "game-active" : "game-active missing"}>
            <UiIcon icon={active ? "gamepad" : "triangleAlert"} size="md" />
            <div className="game-active-text">
              {active ? <>
                <strong className="mono">{active.path}</strong>
                <small>{gameInstallationFacts(active, t)}</small>
              </> : <>
                {settings.configuredPath ? <strong className="mono">{settings.configuredPath}</strong> : null}
                <small>{t(settings.configuredPath ? "game.invalid" : "game.missing")}</small>
              </>}
            </div>
          </div>
          <div className="game-actions">
            <button className="button button-secondary" type="button" disabled={busy} onClick={() => void choose()}>
              <UiIcon icon="folderOpen" size="sm" /> {t("game.choose")}
            </button>
            {settings.configuredPath ? (
              <button className="button button-ghost" type="button" disabled={busy} onClick={() => void apply(null)}>{t("game.useDetection")}</button>
            ) : null}
          </div>
          {settings.detected.length > 0 ? (
            <div className="game-detected">
              <span className="field-label">{t("game.detectedTitle")}</span>
              <ul>
                {settings.detected.map((installation) => {
                  const inUse = active?.path === installation.path;
                  return (
                    <li key={installation.path}>
                      <span className="game-active-text">
                        <span className="mono">{installation.path}</span>
                        <small>{gameInstallationFacts(installation, t)}</small>
                      </span>
                      {inUse
                        ? <span className="chip">{t("game.inUse")}</span>
                        : <button className="button button-secondary" type="button" disabled={busy} onClick={() => void apply(installation.path)}>{t("game.use")}</button>}
                    </li>
                  );
                })}
              </ul>
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}
