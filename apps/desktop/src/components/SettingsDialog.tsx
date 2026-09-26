import { memo, useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { appInfo } from "../ipc";
import { AiProvidersSettings } from "./AiProvidersSettings";
import { GameSettings, SourcePackages } from "./GameSettings";
import { keyboardShortcuts, shortcutGroupLabels } from "../shortcuts";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { BranchesSetting, IdentitySetting, MainBranchSetting, MergeDriverSetting, OtherFilesSetting, RemotesSetting, UpstreamSetting } from "./RepositorySettings";
import { UpdateChannelSetting, UpdatesSetting } from "./UpdateSettings";
import { editorFontSizes, interfaceZoomOptions, usePreferences } from "../ui/preferences";
import { useTheme } from "../ui/theme/theme";
import { themeRegistry, type ThemeDefinition } from "../ui/theme/registry";
import { hasWindowsBackdrop } from "../ui/theme/windowBackdrop";
import { useI18n } from "../ui/i18n";
import { languagePreferences, localeNames, type MessageKey } from "../i18n/translate";

export const accentPresets: readonly { value: string; label: MessageKey }[] = [
  { value: "#5ec4bd", label: "settings.accent.teal" },
  { value: "#61afef", label: "settings.accent.blue" },
  { value: "#b69cf6", label: "settings.accent.violet" },
  { value: "#f28fb0", label: "settings.accent.rose" },
  { value: "#e8b46b", label: "settings.accent.amber" },
  { value: "#98c379", label: "settings.accent.green" },
];

export type SettingsSection = "appearance" | "editor" | "workflow" | "game" | "ai" | "repository" | "keyboard" | "about";

const sections: ReadonlyArray<{ id: SettingsSection; label: MessageKey; icon: UiIconName }> = [
  { id: "appearance", label: "settings.section.appearance", icon: "palette" },
  { id: "editor", label: "settings.section.editor", icon: "languages" },
  { id: "workflow", label: "settings.section.workflow", icon: "arrowRight" },
  { id: "game", label: "settings.section.game", icon: "gamepad" },
  { id: "ai", label: "settings.section.ai", icon: "sparkles" },
  { id: "repository", label: "settings.section.repository", icon: "gitBranch" },
  { id: "keyboard", label: "settings.section.keyboard", icon: "listFilter" },
  { id: "about", label: "settings.section.about", icon: "info" },
];

type SettingsDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialSection?: SettingsSection;
  /** Repository settings need an open project. */
  projectOpen?: boolean;
};

type SettingEntry = {
  id: string;
  section: SettingsSection;
  title: string;
  description?: string;
  keywords?: string;
  /** Full-width entries render their control below the title. */
  wide?: boolean;
  control: ReactNode;
};

function ThemeCard({ theme, selected, onSelect }: { theme: ThemeDefinition; selected: boolean; onSelect: () => void }) {
  const { t } = useI18n();
  const { tokens } = theme;
  const family = theme.family ?? t("common.themesFamily");
  const preview = {
    "--preview-crust": tokens.crust,
    "--preview-base": tokens.base,
    "--preview-line": tokens.surface1,
    "--preview-text": tokens.text,
    "--preview-accent": tokens.accent,
    "--preview-macro": tokens.macro ?? tokens.accent,
  } as CSSProperties;
  return (
    <button className={selected ? "theme-card selected" : "theme-card"} type="button" role="radio" aria-checked={selected} onClick={onSelect} title={t("common.themeWithFamily", { name: theme.displayName, family })}>
      <span className="theme-card-preview" style={preview} aria-hidden="true">
        <span className="theme-card-rail" />
        <span className="theme-card-panel">
          <span className="theme-card-line strong" />
          <span className="theme-card-line"><span className="theme-card-macro" /></span>
          <span className="theme-card-line short" />
          <span className="theme-card-accent" />
        </span>
      </span>
      <span className="theme-card-label">
        <strong>{theme.displayName}</strong>
        <small>{family}</small>
      </span>
      {selected ? <span className="theme-card-check"><UiIcon icon="check" size="xs" /></span> : null}
    </button>
  );
}

function ThemePicker() {
  const { t } = useI18n();
  const { theme, setThemeId } = useTheme();
  const [filter, setFilter] = useState<"all" | "dark" | "light">("all");
  const [query, setQuery] = useState("");
  const normalized = query.trim().toLocaleLowerCase();
  const visible = themeRegistry.filter((entry) =>
    (filter === "all" || (filter === "light" ? entry.appearance === "light" : entry.appearance !== "light"))
    && (!normalized || `${entry.displayName} ${entry.family ?? ""}`.toLocaleLowerCase().includes(normalized)),
  );
  return (
    <div className="theme-picker">
      <div className="theme-picker-bar">
        <label className="search-field">
          <UiIcon icon="search" size="sm" />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("settings.theme.filterPlaceholder")} aria-label={t("settings.theme.filterPlaceholder")} spellCheck={false} />
        </label>
        <Segmented label={t("settings.theme.appearanceLabel")} value={filter} onChange={setFilter} options={[{ value: "all", label: t("settings.theme.all") }, { value: "dark", label: t("settings.theme.dark") }, { value: "light", label: t("settings.theme.light") }]} />
      </div>
      <div className="theme-grid" role="radiogroup" aria-label={t("settings.theme.gridLabel")}>
        {visible.map((entry) => <ThemeCard key={entry.id} theme={entry} selected={entry.id === theme.id} onSelect={() => setThemeId(entry.id)} />)}
        {visible.length === 0 ? <p className="muted">{t("settings.theme.noMatch")}</p> : null}
      </div>
    </div>
  );
}

/** Memoized so the closed dialog does not re-render with the workbench. */
export const SettingsDialog = memo(function SettingsDialog({ open, onOpenChange, initialSection = "appearance", projectOpen = false }: SettingsDialogProps) {
  const { theme, accentOverride, setAccentOverride, reduceTransparency, setReduceTransparency } = useTheme();
  const { preferences, setPreference, resetPreferences } = usePreferences();
  const { t } = useI18n();
  const [section, setSection] = useState<SettingsSection>(initialSection);
  const [query, setQuery] = useState("");
  const [version, setVersion] = useState<string | null>(null);
  const transparencyAvailable = hasWindowsBackdrop();

  useEffect(() => {
    if (!open) return;
    setSection(initialSection);
    setQuery("");
    void appInfo().then((info) => setVersion(info.version)).catch(() => undefined);
  }, [initialSection, open]);

  const entries: SettingEntry[] = [
    {
      id: "language", section: "appearance", title: t("settings.language.title"), description: t("settings.language.description"), keywords: t("settings.language.keywords"),
      control: <Segmented label={t("settings.language.title")} value={preferences.language} onChange={(value) => setPreference("language", value)} options={languagePreferences.map((language) => ({ value: language, label: language === "system" ? t("settings.language.system") : localeNames[language] }))} />,
    },
    { id: "theme", section: "appearance", title: t("settings.theme.title"), keywords: t("settings.theme.keywords"), wide: true, control: <ThemePicker /> },
    {
      id: "accent", section: "appearance", title: t("settings.accent.title"), description: t("settings.accent.description"), keywords: t("settings.accent.keywords"),
      control: (
        <div className="accent-row" role="radiogroup" aria-label={t("settings.accent.title")}>
          <button className={accentOverride === null ? "accent-swatch default selected" : "accent-swatch default"} type="button" role="radio" aria-checked={accentOverride === null} onClick={() => setAccentOverride(null)} title={t("settings.accent.themeDefault")}>
            <span style={{ background: theme.tokens.accent }} />{t("settings.accent.theme")}
          </button>
          {accentPresets.map((preset) => (
            <button className={accentOverride === preset.value ? "accent-swatch selected" : "accent-swatch"} type="button" role="radio" aria-checked={accentOverride === preset.value} aria-label={t(preset.label)} title={t(preset.label)} key={preset.value} onClick={() => setAccentOverride(preset.value)}>
              <span style={{ background: preset.value }} />
            </button>
          ))}
        </div>
      ),
    },
    {
      id: "zoom", section: "appearance", title: t("settings.zoom.title"), description: t("settings.zoom.description"), keywords: t("settings.zoom.keywords"),
      control: <Segmented label={t("settings.zoom.title")} value={String(preferences.interfaceZoom)} onChange={(value) => setPreference("interfaceZoom", Number(value))} options={interfaceZoomOptions.map((zoom) => ({ value: String(zoom), label: `${Math.round(zoom * 100)}%` }))} />,
    },
    {
      id: "transparency", section: "appearance", title: t("settings.transparency.title"), description: t(transparencyAvailable ? "settings.transparency.description" : "settings.transparency.unavailable"), keywords: t("settings.transparency.keywords"),
      control: <input type="checkbox" className="switch" aria-label={t("settings.transparency.title")} checked={reduceTransparency} disabled={!transparencyAvailable} onChange={(event) => setReduceTransparency(event.target.checked)} />,
    },
    {
      id: "font-size", section: "editor", title: t("settings.fontSize.title"), description: t("settings.fontSize.description"), keywords: t("settings.fontSize.keywords"),
      control: <Segmented label={t("settings.fontSize.label")} value={String(preferences.editorFontSize)} onChange={(value) => setPreference("editorFontSize", Number(value))} options={editorFontSizes.map((size) => ({ value: String(size), label: String(size) }))} />,
    },
    {
      id: "macros", section: "editor", title: t("settings.macros.title"), description: t("settings.macros.description"), keywords: t("settings.macros.keywords"),
      control: <input type="checkbox" className="switch" aria-label={t("settings.macros.title")} checked={preferences.highlightMacros} onChange={(event) => setPreference("highlightMacros", event.target.checked)} />,
    },
    {
      id: "control-chars", section: "editor", title: t("settings.controlChars.title"), description: t("settings.controlChars.description"), keywords: t("settings.controlChars.keywords"),
      control: <input type="checkbox" className="switch" aria-label={t("settings.controlChars.title")} checked={preferences.showControlCharacters} onChange={(event) => setPreference("showControlCharacters", event.target.checked)} />,
    },
    {
      id: "density", section: "editor", title: t("settings.density.title"), description: t("settings.density.description"), keywords: t("settings.density.keywords"),
      control: <Segmented label={t("settings.density.title")} value={preferences.listDensity} onChange={(value) => setPreference("listDensity", value)} options={[{ value: "compact", label: t("settings.density.compact") }, { value: "comfortable", label: t("settings.density.comfortable") }]} />,
    },
    {
      id: "focus-next", section: "workflow", title: t("settings.focusNext.title"), description: t("settings.focusNext.description"), keywords: t("settings.focusNext.keywords"),
      control: <input type="checkbox" className="switch" aria-label={t("settings.focusNext.title")} checked={preferences.focusTargetOnNext} onChange={(event) => setPreference("focusTargetOnNext", event.target.checked)} />,
    },
    {
      id: "game-installation", section: "game", title: t("settings.game.title"), description: t("settings.game.description"), keywords: t("settings.game.keywords"), wide: true,
      control: <GameSettings />,
    },
    {
      id: "source-packages", section: "game", title: t("settings.sources.title"), description: t("settings.sources.description"), keywords: t("settings.sources.keywords"), wide: true,
      control: <SourcePackages />,
    },
    {
      id: "ai-providers", section: "ai", title: t("settings.ai.title"), description: t("settings.ai.description"), keywords: t("settings.ai.keywords"), wide: true,
      control: <AiProvidersSettings />,
    },
    {
      id: "shortcuts", section: "keyboard", title: t("settings.shortcuts.title"), keywords: keyboardShortcuts.map((shortcut) => `${shortcut.keys} ${t(shortcut.action)}`).join(" "), wide: true,
      control: (
        <table className="shortcut-table">
          <tbody>
            {keyboardShortcuts.filter((shortcut) => !query.trim() || `${shortcut.keys} ${t(shortcut.action)}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())).map((shortcut) => (
              <tr key={shortcut.keys + shortcut.action}><td>{t(shortcut.action)}</td><td className="muted">{t(shortcutGroupLabels[shortcut.group])}</td><td><kbd>{shortcut.keys}</kbd></td></tr>
            ))}
          </tbody>
        </table>
      ),
    },
    {
      id: "about", section: "about", title: "Aeria", description: version ? t("launcher.version", { version }) : t("settings.about.description"), keywords: t("settings.about.keywords"),
      control: null,
    },
    {
      id: "updates", section: "about", title: t("settings.updates.title"), keywords: t("settings.updates.keywords"), wide: true,
      control: <UpdatesSetting />,
    },
    {
      id: "update-channel", section: "about", title: t("settings.channel.title"), description: t("settings.channel.description"), keywords: t("settings.channel.keywords"),
      control: <UpdateChannelSetting />,
    },
    ...(projectOpen ? [
      { id: "remotes", section: "repository" as const, title: t("repository.remotes"), description: t("repository.remotesHint"), keywords: t("repository.keywords"), wide: true, control: <RemotesSetting /> },
      { id: "upstream", section: "repository" as const, title: t("repository.upstream"), description: t("repository.upstreamHint"), keywords: t("repository.keywords"), wide: true, control: <UpstreamSetting /> },
      { id: "main-branch", section: "repository" as const, title: t("repository.mainBranch"), description: t("repository.mainBranchHint"), keywords: t("repository.keywords"), wide: true, control: <MainBranchSetting /> },
      { id: "branches", section: "repository" as const, title: t("repository.branches"), description: t("repository.branchesHint"), keywords: t("repository.keywords"), wide: true, control: <BranchesSetting /> },
      { id: "identity", section: "repository" as const, title: t("git.identity"), description: t("repository.identityHint"), keywords: t("repository.keywords"), wide: true, control: <IdentitySetting /> },
      { id: "merge-driver", section: "repository" as const, title: t("repository.mergeDriver"), description: t("repository.mergeDriverHint"), keywords: t("repository.keywords"), wide: true, control: <MergeDriverSetting /> },
      { id: "other-files", section: "repository" as const, title: t("repository.otherFiles"), description: t("repository.otherFilesHint"), keywords: t("repository.keywords"), wide: true, control: <OtherFilesSetting /> },
    ] : [
      { id: "repository-closed", section: "repository" as const, title: t("repository.noProject"), description: t("repository.noProjectHint"), control: null },
    ]),
    {
      id: "storage", section: "about", title: t("settings.storage.title"), description: t("settings.storage.description"), keywords: t("settings.storage.keywords"),
      control: null,
    },
    {
      id: "reset", section: "about", title: t("settings.reset.title"), description: t("settings.reset.description"), keywords: t("settings.reset.keywords"),
      control: <button className="button button-secondary" type="button" onClick={resetPreferences}>{t("settings.reset.button")}</button>,
    },
  ];

  const normalized = query.trim().toLocaleLowerCase();
  const visibleEntries = normalized
    ? entries.filter((entry) => `${entry.title} ${entry.description ?? ""} ${entry.keywords ?? ""}`.toLocaleLowerCase().includes(normalized))
    : entries.filter((entry) => entry.section === section);

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog settings-dialog" aria-describedby={undefined}>
          <aside className="settings-nav">
            <Dialog.Title className="dialog-title">{t("common.settings")}</Dialog.Title>
            <label className="search-field">
              <UiIcon icon="search" size="sm" />
              <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("settings.searchPlaceholder")} aria-label={t("settings.searchPlaceholder")} spellCheck={false} />
            </label>
            <nav aria-label={t("settings.sectionsLabel")}>
              {sections.map((entry) => (
                <button key={entry.id} type="button" className={!normalized && section === entry.id ? "settings-nav-item active" : "settings-nav-item"} onClick={() => { setQuery(""); setSection(entry.id); }}>
                  <UiIcon icon={entry.icon} size="sm" />{t(entry.label)}
                </button>
              ))}
            </nav>
          </aside>
          <section className="settings-content">
            <header className="settings-content-head">
              <h3>{normalized ? t("settings.resultsFor", { query: query.trim() }) : t(sections.find((entry) => entry.id === section)!.label)}</h3>
              <Dialog.Close className="icon-button icon-button-ghost" aria-label={t("settings.closeLabel")}><UiIcon icon="x" size="sm" /></Dialog.Close>
            </header>
            <div className="settings-list">
              {visibleEntries.map((entry) => (
                <div className={entry.wide ? "setting setting-wide" : "setting"} key={entry.id}>
                  <div className="setting-text">
                    <strong>{entry.title}</strong>
                    {normalized ? <small className="setting-section">{t(sections.find((candidate) => candidate.id === entry.section)!.label)}</small> : null}
                    {entry.description ? <p>{entry.description}</p> : null}
                  </div>
                  {entry.control ? <div className="setting-control">{entry.control}</div> : null}
                </div>
              ))}
              {visibleEntries.length === 0 ? <div className="empty-state"><UiIcon icon="search" size="xl" /><strong>{t("settings.noResults")}</strong></div> : null}
            </div>
          </section>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
});
