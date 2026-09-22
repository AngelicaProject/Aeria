import { Icon } from "../ui/primitives/Icon";

export type DocumentTab = {
  id: string;
  label: string;
  detail?: string;
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
  return (
    <div className="document-tabs" role="tablist" aria-label="Open documents">
      {documents.map((document) => {
        const active = document.id === activeDocumentId;
        const className = ["document-tab", active ? "active" : "", document.pinned ? "pinned" : "", document.preview ? "preview" : "", document.dirty ? "dirty" : ""].filter(Boolean).join(" ");
        return (
          <div className={className} key={document.id} role="presentation" draggable={Boolean(onReorder)} onDragStart={(event) => { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("text/aeria-document", document.id); }} onDragOver={(event) => { if (onReorder) event.preventDefault(); }} onDrop={(event) => { event.preventDefault(); const movingId = event.dataTransfer.getData("text/aeria-document"); if (movingId && onReorder && movingId !== document.id) onReorder(movingId, document.id); }}>
            <button type="button" role="tab" aria-selected={active} title={document.detail} onClick={() => onSelect(document.id)} onDoubleClick={() => onPin?.(document.id)} onAuxClick={(event) => { if (event.button === 1) { event.preventDefault(); onClose(document.id); } }}>
              {document.dirty ? <span className="tab-dirty-dot" aria-label="Unsaved changes">●</span> : null}<span className="document-tab-label">{document.label}</span>{document.detail ? <small>{document.detail}</small> : null}
            </button>
            {document.closable ? <button className="document-tab-close" type="button" aria-label={`Close ${document.label}`} onClick={() => onClose(document.id)}><Icon name="close" size={12} /></button> : null}
          </div>
        );
      })}
      {documents.length === 0 ? <span className="document-tabs-empty">No open documents</span> : null}
    </div>
  );
}
