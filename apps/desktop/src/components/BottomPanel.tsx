import { IconButton } from "../ui/primitives/IconButton";

export type BottomPanelTab = "tasks" | "gitChanges" | "diagnostics";

const tabs: Array<{ id: BottomPanelTab; label: string; empty: string }> = [
  { id: "tasks", label: "Tasks", empty: "No active tasks." },
  { id: "gitChanges", label: "Git changes", empty: "Git changes are not available in this build." },
  { id: "diagnostics", label: "Diagnostics", empty: "Diagnostics are not available in this build." },
];

type BottomPanelProps = {
  activeTab: BottomPanelTab;
  onTabChange: (tab: BottomPanelTab) => void;
  onCollapse?: () => void;
  onDetach?: () => void;
  onDropPanel?: (panelId: string) => void;
};

export function BottomPanel({ activeTab, onTabChange, onCollapse, onDetach, onDropPanel }: BottomPanelProps) {
  const active = tabs.find((tab) => tab.id === activeTab) ?? tabs[0]!;
  return (
    <section
      className="panel bottom-panel"
      aria-label="Bottom panel"
      onDragOver={(event) => { if (onDropPanel) event.preventDefault(); }}
      onDrop={(event) => { event.preventDefault(); const dragged = event.dataTransfer.getData("text/aeria-panel"); if (dragged) onDropPanel?.(dragged); }}
    >
      <header className="panel-header">
        <div className="panel-tabs" role="tablist" aria-label="Bottom panel">
          {tabs.map((tab) => <button className={tab.id === activeTab ? "panel-tab active" : "panel-tab"} type="button" role="tab" key={tab.id} aria-selected={tab.id === activeTab} onClick={() => onTabChange(tab.id)}>{tab.label}</button>)}
        </div>
        <span className="spacer" />
        {onDetach ? <IconButton icon="externalLink" label="Open in new window" onClick={onDetach} /> : null}
        {onCollapse ? <IconButton icon="chevronDown" label="Hide panel" shortcut="Ctrl+J" onClick={onCollapse} /> : null}
      </header>
      <div className="panel-body bottom-panel-body" role="tabpanel" aria-live="polite"><p className="muted">{active.empty}</p></div>
    </section>
  );
}
