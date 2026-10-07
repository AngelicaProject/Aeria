import { useRef, useState, type ReactNode } from "react";
import { ContextMenu } from "radix-ui";
import { useI18n } from "./i18n";
import { UiIcon, type UiIconName } from "./primitives/UiIcon";

/** Input types that hold text a person can edit. */
const TEXT_INPUTS = new Set(["text", "search", "url", "email", "tel", "password", "number"]);

/** What was right-clicked: an editable field, or text selected elsewhere. */
type TextTarget = {
  /** The field or editable element; `null` for text selected outside one. */
  element: HTMLElement | null;
  /** The selection inside an input or a text area. */
  start: number | null;
  end: number | null;
  /** The selection inside an editable element. */
  range: Range | null;
  selected: string;
  writable: boolean;
  /** A password is never copied out. */
  secret: boolean;
};

function textTarget(target: EventTarget | null): TextTarget | null {
  const element = target instanceof Element ? target : null;
  const field = element?.closest("input, textarea");
  if (field instanceof HTMLTextAreaElement || (field instanceof HTMLInputElement && TEXT_INPUTS.has(field.type))) {
    // A number field has no selection to read.
    let start: number | null = null;
    let end: number | null = null;
    try { start = field.selectionStart; end = field.selectionEnd; } catch { /* no selection */ }
    const selected = start !== null && end !== null ? field.value.slice(start, end) : "";
    return { element: field, start, end, range: null, selected, writable: !field.readOnly && !field.disabled, secret: field.type === "password" };
  }
  const selection = window.getSelection();
  const selected = selection?.toString() ?? "";
  const editable = element?.closest<HTMLElement>("[contenteditable='true']");
  if (editable) {
    const range = selection && selection.rangeCount > 0 && editable.contains(selection.anchorNode) ? selection.getRangeAt(0).cloneRange() : null;
    return { element: editable, start: null, end: null, range, selected: range ? selected : "", writable: true, secret: false };
  }
  // Text selected somewhere else is not what was right-clicked.
  const here = element !== null && selection !== null && selection.rangeCount > 0 && selection.getRangeAt(0).intersectsNode(element);
  return here && selected.trim() !== "" ? { element: null, start: null, end: null, range: null, selected, writable: false, secret: false } : null;
}

/** Gives the field its focus and selection back once the menu has closed. */
function restore(target: TextTarget) {
  const { element } = target;
  if (!element) return;
  element.focus({ preventScroll: true });
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    if (target.start !== null && target.end !== null) {
      try { element.setSelectionRange(target.start, target.end); } catch { /* no selection */ }
    }
  } else if (target.range) {
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(target.range);
  }
}

/**
 * Replaces the web view's own right-click menu everywhere in the window.
 * A text field, or selected text, gets the clipboard; a place with a menu of
 * its own keeps it; anywhere else a right-click does nothing.
 *
 * The edits go through the browser's editing commands, so the field's
 * change handlers and its undo history see them as typing.
 */
export function TextContextMenu({ children }: { children: ReactNode }) {
  const { t } = useI18n();
  const [target, setTarget] = useState<TextTarget | null>(null);
  // The chosen action runs after the menu has given the focus back.
  const pending = useRef<((target: TextTarget) => void) | null>(null);

  const item = (id: string, icon: UiIconName, label: string, shortcut: string, enabled: boolean, run: (target: TextTarget) => void) => (
    <ContextMenu.Item className="menu-item" key={id} disabled={!enabled} onSelect={() => { pending.current = run; }}>
      <span className="menu-item-check"><UiIcon icon={icon} size="xs" /></span>
      <span className="menu-item-label">{label}</span>
      <kbd className="menu-item-shortcut">{shortcut}</kbd>
    </ContextMenu.Item>
  );

  const copyable = target !== null && target.selected !== "" && !target.secret;
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger asChild>
        <div
          className="text-menu-scope"
          onContextMenu={(event) => {
            // A menu of its own already took the right-click.
            if (event.defaultPrevented) return;
            const found = textTarget(event.target);
            if (!found) event.preventDefault();
            else setTarget(found);
          }}
        >
          {children}
        </div>
      </ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content
          className="menu-content"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const run = pending.current;
            pending.current = null;
            if (!target) return;
            restore(target);
            run?.(target);
          }}
        >
          {target?.element ? <>
            {item("cut", "scissors", t("edit.cut"), "Ctrl+X", copyable && target.writable, (found) => {
              void navigator.clipboard?.writeText(found.selected);
              document.execCommand("delete");
            })}
            {item("copy", "copy", t("edit.copy"), "Ctrl+C", copyable, (found) => void navigator.clipboard?.writeText(found.selected))}
            {item("paste", "clipboard", t("edit.paste"), "Ctrl+V", target.writable, () => {
              void navigator.clipboard?.readText().then((text) => { if (text) document.execCommand("insertText", false, text); }, () => undefined);
            })}
            <ContextMenu.Separator className="menu-separator" />
            {item("selectAll", "textSelect", t("edit.selectAll"), "Ctrl+A", true, (found) => {
              if (found.element instanceof HTMLInputElement || found.element instanceof HTMLTextAreaElement) found.element.select();
              else document.execCommand("selectAll");
            })}
          </> : item("copy", "copy", t("edit.copy"), "Ctrl+C", copyable, (found) => void navigator.clipboard?.writeText(found.selected))}
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}
