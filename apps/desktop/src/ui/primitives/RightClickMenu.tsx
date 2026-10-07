import { useRef, type ReactNode } from "react";
import { ContextMenu } from "radix-ui";
import { UiIcon, type UiIconName } from "./UiIcon";

/** One line of a right-click menu: an action, a separator, or a submenu. */
export type MenuEntry =
  | { id: string; label: string; icon?: UiIconName; shortcut?: string; disabled?: boolean; danger?: boolean; run: () => void }
  | { id: string; separator: true }
  | { id: string; label: string; icon?: UiIconName; disabled?: boolean; items: readonly MenuEntry[] };

/** Writes text to the clipboard. */
export function copyText(text: string): void {
  void navigator.clipboard?.writeText(text);
}

function Entries({ entries, onPick }: { entries: readonly MenuEntry[]; onPick: (run: () => void) => void }) {
  return entries.map((entry) => {
    if ("separator" in entry) return <ContextMenu.Separator className="menu-separator" key={entry.id} />;
    const head = <>
      <span className="menu-item-check">{entry.icon ? <UiIcon icon={entry.icon} size="xs" /> : null}</span>
      <span className="menu-item-label">{entry.label}</span>
    </>;
    if ("items" in entry) {
      return (
        <ContextMenu.Sub key={entry.id}>
          <ContextMenu.SubTrigger className="menu-item" disabled={entry.disabled ?? false}>
            {head}
            <UiIcon icon="chevronRight" size="xs" className="menu-item-chevron" />
          </ContextMenu.SubTrigger>
          <ContextMenu.Portal>
            <ContextMenu.SubContent className="menu-content menu-content-scroll" sideOffset={4} alignOffset={-5}>
              <Entries entries={entry.items} onPick={onPick} />
            </ContextMenu.SubContent>
          </ContextMenu.Portal>
        </ContextMenu.Sub>
      );
    }
    return (
      <ContextMenu.Item className={entry.danger ? "menu-item danger" : "menu-item"} key={entry.id} disabled={entry.disabled ?? false} onSelect={() => onPick(entry.run)}>
        {head}
        {entry.shortcut ? <kbd className="menu-item-shortcut">{entry.shortcut}</kbd> : null}
      </ContextMenu.Item>
    );
  });
}

/**
 * A right-click menu around `children`. The entries are read when the menu
 * opens, so a long list does not build a menu for every row it draws.
 *
 * The chosen action runs once the menu has closed and let go of the focus,
 * so an action that focuses something, such as a name to edit, keeps it.
 */
export function RightClickMenu({ entries, disabled, children }: { entries: () => readonly MenuEntry[]; disabled?: boolean | undefined; children: ReactNode }) {
  const pending = useRef<(() => void) | null>(null);
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger asChild disabled={disabled ?? false}>{children}</ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content
          className="menu-content"
          onCloseAutoFocus={() => {
            const run = pending.current;
            pending.current = null;
            run?.();
          }}
        >
          <OpenEntries entries={entries} onPick={(run) => { pending.current = run; }} />
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}

/** Mounted only while the menu is open, which is when it reads the entries. */
function OpenEntries({ entries, onPick }: { entries: () => readonly MenuEntry[]; onPick: (run: () => void) => void }) {
  return <Entries entries={entries()} onPick={onPick} />;
}
