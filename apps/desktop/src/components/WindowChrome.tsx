import { useEffect, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { ApplicationMenu, type ApplicationMenuDefinition } from "./ApplicationMenu";
import { Icon } from "../ui/primitives/Icon";
import appIcon from "../assets/app-icon-20.png";

type WindowChromeProps = {
  context: string;
  detail?: string;
  mode: "launcher" | "workbench";
  projectName?: string;
  onClose?: () => void;
  onCloseProject?: () => void;
  onToggleDock?: () => void;
  onToggleBottom?: () => void;
  onSelectTool?: (tool: "search" | "ai" | "git") => void;
  onQuickFind?: () => void;
};

const launcherSize = { width: 900, height: 560 } as const;
const launcherMinimum = { width: 900, height: 560 } as const;
const workbenchMinimum = { width: 1140, height: 710 } as const;
const geometryKey = "aeria.workbench.geometry";

type SavedGeometry = { width: number; height: number; maximized: boolean };

function readSavedGeometry(): SavedGeometry | null {
  try {
    const value = JSON.parse(localStorage.getItem(geometryKey) ?? "null") as Partial<SavedGeometry> | null;
    return value && typeof value.width === "number" && typeof value.height === "number" && typeof value.maximized === "boolean"
      ? { width: value.width, height: value.height, maximized: value.maximized }
      : null;
  } catch {
    return null;
  }
}

function saveGeometry(geometry: SavedGeometry): void {
  try {
    localStorage.setItem(geometryKey, JSON.stringify(geometry));
  } catch {
    // Geometry persistence is a convenience and must not block window changes.
  }
}

export function WindowChrome({ context, detail, mode, projectName, onClose, onCloseProject, onToggleDock, onToggleBottom, onSelectTool, onQuickFind }: WindowChromeProps) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let active = true;
    let unlistenResize: (() => void) | undefined;

    void (async () => {
      try {
        const window = getCurrentWindow();
        if (mode === "launcher") {
          await window.unmaximize();
          await window.setResizable(false);
          await window.setMinSize(new LogicalSize(launcherMinimum.width, launcherMinimum.height));
          await window.setSize(new LogicalSize(launcherSize.width, launcherSize.height));
          await window.center();
        } else {
          await window.setResizable(true);
          await window.setMinSize(new LogicalSize(workbenchMinimum.width, workbenchMinimum.height));
          const currentSize = await window.innerSize();
          const saved = readSavedGeometry();
          const launcherSized = currentSize.width <= 920 && currentSize.height <= 580;
          if (saved?.maximized || launcherSized && !saved) {
            await window.maximize();
          } else if (saved && launcherSized) {
            await window.unmaximize();
            await window.setSize(new LogicalSize(saved.width, saved.height));
          }
        }
        if (active) setMaximized(await window.isMaximized());
        const cleanupResize = await window.onResized(async () => {
          if (!active) return;
          const isMaximized = await window.isMaximized();
          setMaximized(isMaximized);
          if (mode === "workbench" && !isMaximized) {
            const size = await window.innerSize();
            saveGeometry({ width: size.width, height: size.height, maximized: false });
          } else if (mode === "workbench") {
            saveGeometry({ width: workbenchMinimum.width, height: workbenchMinimum.height, maximized: true });
          }
        });
        if (active) unlistenResize = cleanupResize;
        else cleanupResize();
      } catch {
        // The renderer can also run in a browser during development.
      }
    })();

    return () => {
      active = false;
      unlistenResize?.();
    };
  }, [mode]);

  async function handleToggleMaximize() {
    try {
      const window = getCurrentWindow();
      if (await window.isMaximized()) await window.unmaximize();
      else await window.maximize();
      setMaximized(await window.isMaximized());
    } catch {
      // Window controls are only available in the desktop shell.
    }
  }

  function handleMinimize() {
    try { void getCurrentWindow().minimize().catch(() => undefined); } catch { /* browser fallback */ }
  }

  function handleClose() {
    if (onClose) { onClose(); return; }
    try { void getCurrentWindow().close().catch(() => undefined); } catch { /* browser fallback */ }
  }

  function menuCommand(spec: Omit<import("./ApplicationMenu").ApplicationMenuCommandItem, "kind">): import("./ApplicationMenu").ApplicationMenuCommandItem {
    const { onSelect, ...rest } = spec;
    return onSelect ? { ...rest, kind: "command", onSelect } : { ...rest, kind: "command", disabled: true };
  }

  const menus: readonly ApplicationMenuDefinition[] = mode === "workbench"
    ? [
        { id: "file", label: "File", items: [menuCommand({ id: "close", label: "Close project", onSelect: onCloseProject ?? handleClose })] },
        { id: "view", label: "View", items: [menuCommand({ id: "sheets", label: "Sheets panel", ...(onToggleDock ? { onSelect: onToggleDock } : {}) }), menuCommand({ id: "search", label: "Project Search", ...(onSelectTool ? { onSelect: () => onSelectTool("search") } : {}) }), menuCommand({ id: "bottom", label: "Bottom panel", ...(onToggleBottom ? { onSelect: onToggleBottom } : {}) })] },
        { id: "project", label: "Project", items: [menuCommand({ id: "close-project", label: "Close project", onSelect: onCloseProject ?? handleClose })] },
        { id: "sheet", label: "Sheet", items: [menuCommand({ id: "quick-find", label: "Quick Find sheets", shortcut: "Ctrl+F", ...(onQuickFind ? { onSelect: onQuickFind } : {}) })] },
        { id: "translation", label: "Translation", items: [menuCommand({ id: "translation-unavailable", label: "Translation commands unavailable", disabled: true })] },
        { id: "ai", label: "AI", items: [menuCommand({ id: "open-ai", label: "Open AI panel", onSelect: () => onSelectTool?.("ai") })] },
        { id: "git", label: "Git", items: [menuCommand({ id: "open-git", label: "Open Git panel", onSelect: () => onSelectTool?.("git") })] },
        { id: "window", label: "Window", items: [menuCommand({ id: "minimize", label: "Minimize", onSelect: handleMinimize }), menuCommand({ id: "maximize", label: maximized ? "Restore" : "Maximize", onSelect: () => void handleToggleMaximize() })] },
        { id: "help", label: "Help", items: [menuCommand({ id: "about", label: "About Aeria", disabled: true })] },
      ]
    : [];

  return (
    <header className={mode === "workbench" ? "app-chrome workbench-chrome" : "app-chrome launcher-chrome"}>
      {mode === "workbench" ? (
        <>
          <div className="chrome-project" title={context}>
            <img className="chrome-app-icon" src={appIcon} alt="" aria-hidden="true" />
            <span className="chrome-project-name">{projectName ?? context}</span>
            {detail ? <span className="chrome-project-detail">{detail}</span> : null}
          </div>
          <ApplicationMenu menus={menus} />
          <div className="chrome-drag-region" data-tauri-drag-region />
        </>
      ) : (
        <>
          <div className="chrome-brand"><img className="chrome-app-icon" src={appIcon} alt="" aria-hidden="true" /><span>Aeria</span></div>
          <div className="chrome-drag-region" data-tauri-drag-region />
        </>
      )}
      <div className="window-controls" aria-label="Window controls">
        <button className="window-control" type="button" aria-label="Minimize window" onClick={handleMinimize}><Icon name="minimize" size={13} /></button>
        {mode === "workbench" ? <button className="window-control" type="button" aria-label={maximized ? "Restore window" : "Maximize window"} onClick={() => void handleToggleMaximize()}><Icon name="maximize" size={13} /></button> : null}
        <button className="window-control close" type="button" aria-label="Close window" onClick={handleClose}><Icon name="close" size={13} /></button>
      </div>
    </header>
  );
}
