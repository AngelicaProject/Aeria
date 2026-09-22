import { useEffect, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { ApplicationMenu, type ApplicationMenuDefinition } from "./ApplicationMenu";
import { Icon } from "../ui/primitives/Icon";

type WindowChromeProps = {
  context: string;
  detail?: string;
  mode: "launcher" | "workbench";
  projectName?: string;
  onClose?: () => void;
  onCloseProject?: () => void;
  onToggleDock?: () => void;
};

const launcherSize = { width: 900, height: 560 } as const;
const launcherMinimum = { width: 900, height: 560 } as const;
const workbenchMinimum = { width: 1140, height: 710 } as const;

let workbenchInitialized = false;

export function WindowChrome({ context, detail, mode, projectName, onClose, onCloseProject, onToggleDock }: WindowChromeProps) {
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
        } else {
          await window.setResizable(true);
          await window.setMinSize(new LogicalSize(workbenchMinimum.width, workbenchMinimum.height));
          if (!workbenchInitialized) {
            workbenchInitialized = true;
            await window.maximize();
          }
        }
        if (active) setMaximized(await window.isMaximized());
        unlistenResize = await window.onResized(async () => {
          if (active) setMaximized(await window.isMaximized());
        });
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
    try {
      void getCurrentWindow().minimize().catch(() => undefined);
    } catch {
      // Window controls are only available in the desktop shell.
    }
  }

  function handleClose() {
    if (onClose) {
      onClose();
      return;
    }
    try {
      void getCurrentWindow().close().catch(() => undefined);
    } catch {
      // Window controls are only available in the desktop shell.
    }
  }

  const menus: readonly ApplicationMenuDefinition[] = mode === "workbench"
    ? [
        { id: "file", label: "File", items: [{ id: "close", label: "Close project", shortcut: "Ctrl+W", onSelect: onCloseProject ?? handleClose }] },
        { id: "view", label: "View", items: [{ id: "sheets", label: "Sheets panel", onSelect: onToggleDock ?? (() => undefined) }] },
        { id: "project", label: "Project", items: [{ id: "close-project", label: "Close project", onSelect: onCloseProject ?? handleClose }] },
        { id: "window", label: "Window", items: [
          { id: "minimize", label: "Minimize", onSelect: handleMinimize },
          { id: "maximize", label: maximized ? "Restore" : "Maximize", onSelect: () => void handleToggleMaximize() },
        ] },
      ]
    : [];

  return (
    <header className={mode === "workbench" ? "app-chrome workbench-chrome" : "app-chrome launcher-chrome"}>
      {mode === "workbench" ? (
        <>
          <div className="chrome-project" title={context}>
            <span className="chrome-project-name">{projectName ?? context}</span>
            {detail ? <span className="chrome-project-detail">{detail}</span> : null}
          </div>
          <div className="chrome-drag-region" data-tauri-drag-region />
          <ApplicationMenu menus={menus} />
        </>
      ) : (
        <div className="chrome-brand" aria-label="Aeria">Aeria</div>
      )}
      <div className="window-controls" aria-label="Window controls">
        <button className="window-control" type="button" aria-label="Minimize window" onClick={handleMinimize}>
          <Icon name="minimize" size={13} />
        </button>
        {mode === "workbench" ? (
          <button className="window-control" type="button" aria-label={maximized ? "Restore window" : "Maximize window"} onClick={() => void handleToggleMaximize()}>
            <Icon name="maximize" size={13} />
          </button>
        ) : null}
        <button className="window-control close" type="button" aria-label="Close window" onClick={handleClose}>
          <Icon name="close" size={13} />
        </button>
      </div>
    </header>
  );
}
