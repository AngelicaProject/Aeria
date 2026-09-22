export type BottomPanelTab = "tasks" | "gitChanges" | "diagnostics";

const tabs: Array<{ id: BottomPanelTab; label: string }> = [
  { id: "tasks", label: "Tasks" },
  { id: "gitChanges", label: "Git Changes" },
  { id: "diagnostics", label: "Diagnostics" },
];

type BottomPanelProps = {
  activeTab: BottomPanelTab;
  onTabChange: (tab: BottomPanelTab) => void;
};

export function BottomPanel({ activeTab, onTabChange }: BottomPanelProps) {
  return (
    <section className="bottom-panel" aria-label="Bottom panel">
      <header className="bottom-panel-tabs">
        {tabs.map((tab) => <button className={tab.id === activeTab ? "bottom-panel-tab active" : "bottom-panel-tab"} type="button" key={tab.id} aria-pressed={tab.id === activeTab} onClick={() => onTabChange(tab.id)}>{tab.label}</button>)}
      </header>
      <div className="bottom-panel-body" aria-live="polite">{activeTab === "tasks" ? <span>No active tasks</span> : <span>{activeTab === "gitChanges" ? "Git changes are not available in this build." : "Diagnostics are not available in this build."}</span>}</div>
    </section>
  );
}
