import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { appInfo } from "../ipc";
import { keyboardShortcuts } from "../shortcuts";
import { Segmented } from "../ui/primitives/Segmented";
import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { editorFontSizes, interfaceZoomOptions, usePreferences } from "../ui/preferences";
import { useTheme } from "../ui/theme/theme";
import { themeRegistry, type ThemeDefinition } from "../ui/theme/registry";
import { hasWindowsBackdrop } from "../ui/theme/windowBackdrop";

export const accentPresets: readonly { value: string; label: string }[] = [
  { value: "#5ec4bd", label: "Teal" },
  { value: "#61afef", label: "Blue" },
  { value: "#b69cf6", label: "Violet" },
  { value: "#f28fb0", label: "Rose" },
  { value: "#e8b46b", label: "Amber" },
  { value: "#98c379", label: "Green" },
];

export type SettingsSection = "appearance" | "editor" | "workflow" | "keyboard" | "about";

const sections: ReadonlyArray<{ id: SettingsSection; label: string; icon: UiIconName }> = [
  { id: "appearance", label: "Appearance", icon: "palette" },
  { id: "editor", label: "Editor", icon: "languages" },
  { id: "workflow", label: "Workflow", icon: "arrowRight" },
  { id: "keyboard", label: "Keyboard shortcuts", icon: "listFilter" },
  { id: "about", label: "About", icon: "info" },
];

type SettingsDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialSection?: SettingsSection;
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
  const { tokens } = theme;
  const preview = {
    "--preview-crust": tokens.crust,
    "--preview-base": tokens.base,
    "--preview-line": tokens.surface1,
    "--preview-text": tokens.text,
    "--preview-accent": tokens.accent,
    "--preview-macro": tokens.macro ?? tokens.accent,
  } as CSSProperties;
  return (
    <button className={selected ? "theme-card selected" : "theme-card"} type="button" role="radio" aria-checked={selected} onClick={onSelect} title={`${theme.displayName} (${theme.family ?? "Themes"})`}>
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
        <small>{theme.family ?? "Themes"}</small>
      </span>
      {selected ? <span className="theme-card-check"><UiIcon icon="check" size="xs" /></span> : null}
    </button>
  );
}

function ThemePicker() {
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
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter themes" aria-label="Filter themes" spellCheck={false} />
        </label>
        <Segmented label="Theme appearance" value={filter} onChange={setFilter} options={[{ value: "all", label: "All" }, { value: "dark", label: "Dark" }, { value: "light", label: "Light" }]} />
      </div>
      <div className="theme-grid" role="radiogroup" aria-label="Theme">
        {visible.map((entry) => <ThemeCard key={entry.id} theme={entry} selected={entry.id === theme.id} onSelect={() => setThemeId(entry.id)} />)}
        {visible.length === 0 ? <p className="muted">No themes match.</p> : null}
      </div>
    </div>
  );
}

export function SettingsDialog({ open, onOpenChange, initialSection = "appearance" }: SettingsDialogProps) {
  const { theme, accentOverride, setAccentOverride, reduceTransparency, setReduceTransparency } = useTheme();
  const { preferences, setPreference, resetPreferences } = usePreferences();
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
    { id: "theme", section: "appearance", title: "Color theme", keywords: "dark light colors palette vs code", wide: true, control: <ThemePicker /> },
    {
      id: "accent", section: "appearance", title: "Accent color", description: "Overrides the theme accent used for selection and primary actions.", keywords: "highlight color",
      control: (
        <div className="accent-row" role="radiogroup" aria-label="Accent color">
          <button className={accentOverride === null ? "accent-swatch default selected" : "accent-swatch default"} type="button" role="radio" aria-checked={accentOverride === null} onClick={() => setAccentOverride(null)} title="Theme default">
            <span style={{ background: theme.tokens.accent }} />Theme
          </button>
          {accentPresets.map((preset) => (
            <button className={accentOverride === preset.value ? "accent-swatch selected" : "accent-swatch"} type="button" role="radio" aria-checked={accentOverride === preset.value} aria-label={preset.label} title={preset.label} key={preset.value} onClick={() => setAccentOverride(preset.value)}>
              <span style={{ background: preset.value }} />
            </button>
          ))}
        </div>
      ),
    },
    {
      id: "zoom", section: "appearance", title: "Interface zoom", description: "Scales the whole window, like browser zoom.", keywords: "scale size font ui",
      control: <Segmented label="Interface zoom" value={String(preferences.interfaceZoom)} onChange={(value) => setPreference("interfaceZoom", Number(value))} options={interfaceZoomOptions.map((zoom) => ({ value: String(zoom), label: `${Math.round(zoom * 100)}%` }))} />,
    },
    {
      id: "transparency", section: "appearance", title: "Reduce transparency", description: transparencyAvailable ? "Use an opaque window background instead of the system backdrop." : "The system backdrop is only available on Windows.", keywords: "acrylic blur opaque backdrop",
      control: <input type="checkbox" className="switch" aria-label="Reduce transparency" checked={reduceTransparency} disabled={!transparencyAvailable} onChange={(event) => setReduceTransparency(event.target.checked)} />,
    },
    {
      id: "font-size", section: "editor", title: "Text size", description: "Size of source and target text in the editor.", keywords: "font editor",
      control: <Segmented label="Editor text size" value={String(preferences.editorFontSize)} onChange={(value) => setPreference("editorFontSize", Number(value))} options={editorFontSizes.map((size) => ({ value: String(size), label: String(size) }))} />,
    },
    {
      id: "macros", section: "editor", title: "Highlight macros", description: "Tint game macros such as <num(lnum1)> in the editor.", keywords: "sestring syntax color",
      control: <input type="checkbox" className="switch" aria-label="Highlight macros" checked={preferences.highlightMacros} onChange={(event) => setPreference("highlightMacros", event.target.checked)} />,
    },
    {
      id: "control-chars", section: "editor", title: "Show control characters", description: "Mark invisible control characters instead of hiding them.", keywords: "whitespace special invisible",
      control: <input type="checkbox" className="switch" aria-label="Show control characters" checked={preferences.showControlCharacters} onChange={(event) => setPreference("showControlCharacters", event.target.checked)} />,
    },
    {
      id: "density", section: "editor", title: "String list density", description: "Row height of the strings list.", keywords: "compact rows spacing",
      control: <Segmented label="String list density" value={preferences.listDensity} onChange={(value) => setPreference("listDensity", value)} options={[{ value: "compact", label: "Compact" }, { value: "comfortable", label: "Comfortable" }]} />,
    },
    {
      id: "focus-next", section: "workflow", title: "Focus the next target", description: "After Save & next, put the cursor in the next string's target.", keywords: "save next cursor keyboard",
      control: <input type="checkbox" className="switch" aria-label="Focus the next target" checked={preferences.focusTargetOnNext} onChange={(event) => setPreference("focusTargetOnNext", event.target.checked)} />,
    },
    {
      id: "shortcuts", section: "keyboard", title: "Shortcuts", keywords: keyboardShortcuts.map((shortcut) => `${shortcut.keys} ${shortcut.action}`).join(" "), wide: true,
      control: (
        <table className="shortcut-table">
          <tbody>
            {keyboardShortcuts.filter((shortcut) => !query.trim() || `${shortcut.keys} ${shortcut.action}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())).map((shortcut) => (
              <tr key={shortcut.keys + shortcut.action}><td>{shortcut.action}</td><td className="muted">{shortcut.group}</td><td><kbd>{shortcut.keys}</kbd></td></tr>
            ))}
          </tbody>
        </table>
      ),
    },
    {
      id: "about", section: "about", title: "Aeria", description: version ? `Version ${version}` : "Desktop application", keywords: "version",
      control: null,
    },
    {
      id: "storage", section: "about", title: "Where settings live", description: "Appearance and editor settings are stored on this computer only. They are never written to project repositories.", keywords: "local storage",
      control: null,
    },
    {
      id: "reset", section: "about", title: "Reset editor settings", description: "Restore zoom, text size, and workflow options to their defaults.", keywords: "defaults",
      control: <button className="button button-secondary" type="button" onClick={resetPreferences}>Reset</button>,
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
            <Dialog.Title className="dialog-title">Settings</Dialog.Title>
            <label className="search-field">
              <UiIcon icon="search" size="sm" />
              <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search settings" aria-label="Search settings" spellCheck={false} />
            </label>
            <nav aria-label="Settings sections">
              {sections.map((entry) => (
                <button key={entry.id} type="button" className={!normalized && section === entry.id ? "settings-nav-item active" : "settings-nav-item"} onClick={() => { setQuery(""); setSection(entry.id); }}>
                  <UiIcon icon={entry.icon} size="sm" />{entry.label}
                </button>
              ))}
            </nav>
          </aside>
          <section className="settings-content">
            <header className="settings-content-head">
              <h3>{normalized ? `Results for “${query.trim()}”` : sections.find((entry) => entry.id === section)?.label}</h3>
              <Dialog.Close className="icon-button icon-button-ghost" aria-label="Close settings"><UiIcon icon="x" size="sm" /></Dialog.Close>
            </header>
            <div className="settings-list">
              {visibleEntries.map((entry) => (
                <div className={entry.wide ? "setting setting-wide" : "setting"} key={entry.id}>
                  <div className="setting-text">
                    <strong>{entry.title}</strong>
                    {normalized ? <small className="setting-section">{sections.find((candidate) => candidate.id === entry.section)?.label}</small> : null}
                    {entry.description ? <p>{entry.description}</p> : null}
                  </div>
                  {entry.control ? <div className="setting-control">{entry.control}</div> : null}
                </div>
              ))}
              {visibleEntries.length === 0 ? <div className="empty-state"><UiIcon icon="search" size="xl" /><strong>No settings found</strong></div> : null}
            </div>
          </section>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
