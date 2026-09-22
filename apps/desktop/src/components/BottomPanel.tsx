import { Icon } from "../ui/primitives/Icon";

export type BottomPanelTab = "tasks" | "gitChanges" | "diagnostics";

const tabs: Array<{ id: BottomPanelTab; label: string }> = [
  { id: "tasks", label: "Tasks" },
  { id: "gitChanges", label: "Git Changes" },
  { id: "diagnostics", label: "Diagnostics" },
];

type BottomPanelProps = {
  activeTab: BottomPanelTab;
  onTabChange: (tab: BottomPanelTab) => void;
  onCollapse?: () => void;
  onDetach?: () => void;
  panelId?: string;
  onDragStart?: (panelId: string) => void;
  onDropPanel?: (panelId: string) => void;
};

export function BottomPanel({ activeTab, onTabChange, onCollapse, onDetach, panelId, onDragStart, onDropPanel }: BottomPanelProps) {
  return (
    <section className="bottom-panel" aria-label="Bottom panel" onDragOver={(event) => { if (onDropPanel) event.preventDefault(); }} onDrop={(event) => { event.preventDefault(); const dragged = event.dataTransfer.getData("text/aeria-panel"); if (dragged) onDropPanel?.(dragged); }}>
      <header className="bottom-panel-tabs" draggable={Boolean(panelId && onDragStart)} onDragStart={(event) => { if (panelId) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("text/aeria-panel", panelId); onDragStart?.(panelId); } }}>
        {tabs.map((tab) => <button className={tab.id === activeTab ? "bottom-panel-tab active" : "bottom-panel-tab"} type="button" key={tab.id} aria-pressed={tab.id === activeTab} onClick={() => onTabChange(tab.id)}>{tab.label}</button>)}
        <button className="bottom-panel-collapse" type="button" aria-label="Detach bottom panel" title="Detach bottom panel" onClick={onDetach}><Icon name="external" size={13} /></button>
        <button className="bottom-panel-collapse" type="button" aria-label="Hide bottom panel" title="Hide bottom panel" onClick={onCollapse}><Icon name="chevronDown" size={13} /></button>
      </header>
      <div className="bottom-panel-body" aria-live="polite">{activeTab === "tasks" ? <span>No active tasks</span> : <span>{activeTab === "gitChanges" ? "Git changes are not available in this build." : "Diagnostics are not available in this build."}</span>}</div>
    </section>
  );
}
