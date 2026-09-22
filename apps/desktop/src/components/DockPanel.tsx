import type { PropsWithChildren, ReactNode } from "react";
import { Icon } from "../ui/primitives/Icon";

type DockPanelProps = PropsWithChildren<{
  title: string;
  meta?: ReactNode;
  className?: string;
}>;

export function DockPanel({ title, meta, className = "", children }: DockPanelProps) {
  return (
    <section className={`dock-panel ${className}`}>
      <header className="dock-header"><span className="dock-title">{title}</span>{meta ? <span className="dock-meta">{meta}</span> : null}<span className="dock-spacer" /><button className="dock-action" type="button" aria-label={`${title} options`} disabled><Icon name="more" size={15} /></button></header>
      <div className="dock-body">{children}</div>
    </section>
  );
}
