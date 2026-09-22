import { useEffect, useRef, useState, type PropsWithChildren, type ReactNode } from "react";
import { Icon } from "../ui/primitives/Icon";

type DockPanelProps = PropsWithChildren<{
  title: string;
  meta?: ReactNode;
  headerActions?: ReactNode;
  className?: string;
  panelId?: string;
  moveTargets?: readonly { id: string; label: string }[];
  canFloat?: boolean;
  onMove?: (regionId: string) => void;
  onFloat?: () => void;
  onHide?: () => void;
  onDragStart?: (panelId: string) => void;
  onDropPanel?: (panelId: string) => void;
}>;

export function DockPanel({
  title,
  meta,
  headerActions,
  className = "",
  panelId,
  moveTargets = [],
  canFloat = false,
  onMove,
  onFloat,
  onHide,
  onDragStart,
  onDropPanel,
  children,
}: DockPanelProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const rootRef = useRef<HTMLElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    function closeMenu(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) setMenuOpen(false);
    }
    window.addEventListener("pointerdown", closeMenu);
    return () => window.removeEventListener("pointerdown", closeMenu);
  }, [menuOpen]);

  return (
    <section
      className={`dock-panel ${className}`}
      ref={rootRef}
      onDragOver={(event) => { if (onDropPanel) event.preventDefault(); }}
      onDrop={(event) => { event.preventDefault(); const dragged = event.dataTransfer.getData("text/aeria-panel"); if (dragged) onDropPanel?.(dragged); }}
    >
      <header className="dock-header" draggable={Boolean(panelId && onDragStart)} onDragStart={(event) => { if (panelId) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("text/aeria-panel", panelId); onDragStart?.(panelId); } }}>
        <span className="dock-title">{title}</span>{meta ? <span className="dock-meta">{meta}</span> : null}<span className="dock-spacer" />
        {headerActions ? <div className="dock-header-actions" role="group" aria-label={`${title} actions`}>{headerActions}</div> : null}
        <button className="dock-action" type="button" aria-label={`${title} options`} aria-haspopup="menu" aria-expanded={menuOpen} onClick={() => setMenuOpen((current) => !current)}><Icon name="more" size={15} /></button>
        {menuOpen ? <div className="dock-panel-menu" role="menu">
          {moveTargets.map((target) => <button type="button" role="menuitem" key={target.id} onClick={() => { onMove?.(target.id); setMenuOpen(false); }}>Move {target.label}</button>)}
          {canFloat ? <button type="button" role="menuitem" onClick={() => { onFloat?.(); setMenuOpen(false); }}>Float / Detach</button> : null}
          {onHide ? <button type="button" role="menuitem" onClick={() => { onHide(); setMenuOpen(false); }}>Close / Hide</button> : null}
        </div> : null}
      </header>
      <div className="dock-body">{children}</div>
    </section>
  );
}
