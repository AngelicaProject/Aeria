import type { MessageKey } from "../i18n/translate";
import { IconButton } from "../ui/primitives/IconButton";
import { useI18n } from "../ui/i18n";

export type BottomPanelTab = "tasks" | "gitChanges" | "diagnostics";

export const bottomPanelTabs: ReadonlyArray<{ id: BottomPanelTab; label: MessageKey; empty: MessageKey }> = [
  { id: "tasks", label: "bottom.tasks", empty: "bottom.tasksEmpty" },
  { id: "gitChanges", label: "bottom.gitChanges", empty: "bottom.gitChangesEmpty" },
  { id: "diagnostics", label: "bottom.diagnostics", empty: "bottom.diagnosticsEmpty" },
];

type BottomPanelProps = {
  activeTab: BottomPanelTab;
  onTabChange: (tab: BottomPanelTab) => void;
  onCollapse?: () => void;
  onDetach?: () => void;
  onDropPanel?: (panelId: string) => void;
};

export function BottomPanel({ activeTab, onTabChange, onCollapse, onDetach, onDropPanel }: BottomPanelProps) {
  const { t } = useI18n();
  const active = bottomPanelTabs.find((tab) => tab.id === activeTab) ?? bottomPanelTabs[0]!;
  return (
    <section
      className="panel bottom-panel"
      aria-label={t("bottom.label")}
      onDragOver={(event) => { if (onDropPanel) event.preventDefault(); }}
      onDrop={(event) => { event.preventDefault(); const dragged = event.dataTransfer.getData("text/aeria-panel"); if (dragged) onDropPanel?.(dragged); }}
    >
      <header className="panel-header">
        <div className="panel-tabs" role="tablist" aria-label={t("bottom.label")}>
          {bottomPanelTabs.map((tab) => <button className={tab.id === activeTab ? "panel-tab active" : "panel-tab"} type="button" role="tab" key={tab.id} aria-selected={tab.id === activeTab} onClick={() => onTabChange(tab.id)}>{t(tab.label)}</button>)}
        </div>
        <span className="spacer" />
        {onDetach ? <IconButton icon="externalLink" label={t("common.openInNewWindow")} onClick={onDetach} /> : null}
        {onCollapse ? <IconButton icon="chevronDown" label={t("bottom.hide")} shortcut="Ctrl+J" onClick={onCollapse} /> : null}
      </header>
      <div className="panel-body bottom-panel-body" role="tabpanel" aria-live="polite"><p className="muted">{t(active.empty)}</p></div>
    </section>
  );
}
