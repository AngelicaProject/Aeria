import { useEffect, useState } from "react";
import { open as openNativeDialog } from "@tauri-apps/plugin-dialog";
import { deleteSourcePackage, gameSettings, listSourcePackages, normalizeCommandError, revealSourcePackages, setGamePath } from "../ipc";
import type { CommandError, GameInstallationDto, GameOrigin, GameSettingsDto, SourcePackageEntryDto } from "../types";
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

const languageLabels: Readonly<Record<string, MessageKey>> = {
  en: "language.en",
  ja: "language.ja",
  de: "language.de",
  fr: "language.fr",
};

/** The packages in Aeria's source-package store, with deletion of unused ones. */
export function SourcePackages() {
  const { t, locale } = useI18n();
  const [packages, setPackages] = useState<readonly SourcePackageEntryDto[] | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [pendingDelete, setPendingDelete] = useState<SourcePackageEntryDto | null>(null);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => {
    let active = true;
    listSourcePackages()
      .then((next) => { if (active) setPackages(next); })
      .catch((reason: unknown) => { if (active) setError(normalizeCommandError(reason)); });
    return () => { active = false; };
  }, []);

  async function confirmDelete() {
    const entry = pendingDelete;
    setPendingDelete(null);
    if (!entry) return;
    setDeleting(true);
    setError(null);
    try { setPackages(await deleteSourcePackage(entry.packageId)); }
    catch (reason) { setError(normalizeCommandError(reason)); }
    finally { setDeleting(false); }
  }

  const now = Date.now();
  const size = new Intl.NumberFormat(locale, { style: "unit", unit: "megabyte", maximumFractionDigits: 0 });
  const languageName = (entry: SourcePackageEntryDto) => {
    const label = languageLabels[entry.sourceLanguage];
    return label ? t(label) : entry.sourceLanguage.toUpperCase();
  };
  const sizeOf = (entry: SourcePackageEntryDto) => size.format(entry.sizeBytes / 1_000_000);
  return (
    <div className="game-settings">
      {error ? <ErrorBanner title={t("settings.sources.title")} error={error} onDismiss={() => setError(null)} /> : null}
      {packages === null ? (
        error ? null : <p className="muted">{t("sources.loading")}</p>
      ) : packages.length === 0 ? (
        <p className="field-hint">{t("sources.empty")}</p>
      ) : (
        <ul className="source-packages">
          {packages.map((entry) => (
            <li key={entry.path} title={entry.path}>
              <span className="game-active-text">
                <strong>{languageName(entry)}</strong>
                <small>{t("sources.facts", {
                  version: entry.gameVersion,
                  size: sizeOf(entry),
                  time: entry.builtAtUnixMs === null ? "—" : formatRelativeTime(entry.builtAtUnixMs, now, locale, t("time.justNow")),
                })}</small>
              </span>
              <span className="source-package-state">
                {entry.current ? <span className="chip chip-added" title={t("sources.currentHint")}>{t("sources.current")}</span> : null}
                {entry.usedBy.length > 0 ? (
                  <span className="chip" title={entry.usedBy.map(displayPath).join("\n")}>{t("sources.usedBy", { count: entry.usedBy.length })}</span>
                ) : null}
                {entry.removable ? (
                  <IconButton icon="trash" label={t("sources.deleteLabel", { version: entry.gameVersion })} disabled={deleting} onClick={() => setPendingDelete(entry)} />
                ) : null}
              </span>
            </li>
          ))}
        </ul>
      )}
      <div className="game-actions">
        <button className="button button-secondary" type="button" onClick={() => void revealSourcePackages().catch((reason: unknown) => setError(normalizeCommandError(reason)))}>
          <UiIcon icon="folderOpen" size="sm" /> {t("sources.reveal")}
        </button>
      </div>
      <ConfirmDialog
        open={pendingDelete !== null}
        title={t("sources.deleteTitle")}
        message={pendingDelete ? t("sources.deleteMessage", { language: languageName(pendingDelete), version: pendingDelete.gameVersion, size: sizeOf(pendingDelete) }) : ""}
        confirmLabel={t("sources.delete")}
        cancelLabel={t("common.cancel")}
        onKeepEditing={() => setPendingDelete(null)}
        onDiscard={() => void confirmDelete()}
      />
    </div>
  );
}

/** The one game installation this Aeria instance builds source packages from. */
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
