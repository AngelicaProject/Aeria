import { useEffect, useState } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

type WindowChromeProps = {
  context: string;
  detail?: string;
  mode: "launcher" | "workbench";
  launcherMode?: "recent" | "open" | "create";
  onClose?: () => void;
};

const launcherSizes = {
  recent: { width: 980, height: 620 },
  open: { width: 720, height: 500 },
  create: { width: 760, height: 600 },
} as const;

const launcherMinimums = {
  recent: { width: 900, height: 560 },
  open: { width: 680, height: 440 },
  create: { width: 720, height: 520 },
} as const;

const workbenchSize = { width: 1440, height: 900 } as const;
const workbenchMinimum = { width: 900, height: 600 } as const;

export function WindowChrome({ context, detail, mode, launcherMode = "recent", onClose }: WindowChromeProps) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let active = true;

    void (async () => {
      try {
        const window = getCurrentWindow();
        const size = mode === "workbench" ? workbenchSize : launcherSizes[launcherMode];
        const minimum = mode === "workbench" ? workbenchMinimum : launcherMinimums[launcherMode];

        if (mode === "launcher") await window.unmaximize();
        await window.setResizable(mode === "workbench");
        await window.setMinSize(new LogicalSize(minimum.width, minimum.height));
        await window.setSize(new LogicalSize(size.width, size.height));
        const isMaximized = mode === "workbench" && await window.isMaximized();
        if (active) setMaximized(isMaximized);
      } catch {
        // The renderer can also run in a browser during development.
      }
    })();

    return () => {
      active = false;
    };
  }, [launcherMode, mode]);

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

  return (
    <header className="window-chrome">
      <div className="chrome-brand" aria-label="Aeria">
        <span className="chrome-mark" aria-hidden="true">A</span>
        <span className="chrome-name">Aeria</span>
      </div>
      <div
        className="chrome-drag-region"
        data-tauri-drag-region
        onDoubleClick={mode === "workbench" ? () => void handleToggleMaximize() : undefined}
      >
        <span>{context}</span>
        {detail ? <span className="chrome-detail">{detail}</span> : null}
      </div>
      <div className="window-controls" aria-label="Window controls">
        <button className="window-control" type="button" aria-label="Minimize window" onClick={handleMinimize}>
          <span aria-hidden="true">−</span>
        </button>
        {mode === "workbench" ? (
          <button
            className="window-control maximize"
            type="button"
            aria-label={maximized ? "Restore window" : "Maximize window"}
            onClick={() => void handleToggleMaximize()}
          >
            <span aria-hidden="true">{maximized ? "❐" : "□"}</span>
          </button>
        ) : null}
        <button className="window-control close" type="button" aria-label="Close window" onClick={handleClose}>
          <span aria-hidden="true">×</span>
        </button>
      </div>
    </header>
  );
}
