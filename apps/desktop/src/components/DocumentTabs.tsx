import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

export type DocumentTab = {
  id: string;
  label: string;
  closable?: boolean;
  pinned?: boolean;
  preview?: boolean;
  dirty?: boolean;
};

type DocumentTabsProps = {
  documents: readonly DocumentTab[];
  activeDocumentId: string | null;
  onSelect: (documentId: string) => void;
  onClose: (documentId: string) => void;
  onPin?: (documentId: string) => void;
  onReorder?: (documentId: string, beforeDocumentId: string | null) => void;
};

export function DocumentTabs({ documents, activeDocumentId, onSelect, onClose, onPin, onReorder }: DocumentTabsProps) {
  const { t } = useI18n();
  return (
    <div className="doc-tabs" role="tablist" aria-label={t("tabs.label")}>
      {documents.map((document) => {
        const active = document.id === activeDocumentId;
        const className = ["doc-tab", active ? "active" : "", document.preview ? "preview" : "", document.dirty ? "dirty" : ""].filter(Boolean).join(" ");
        return (
          <div
            className={className}
            key={document.id}
            role="presentation"
            draggable={Boolean(onReorder)}
            onDragStart={(event) => { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("text/aeria-document", document.id); }}
            onDragOver={(event) => { if (onReorder) event.preventDefault(); }}
            onDrop={(event) => { event.preventDefault(); const movingId = event.dataTransfer.getData("text/aeria-document"); if (movingId && onReorder && movingId !== document.id) onReorder(movingId, document.id); }}
          >
            <button
              type="button"
              role="tab"
              className="doc-tab-main"
              aria-selected={active}
              title={document.preview ? t("tabs.preview", { label: document.label }) : document.label}
              onClick={() => onSelect(document.id)}
              onDoubleClick={() => onPin?.(document.id)}
              onAuxClick={(event) => { if (event.button === 1) { event.preventDefault(); onClose(document.id); } }}
            >
              <UiIcon icon="table2" size="sm" className="doc-tab-icon" />
              <span className="doc-tab-label">{document.label}</span>
            </button>
            {document.closable ? (
              <button className="doc-tab-close" type="button" aria-label={t(document.dirty ? "tabs.closeUnsaved" : "tabs.close", { label: document.label })} onClick={() => onClose(document.id)}>
                <span className="doc-tab-dirty" aria-hidden="true" />
                <UiIcon icon="x" size="xs" className="doc-tab-x" />
              </button>
            ) : null}
          </div>
        );
      })}
      {documents.length === 0 ? <span className="doc-tabs-empty">{t("tabs.empty")}</span> : null}
    </div>
  );
}
