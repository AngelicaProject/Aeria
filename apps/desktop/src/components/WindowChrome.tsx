import { useEffect, useState, type ReactNode } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { ApplicationMenu, type ApplicationMenuDefinition } from "./ApplicationMenu";
import { UpdateTitleButton } from "./UpdateNotice";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import appIcon from "../assets/app-icon-20.png";

type WindowChromeProps = {
  /** `detached` tool windows keep their own geometry. */
  mode: "launcher" | "workbench" | "detached";
  title?: string;
  subtitle?: string | null;
  /** Interactive content centered in the drag region, such as the command center. */
  center?: ReactNode;
  menus?: readonly ApplicationMenuDefinition[];
  actions?: ReactNode;
  onClose?: () => void;
};

const launcherSize = { width: 900, height: 560 } as const;
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

async function configureWindow(mode: WindowChromeProps["mode"]): Promise<void> {
  const window = getCurrentWindow();
  if (mode === "launcher") {
    await window.unmaximize();
    await window.setResizable(false);
    await window.setMaximizable(false);
    await window.setMinSize(new LogicalSize(launcherSize.width, launcherSize.height));
    await window.setSize(new LogicalSize(launcherSize.width, launcherSize.height));
    await window.center();
  } else if (mode === "workbench") {
    await window.setResizable(true);
    await window.setMaximizable(true);
    await window.setMinSize(new LogicalSize(workbenchMinimum.width, workbenchMinimum.height));
    const currentSize = (await window.innerSize()).toLogical(await window.scaleFactor());
    const saved = readSavedGeometry();
    const launcherSized = currentSize.width <= launcherSize.width + 20 && currentSize.height <= launcherSize.height + 20;
    if (saved?.maximized || (launcherSized && !saved)) {
      await window.maximize();
    } else if (saved && launcherSized) {
      await window.unmaximize();
      await window.setSize(new LogicalSize(saved.width, saved.height));
      await window.center();
    }
  }
}

export function WindowChrome({ mode, title, subtitle, center, menus = [], actions, onClose }: WindowChromeProps) {
  const { t } = useI18n();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let active = true;
    let unlistenResize: (() => void) | undefined;

    void (async () => {
      try {
        const window = getCurrentWindow();
        await configureWindow(mode);
        if (active) setMaximized(await window.isMaximized());
        const cleanupResize = await window.onResized(async () => {
          if (!active) return;
          const isMaximized = await window.isMaximized();
          setMaximized(isMaximized);
          if (mode !== "workbench") return;
          if (isMaximized) {
            saveGeometry({ width: workbenchMinimum.width, height: workbenchMinimum.height, maximized: true });
          } else {
            const size = (await window.innerSize()).toLogical(await window.scaleFactor());
            saveGeometry({ width: Math.round(size.width), height: Math.round(size.height), maximized: false });
          }
        });
        if (active) unlistenResize = cleanupResize;
        else cleanupResize();
      } catch (error) {
        if (isTauri()) console.warn("Could not configure the Aeria window", error);
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
      await window.toggleMaximize();
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

  const resizable = mode !== "launcher";

  // Double-clicking a drag region maximizes natively; no extra handler here.
  return (
    <header className={`titlebar titlebar-${mode}`}>
      <div className="titlebar-brand" data-tauri-drag-region>
        <img className="titlebar-icon" src={appIcon} alt="" aria-hidden="true" />
      </div>
      {menus.length > 0 ? <ApplicationMenu menus={menus} /> : null}
      <div className="titlebar-drag" data-tauri-drag-region>
        {center ?? null}
        {!center && title ? (
          <div className="titlebar-title" data-tauri-drag-region>
            <strong data-tauri-drag-region>{title}</strong>
            {subtitle ? <><span className="titlebar-title-sep" data-tauri-drag-region>/</span><span data-tauri-drag-region>{subtitle}</span></> : null}
          </div>
        ) : null}
      </div>
      {mode !== "detached" ? <UpdateTitleButton /> : null}
      {actions ? <div className="titlebar-actions">{actions}</div> : null}
      <div className="window-controls" role="group" aria-label={t("window.controls")}>
        <button className="window-control" type="button" aria-label={t("window.minimize")} onClick={handleMinimize}><UiIcon icon="minus" size="sm" /></button>
        {resizable ? <button className="window-control" type="button" aria-label={t(maximized ? "window.restore" : "window.maximize")} onClick={() => void handleToggleMaximize()}><UiIcon icon={maximized ? "copy" : "square"} size="xs" /></button> : null}
        <button className="window-control close" type="button" aria-label={t("window.close")} onClick={handleClose}><UiIcon icon="x" size="sm" /></button>
      </div>
    </header>
  );
}
