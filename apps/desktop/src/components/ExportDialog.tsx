import { memo, useCallback, useEffect, useState, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { open as openNativeDialog, save as saveNativeDialog } from "@tauri-apps/plugin-dialog";
import {
  exportBackupKey,
  exportGenerateKey,
  exportGitAuthors,
  exportImportKey,
  exportInstallWorkflow,
  exportOverview,
  exportPack,
  exportPublish,
  exportRemoveKey,
  exportSaveSettings,
  normalizeCommandError,
} from "../ipc";
import type { CommandError, ExportOverviewDto, ExportReportDto, PackSettings, ReleaseChannel } from "../types";
import { useI18n } from "../ui/i18n";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { ConfirmDialog } from "./ConfirmDialog";
import { ErrorBanner } from "./ErrorBanner";
import { FontSettingsSection } from "./FontSettingsSection";

type ExportDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Closes the dialog and shows uncommitted changes in the Git panel. */
  onOpenChanges?: (() => void) | undefined;
};

type Section = "release" | "pack" | "fonts" | "key" | "github";

type Result = { report: ExportReportDto; version: string; path?: string; releaseUrl?: string; feedUrl?: string; published?: boolean };

type Release = {
  channel: ReleaseChannel;
  changelog: string;
};

const DEFAULT_MIN_HARMONIA = "0.1.2.0";
const emptySettings: PackSettings = { title: "", teamName: "", teamUrl: null, authors: [], license: null, minHarmonia: DEFAULT_MIN_HARMONIA };

/**
 * The version after `version` (`YYYY.MM.DD.NNNN`) on the same day. Versions
 * have fixed widths, so text order is version order.
 */
export function versionAfter(version: string): string {
  const match = /^(\d{4}\.\d{2}\.\d{2})\.(\d{4})$/.exec(version);
  if (!match) return version;
  return `${match[1]}.${String(Number(match[2]) + 1).padStart(4, "0")}`;
}

/** Authors as the settings keep them: trimmed, without blanks or repeats. */
function cleanAuthors(authors: string[]): string[] {
  return [...new Set(authors.map((author) => author.trim()).filter((author) => author !== ""))];
}

function shortFingerprint(fingerprint: string): string {
  return `${fingerprint.slice(0, 16)}…`;
}

function settingsFrom(overview: ExportOverviewDto): PackSettings {
  if (!overview.settings) return { ...emptySettings, teamName: overview.github?.owner ?? "" };
  const { signingKeyFingerprint: _fingerprint, ...settings } = overview.settings;
  return settings;
}

function sameSettings(left: PackSettings, right: PackSettings): boolean {
  return (Object.keys(left) as (keyof PackSettings)[]).every((key) => key === "authors"
    ? cleanAuthors(left.authors).join("\n") === cleanAuthors(right.authors).join("\n")
    : (left[key] ?? "") === (right[key] ?? ""));
}

/** One line of the release readiness list. */
function Check({ state, children, action }: { state: "ok" | "warn" | "info"; children: ReactNode; action?: ReactNode }) {
  const icon: UiIconName = state === "ok" ? "circleCheck" : state === "warn" ? "circleAlert" : "info";
  return (
    <li className={`export-check export-check-${state}`}>
      <UiIcon icon={icon} size="sm" />
      <span>{children}</span>
      {action}
    </li>
  );
}

/** Builds, signs, and publishes the project's Harmonia pack. */
export const ExportDialog = memo(function ExportDialog({ open, onOpenChange, onOpenChanges }: ExportDialogProps) {
  const { t } = useI18n();
  const [overview, setOverview] = useState<ExportOverviewDto | null>(null);
  const [section, setSection] = useState<Section>("release");
  const [settings, setSettings] = useState<PackSettings>(emptySettings);
  const [release, setRelease] = useState<Release>({ channel: "stable", changelog: "" });
  /** The last version published from this dialog: GitHub has its tag, the local repository not yet. */
  const [publishedHere, setPublishedHere] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [confirm, setConfirm] = useState<{ message: string; confirmLabel: string; run: () => void } | null>(null);

  const show = useCallback((next: ExportOverviewDto) => {
    setOverview(next);
    setSettings(settingsFrom(next));
  }, []);

  useEffect(() => {
    if (!open) return;
    setError(null);
    setResult(null);
    setPublishedHere("");
    void exportOverview()
      .then((next) => {
        show(next);
        setSection(next.settings ? "release" : "pack");
      })
      .catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, [open, show]);

  const run = async <T,>(label: string, operation: () => Promise<T>): Promise<T | undefined> => {
    setBusy(label);
    setError(null);
    try {
      return await operation();
    } catch (reason) {
      setError(normalizeCommandError(reason));
      return undefined;
    } finally {
      setBusy(null);
    }
  };

  const refresh = async (operation: () => Promise<ExportOverviewDto>, label: string) => {
    const next = await run(label, operation);
    if (next) show(next);
    return next;
  };

  const reloadOverview = () => void exportOverview().then(show).catch((reason: unknown) => setError(normalizeCommandError(reason)));

  const saved = overview ? settingsFrom(overview) : emptySettings;
  const firstSetup = overview !== null && overview.settings === null;
  const settingsDirty = overview !== null && (firstSetup || !sameSettings(settings, saved));
  const fingerprint = overview?.settings?.signingKeyFingerprint ?? null;
  const localKey = overview?.key.fingerprint ?? null;
  const keyMatches = localKey !== null && localKey === fingerprint;
  const project = overview?.project;
  const afterPublished = publishedHere ? versionAfter(publishedHere) : "";
  const nextVersion = overview ? (afterPublished > overview.nextVersion ? afterPublished : overview.nextVersion) : "";
  const committed = project !== undefined && project.commit !== null && !project.uncommitted;
  const pushed = committed && project.upstream !== null && project.ahead === 0;
  const packReady = overview?.settings != null && !settingsDirty;
  const canExport = packReady && committed && busy === null;
  const canPublish = canExport && overview?.github != null && keyMatches && pushed;
  const requiredFilled = settings.title.trim() !== "" && settings.teamName.trim() !== "";

  const releaseInput = () => ({
    channel: release.channel,
    changelog: release.changelog.trim() || null,
  });

  const addGitAuthors = async () => {
    const found = await run(t("common.loading"), exportGitAuthors);
    if (found) setSettings((current) => ({ ...current, authors: cleanAuthors([...current.authors, ...found]) }));
  };

  const partKeys = { translations: "export.check.part.translations", pack: "export.check.part.pack", fonts: "export.check.part.fonts" } as const;
  const saveSettings = async () => {
    const next = await refresh(() => exportSaveSettings({ ...settings, authors: cleanAuthors(settings.authors), minHarmonia: settings.minHarmonia.trim() || DEFAULT_MIN_HARMONIA }), t("common.saving"));
    if (next && firstSetup) setSection("release");
  };

  const pickKeyFile = async (mode: "open" | "save"): Promise<string | null> => {
    const filters = [{ name: t("export.key.backupFilter"), extensions: ["json"] }];
    const selection = mode === "open"
      ? await openNativeDialog({ multiple: false, directory: false, filters })
      : await saveNativeDialog({ filters, defaultPath: `${overview?.settings?.title ?? "pack"} signing key.json` });
    return typeof selection === "string" ? selection : null;
  };

  const generateKey = () => {
    if (fingerprint === null) { void refresh(() => exportGenerateKey(false), t("export.key.generating")); return; }
    setConfirm({ message: t("export.key.replaceConfirm"), confirmLabel: t("export.key.replace"), run: () => void refresh(() => exportGenerateKey(true), t("export.key.generating")) });
  };

  const importKey = async () => {
    const path = await pickKeyFile("open");
    if (path) void refresh(() => exportImportKey(path), t("export.key.importing"));
  };

  const backupKey = async () => {
    const path = await pickKeyFile("save");
    if (path) await run(t("common.saving"), () => exportBackupKey(path));
  };

  const removeKey = () => setConfirm({ message: t("export.key.removeConfirm"), confirmLabel: t("export.key.remove"), run: () => void refresh(exportRemoveKey, t("export.key.removing")) });

  const exportLocal = async () => {
    const directory = await openNativeDialog({ directory: true, multiple: false });
    if (typeof directory !== "string") return;
    const exported = await run(t("export.building"), () => exportPack(releaseInput(), directory, keyMatches));
    if (exported) setResult({ report: exported.report, version: exported.version, path: exported.path });
  };

  const publish = () => setConfirm({
    message: t("export.publish.confirm", { version: nextVersion, repository: `${overview?.github?.owner}/${overview?.github?.name}` }),
    confirmLabel: t("export.publish.action"),
    run: () => void run(t("export.publishing"), () => exportPublish(releaseInput())).then((done) => {
      if (!done) return;
      setResult({ report: done.report, version: done.version, releaseUrl: done.releaseUrl, feedUrl: done.feedUrl, published: true });
      setPublishedHere(done.version);
    }),
  });

  const field = (key: Exclude<keyof PackSettings, "authors">, label: string, hint: string, options: { placeholder?: string; optional?: boolean; onChange?: (value: string) => void } = {}) => (
    <label className="field">
      <span className="field-label">{label}{options.optional ? <span className="export-optional">{t("export.optional")}</span> : null}</span>
      <input
        className="input"
        value={settings[key] ?? ""}
        placeholder={options.placeholder}
        spellCheck={false}
        disabled={busy !== null}
        onChange={(event) => {
          const value = event.target.value;
          if (options.onChange) { options.onChange(value); return; }
          setSettings((current) => ({ ...current, [key]: options.optional && value === "" ? null : value }));
        }}
      />
      <span className="field-hint">{hint}</span>
    </label>
  );

  const navItems: { id: Section; icon: UiIconName; label: string; attention: boolean; hidden?: boolean }[] = [
    { id: "release", icon: "cloud", label: t("export.nav.release"), attention: false },
    { id: "pack", icon: "info", label: t("export.nav.pack"), attention: !packReady },
    { id: "fonts", icon: "languages", label: t("export.nav.fonts"), attention: false },
    { id: "key", icon: "user", label: t("export.nav.key"), attention: overview?.settings != null && !keyMatches },
    { id: "github", icon: "gitBranch", label: t("export.nav.github"), attention: overview?.github != null && overview.workflow !== "current", hidden: !overview?.github },
  ];
  const title = navItems.find((item) => item.id === section)?.label ?? "";
  const goto = (target: Section) => <button className="button button-ghost export-check-action" type="button" onClick={() => setSection(target)}>{t("export.check.open")}</button>;

  const releaseSection = overview && project ? (
    <>
      <p className="export-facts">
        <span>{t("export.project.languages", { source: project.sourceLanguage, target: project.targetLanguage })}</span>
        <span>{t("export.project.gameVersion", { version: project.gameVersion })}</span>
        {project.commit ? <span>{t("export.project.commit", { commit: project.commit.slice(0, 10) })}</span> : null}
      </p>

      <ul className="export-checks">
        {packReady
          ? <Check state="ok">{t("export.check.pack", { title: overview.settings!.title })}</Check>
          : <Check state="warn" action={goto("pack")}>{t("export.check.packMissing")}</Check>}
        {project.commit === null ? <Check state="warn">{t("export.check.noCommit")}</Check>
          : project.uncommitted ? (
            <Check state="warn" action={onOpenChanges ? <button className="button button-ghost export-check-action" type="button" onClick={onOpenChanges}>{t("export.check.openChanges")}</button> : undefined}>
              {t("export.check.uncommitted", { parts: project.uncommittedParts.map((part) => t(partKeys[part])).join(", ") })}
            </Check>
          )
          : <Check state="ok">{t("export.check.committed")}</Check>}
        {keyMatches
          ? <Check state="ok">{t("export.check.signed")}</Check>
          : <Check state="info" action={overview.settings ? goto("key") : undefined}>{t("export.check.unsigned")}</Check>}
        <Check state="info" action={goto("fonts")}>{t(overview.fontsConfigured ? "export.check.fonts" : "export.check.noFonts")}</Check>
        {overview.github && !overview.workflowOnGithub ? (
          <Check state="warn" action={goto("github")}>{t("export.check.workflowMissing", { branch: overview.mainBranch ?? "main" })}</Check>
        ) : null}
        {overview.github ? (pushed
          ? <Check state="ok">{t("export.check.pushed", { repository: `${overview.github.owner}/${overview.github.name}` })}</Check>
          : <Check state="info">{t("export.check.notPushed")}</Check>) : null}
      </ul>

      <p className="field-hint">{t("export.release.contentHint")}</p>

      <div className="export-block">
        <span className="field-label">{t("export.release.version", { version: nextVersion })}</span>
        <span className="field-hint">{t(overview.github ? "export.release.versionHintGitHub" : "export.release.versionHint")}</span>
      </div>

      <label className="field">
        <span className="field-label">{t("export.release.changelog")}<span className="export-optional">{t("export.optional")}</span></span>
        <textarea className="input export-changelog" value={release.changelog} placeholder={t("export.release.changelogPlaceholder")} disabled={busy !== null} onChange={(event) => setRelease((current) => ({ ...current, changelog: event.target.value }))} />
      </label>

      {overview.github ? (
        <div className="export-block">
          <span className="field-label">{t("export.release.channel")}</span>
          <Segmented<ReleaseChannel> label={t("export.release.channel")} value={release.channel} disabled={busy !== null} onChange={(channel) => setRelease((current) => ({ ...current, channel }))} options={[{ value: "stable", label: t("export.release.stable") }, { value: "testing", label: t("export.release.testing") }]} />
          <span className="field-hint">{t("export.release.channelHint")}</span>
        </div>
      ) : null}

      {result ? (
        <div className="export-result" aria-live="polite">
          <h4 className="export-heading"><UiIcon icon="circleCheck" size="sm" />{result.published ? t("export.result.published", { version: result.version }) : t("export.result.exported", { version: result.version })}</h4>
          <p className="export-facts">
            <span>{t("export.result.counts", { exported: result.report.exported, sheets: result.report.sheets })}</span>
            {result.report.fontTargets > 0 ? <span>{t("export.result.fonts", { glyphs: result.report.fontGlyphs, sizes: result.report.fontTargets })}</span> : null}
            <span>{result.report.signedBy ? t("export.result.signed", { fingerprint: shortFingerprint(result.report.signedBy) }) : t("export.result.unsigned")}</span>
          </p>
          {result.report.skippedFuzzy > 0 ? (
            <p className="field-hint">{t("export.result.skippedFuzzy", { fuzzy: result.report.skippedFuzzy })}</p>
          ) : null}
          {result.path ? <code className="export-selectable">{result.path}</code> : null}
          {result.releaseUrl ? <code className="export-selectable">{result.releaseUrl}</code> : null}
          {result.feedUrl && !overview.workflowOnGithub ? <p className="export-warning"><UiIcon icon="circleAlert" size="xs" />{t("export.result.noWorkflow")}</p> : null}
        </div>
      ) : null}
    </>
  ) : null;

  const packSection = overview ? (
    <>
      <p className="field-hint">{t(firstSetup ? "export.pack.welcome" : "export.pack.intro")}</p>
      {overview.settingsError ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{overview.settingsError}</p> : null}
      {field("title", t("export.pack.packTitle"), t("export.pack.packTitleHint"), { placeholder: t("export.pack.packTitlePlaceholder") })}
      {field("teamName", t("export.pack.team"), t("export.pack.teamHint"), { placeholder: t("export.pack.teamPlaceholder") })}
      <label className="field">
        <span className="field-label">{t("export.pack.authors")}<span className="export-optional">{t("export.optional")}</span></span>
        <textarea
          className="input export-changelog"
          value={settings.authors.join("\n")}
          placeholder={t("export.pack.authorsPlaceholder")}
          spellCheck={false}
          disabled={busy !== null}
          onChange={(event) => {
            const authors = event.target.value.split("\n");
            setSettings((current) => ({ ...current, authors }));
          }}
        />
        <span className="field-hint">{t("export.pack.authorsHint")}</span>
      </label>
      <div className="dialog-actions export-actions-start">
        <button className="button button-ghost" type="button" disabled={busy !== null} onClick={() => void addGitAuthors()}>{t("export.pack.authorsFromGit")}</button>
      </div>
      <details className="export-details">
        <summary>{t("export.pack.more")}</summary>
        <div className="export-details-body">
          {field("teamUrl", t("export.pack.teamUrl"), t("export.pack.teamUrlHint"), { placeholder: "https://github.com/…", optional: true })}
          {field("license", t("export.pack.license"), t("export.pack.licenseHint"), { placeholder: "CC-BY-NC-SA-4.0", optional: true })}
          {field("minHarmonia", t("export.pack.minHarmonia"), t("export.pack.minHarmoniaHint"), { placeholder: DEFAULT_MIN_HARMONIA })}
        </div>
      </details>
      <div className="dialog-actions export-actions-start">
        <button className="button button-primary" type="button" disabled={busy !== null || !requiredFilled || !settingsDirty} onClick={() => void saveSettings()}>
          {t(firstSetup ? "export.pack.create" : "export.pack.save")}
        </button>
        {settingsDirty && !firstSetup ? <button className="button button-ghost" type="button" disabled={busy !== null} onClick={() => setSettings(saved)}>{t("guide.revert")}</button> : null}
      </div>
    </>
  ) : null;

  const keySection = overview ? (overview.settings === null ? <p className="field-hint">{t("export.key.needsSettings")}</p> : (
    <>
      <p className="field-hint">{t("export.key.hint")}</p>
      <dl className="export-key">
        <dt>{t("export.key.project")}</dt>
        <dd>{fingerprint ? <code title={fingerprint}>{shortFingerprint(fingerprint)}</code> : t("export.key.none")}</dd>
        <dt>{t("export.key.local")}</dt>
        <dd>
          {overview.key.state === "unavailable" ? t("export.key.unavailable")
            : localKey ? <><code title={localKey}>{shortFingerprint(localKey)}</code> {keyMatches ? <UiIcon icon="circleCheck" size="xs" /> : <span className="export-warning-inline">{t("export.key.mismatch")}</span>}</>
            : t("export.key.none")}
        </dd>
      </dl>
      <div className="dialog-actions export-actions-start">
        {localKey === null ? <button className="button button-secondary" type="button" disabled={busy !== null} onClick={generateKey}>{fingerprint ? t("export.key.replace") : t("export.key.generate")}</button> : null}
        <button className="button button-secondary" type="button" disabled={busy !== null} onClick={() => void importKey()}>{t("export.key.import")}</button>
        {localKey ? <button className="button button-secondary" type="button" disabled={busy !== null} onClick={() => void backupKey()}>{t("export.key.backup")}</button> : null}
        {localKey ? <button className="button button-ghost" type="button" disabled={busy !== null} onClick={removeKey}>{t("export.key.remove")}</button> : null}
      </div>
    </>
  )) : null;

  const githubSection = overview?.github ? (
    <>
      <p className="field-hint">{t("export.feed.hint", { repository: `${overview.github.owner}/${overview.github.name}` })}</p>
      <p className="export-facts"><span>{t("export.feed.url")}</span><code className="export-selectable">{overview.github.feedUrl}</code></p>
      {overview.workflow === "current" && !overview.workflowOnGithub ? (
        <p className="export-warning"><UiIcon icon="circleAlert" size="xs" />{t("export.feed.notOnGithub", { branch: overview.mainBranch ?? "main" })}</p>
      ) : null}
      <div className="dialog-actions export-actions-start">
        <span className={overview.workflow === "current" ? "muted" : "export-warning-inline"}>{t(overview.workflow === "current" ? "export.feed.workflowCurrent" : overview.workflow === "different" ? "export.feed.workflowDifferent" : "export.feed.workflowMissing")}</span>
        {overview.workflow !== "current" ? <button className="button button-secondary" type="button" disabled={busy !== null} onClick={() => void refresh(exportInstallWorkflow, t("common.saving"))}>{t(overview.workflow === "missing" ? "export.feed.install" : "export.feed.update")}</button> : null}
      </div>
    </>
  ) : null;

  return (
    <Dialog.Root open={open} onOpenChange={(next) => { if (busy === null) onOpenChange(next); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog settings-dialog export-dialog" aria-describedby={undefined}>
          <aside className="settings-nav">
            <Dialog.Title className="dialog-title">{t("export.title")}</Dialog.Title>
            <nav aria-label={t("export.title")}>
              {navItems.filter((item) => !item.hidden).map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className={section === item.id ? "settings-nav-item active" : "settings-nav-item"}
                  disabled={overview === null || (firstSetup && item.id !== "pack")}
                  onClick={() => setSection(item.id)}
                >
                  <UiIcon icon={item.icon} size="sm" />{item.label}
                  {item.attention ? <span className="export-nav-dot" aria-label={t("export.nav.attention")} /> : null}
                </button>
              ))}
            </nav>
          </aside>
          <section className="settings-content">
            <header className="settings-content-head">
              <h3>{title}</h3>
              <Dialog.Close className="icon-button icon-button-ghost" aria-label={t("settings.closeLabel")} disabled={busy !== null}><UiIcon icon="x" size="sm" /></Dialog.Close>
            </header>
            <div className="settings-list export-body">
              {error ? <ErrorBanner title={t("export.error")} error={error} onDismiss={() => setError(null)} /> : null}
              {overview === null ? (error ? null : <p className="muted">{t("common.loading")}</p>)
                : section === "release" ? releaseSection
                : section === "pack" ? packSection
                : section === "fonts" ? <FontSettingsSection disabled={busy !== null} onSaved={reloadOverview} />
                : section === "key" ? keySection
                : githubSection}
            </div>
            {overview && section === "release" ? (
              <footer className="dialog-actions export-footer">
                {busy ? <span className="muted">{busy}</span> : null}
                <button className="button button-secondary" type="button" disabled={!canExport} onClick={() => void exportLocal()} title={t("export.localHint")}><UiIcon icon="folderOpen" size="sm" />{t("export.local")}</button>
                {overview.github ? <button className="button button-primary" type="button" disabled={!canPublish} onClick={publish}><UiIcon icon="cloud" size="sm" />{t("export.publish.action")}</button> : null}
              </footer>
            ) : null}
          </section>

          <ConfirmDialog open={confirm !== null} message={confirm?.message ?? ""} confirmLabel={confirm?.confirmLabel ?? ""} cancelLabel={t("common.cancel")} onKeepEditing={() => setConfirm(null)} onDiscard={() => { const action = confirm?.run; setConfirm(null); action?.(); }} />
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
});
