import { Icon } from "../ui/primitives/Icon";

export type DocumentTab = {
  id: string;
  label: string;
  detail?: string;
  closable?: boolean;
};

type DocumentTabsProps = {
  documents: readonly DocumentTab[];
  activeDocumentId: string | null;
  onSelect: (documentId: string) => void;
  onClose: (documentId: string) => void;
};

export function DocumentTabs({ documents, activeDocumentId, onSelect, onClose }: DocumentTabsProps) {
  return (
    <div className="document-tabs" role="tablist" aria-label="Open documents">
      {documents.map((document) => {
        const active = document.id === activeDocumentId;
        return (
          <div className={active ? "document-tab active" : "document-tab"} key={document.id} role="presentation">
            <button type="button" role="tab" aria-selected={active} onClick={() => onSelect(document.id)}>
              <span>{document.label}</span>{document.detail ? <small>{document.detail}</small> : null}
            </button>
            {document.closable ? <button className="document-tab-close" type="button" aria-label={`Close ${document.label}`} onClick={() => onClose(document.id)}><Icon name="close" size={12} /></button> : null}
          </div>
        );
      })}
    </div>
  );
}
