import { UiIcon, type UiIconName } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";
import { RightClickMenu, type MenuEntry } from "../ui/primitives/RightClickMenu";

export type DocumentTab = {
  id: string;
  label: string;
  closable?: boolean;
  pinned?: boolean;
  preview?: boolean;
  dirty?: boolean;
  icon?: UiIconName;
};

type DocumentTabsProps = {
  documents: readonly DocumentTab[];
  activeDocumentId: string | null;
  onSelect: (documentId: string) => void;
  onClose: (documentId: string) => void;
  /** Closes several tabs at once, from a tab's right-click menu. */
  onCloseMany: (documentIds: readonly string[]) => void;
  /** Turns a preview into a tab of its own. */
  onKeep?: (documentId: string) => void;
  onPinnedChange?: (documentId: string, pinned: boolean) => void;
  onReorder?: (documentId: string, beforeDocumentId: string | null) => void;
  /** Entries a tab's right-click menu adds after its own. */
  menuFor?: (documentId: string) => readonly MenuEntry[];
};

/**
 * The open sheets and commits as tabs, as editors keep them: a single click
 * in the Sheets explorer opens a preview (italic) that the next one
 * replaces; a double click, an edit, or **Keep open** gives it a tab of its
 * own. A pinned tab stays first, shows a pin instead of its close button,
 * and is closed only on purpose: not by a middle click, Ctrl+W, or the
 * menu's bulk closing.
 */
export function DocumentTabs({ documents, activeDocumentId, onSelect, onClose, onCloseMany, onKeep, onPinnedChange, onReorder, menuFor }: DocumentTabsProps) {
  const { t } = useI18n();
  const closable = (document: DocumentTab) => document.closable && !document.pinned;
  const menu = (document: DocumentTab, index: number): MenuEntry[] => {
    const others = documents.filter((other) => other.id !== document.id && closable(other)).map((other) => other.id);
    const right = documents.slice(index + 1).filter(closable).map((other) => other.id);
    const all = documents.filter(closable).map((other) => other.id);
    const extra = menuFor?.(document.id) ?? [];
    return [
      { id: "close", icon: "x", label: t("tabs.menu.close"), ...(document.id === activeDocumentId && !document.pinned ? { shortcut: "Ctrl+W" } : {}), disabled: !document.closable, run: () => onClose(document.id) },
      { id: "closeOthers", label: t("tabs.menu.closeOthers"), disabled: others.length === 0, run: () => onCloseMany(others) },
      { id: "closeRight", label: t("tabs.menu.closeRight"), disabled: right.length === 0, run: () => onCloseMany(right) },
      { id: "closeAll", label: t("tabs.menu.closeAll"), disabled: all.length === 0, run: () => onCloseMany(all) },
      { id: "separator-pin", separator: true },
      ...(onKeep && document.preview ? [{ id: "keepOpen", label: t("tabs.menu.keepOpen"), run: () => onKeep(document.id) }] : []),
      ...(onPinnedChange ? [{ id: "pin", icon: "pin" as const, label: t(document.pinned ? "tabs.menu.unpin" : "tabs.menu.pin"), run: () => onPinnedChange(document.id, !document.pinned) }] : []),
      ...(extra.length > 0 ? [{ id: "separator-extra", separator: true } as const, ...extra] : []),
    ];
  };
  return (
    <div className="doc-tabs" role="tablist" aria-label={t("tabs.label")}>
      {documents.map((document, index) => {
        const active = document.id === activeDocumentId;
        const className = ["doc-tab", active ? "active" : "", document.preview ? "preview" : "", document.pinned ? "pinned" : "", document.dirty ? "dirty" : ""].filter(Boolean).join(" ");
        return (
          <RightClickMenu key={document.id} entries={() => menu(document, index)}>
            <div
              className={className}
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
                onDoubleClick={() => onKeep?.(document.id)}
                onAuxClick={(event) => { if (event.button === 1) { event.preventDefault(); if (closable(document)) onClose(document.id); } }}
              >
                <UiIcon icon={document.icon ?? "table2"} size="sm" className="doc-tab-icon" />
                <span className="doc-tab-label">{document.label}</span>
              </button>
              {document.pinned ? (
                <button className="doc-tab-close doc-tab-pin" type="button" aria-label={t("tabs.unpin", { label: document.label })} onClick={() => onPinnedChange?.(document.id, false)}>
                  <span className="doc-tab-dirty" aria-hidden="true" />
                  <UiIcon icon="pin" size="xs" className="doc-tab-x" />
                </button>
              ) : document.closable ? (
                <button className="doc-tab-close" type="button" aria-label={t(document.dirty ? "tabs.closeUnsaved" : "tabs.close", { label: document.label })} onClick={() => onClose(document.id)}>
                  <span className="doc-tab-dirty" aria-hidden="true" />
                  <UiIcon icon="x" size="xs" className="doc-tab-x" />
                </button>
              ) : null}
            </div>
          </RightClickMenu>
        );
      })}
      {documents.length === 0 ? <span className="doc-tabs-empty">{t("tabs.empty")}</span> : null}
    </div>
  );
}
