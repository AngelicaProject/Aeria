import { useEffect } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

type WindowChromeProps = {
  context: string;
  detail?: string;
  mode: "launcher" | "workbench";
  onClose?: () => void;
};

const windowSizes = {
  launcher: { width: 980, height: 720 },
  workbench: { width: 1440, height: 900 },
} as const;

export function WindowChrome({ context, detail, mode, onClose }: WindowChromeProps) {
  useEffect(() => {
    try {
      const window = getCurrentWindow();
      const size = windowSizes[mode];

      void Promise.all([
        window.setResizable(mode === "workbench"),
        window.setSize(new LogicalSize(size.width, size.height)),
      ]).catch(() => {
        // The renderer can also run in a browser during development.
      });
    } catch {
      // The renderer can also run in a browser during development.
    }
  }, [mode]);

  function handleDrag() {
    try {
      void getCurrentWindow().startDragging().catch(() => undefined);
    } catch {
      // Window dragging is only available in the desktop shell.
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
      <button className="chrome-drag-region" type="button" onMouseDown={handleDrag} aria-label="Move window">
        <span>{context}</span>
        {detail ? <span className="chrome-detail">{detail}</span> : null}
      </button>
      <div className="window-controls" aria-label="Window controls">
        <button className="window-control" type="button" aria-label="Minimize window" onClick={handleMinimize}>
          <span aria-hidden="true">−</span>
        </button>
        <button className="window-control close" type="button" aria-label="Close window" onClick={handleClose}>
          <span aria-hidden="true">×</span>
        </button>
      </div>
    </header>
  );
}
