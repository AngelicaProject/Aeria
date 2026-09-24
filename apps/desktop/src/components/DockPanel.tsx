import type { PropsWithChildren, ReactNode } from "react";
import { DropdownMenu } from "radix-ui";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

type DockPanelProps = PropsWithChildren<{
  title: string;
  meta?: ReactNode;
  headerActions?: ReactNode;
  className?: string;
  hidden?: boolean;
  panelId?: string;
  moveTargets?: readonly { id: string; label: string }[];
  canFloat?: boolean;
  onMove?: (regionId: string) => void;
  onFloat?: () => void;
  onHide?: () => void;
  onDropPanel?: (panelId: string) => void;
}>;

export function DockPanel({
  title,
  meta,
  headerActions,
  className = "",
  hidden = false,
  panelId,
  moveTargets = [],
  canFloat = false,
  onMove,
  onFloat,
  onHide,
  onDropPanel,
  children,
}: DockPanelProps) {
  const { t } = useI18n();
  const hasMenu = moveTargets.length > 0 || canFloat || Boolean(onHide);
  return (
    <section
      className={`panel dock-panel${className ? ` ${className}` : ""}`}
      aria-label={title}
      hidden={hidden}
      onDragOver={(event) => { if (onDropPanel) event.preventDefault(); }}
      onDrop={(event) => { event.preventDefault(); const dragged = event.dataTransfer.getData("text/aeria-panel"); if (dragged) onDropPanel?.(dragged); }}
    >
      <header
        className="panel-header"
        draggable={Boolean(panelId)}
        onDragStart={(event) => { if (panelId) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("text/aeria-panel", panelId); } }}
      >
        <span className="panel-title">{title}</span>
        {meta ? <span className="panel-meta">{meta}</span> : null}
        <span className="spacer" />
        {headerActions ? <div className="panel-actions" role="group" aria-label={t("dock.actions", { title })}>{headerActions}</div> : null}
        {hasMenu ? (
          <DropdownMenu.Root>
            <DropdownMenu.Trigger asChild>
              <button className="icon-button icon-button-ghost" type="button" aria-label={t("dock.options", { title })}><UiIcon icon="ellipsis" size="md" /></button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content className="menu-content" align="end" sideOffset={4}>
                {moveTargets.map((target) => (
                  <DropdownMenu.Item className="menu-item" key={target.id} onSelect={() => onMove?.(target.id)}>
                    <span className="menu-item-label">{target.label}</span>
                  </DropdownMenu.Item>
                ))}
                {canFloat ? <DropdownMenu.Item className="menu-item" onSelect={() => onFloat?.()}><span className="menu-item-label">{t("common.openInNewWindow")}</span></DropdownMenu.Item> : null}
                {onHide ? <>
                  {moveTargets.length > 0 || canFloat ? <DropdownMenu.Separator className="menu-separator" /> : null}
                  <DropdownMenu.Item className="menu-item" onSelect={onHide}><span className="menu-item-label">{t("common.hide")}</span></DropdownMenu.Item>
                </> : null}
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        ) : null}
      </header>
      <div className="panel-body">{children}</div>
    </section>
  );
}
