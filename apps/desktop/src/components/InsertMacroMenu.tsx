import { useState, type ReactNode } from "react";
import { ContextMenu, DropdownMenu } from "radix-ui";
import { macroInsertions } from "../ipc";
import type { MacroInsertionDto } from "../types";
import type { MessageKey, Translate } from "../i18n/translate";
import { useI18n } from "../ui/i18n";
import { IconButton } from "../ui/primitives/IconButton";
import { UiIcon } from "../ui/primitives/UiIcon";
import type { MacroEditorApi } from "./MacroEditor";
import type { ChipPick } from "./macroChipsExtension";
import { insertionPick } from "../macroInsertions";

/**
 * Macros a translator can insert into a translation: the player character's
 * name, class, and race, choices by gender, race, or class, and formatting.
 * The forms come from Rust (`aeria_se::catalog::INSERTIONS`), with the races
 * and classes of the open project's game.
 */

const GROUPS = ["player", "choice", "format"] as const;

function label(t: Translate, key: string, fallback: string): string {
  const text = t(key as MessageKey);
  return text === key ? fallback : text;
}

type MenuParts = typeof DropdownMenu | typeof ContextMenu;

/** The insertion items, grouped, in either kind of menu. */
function InsertionItems({ menu, insertions, onPick }: { menu: MenuParts; insertions: readonly MacroInsertionDto[] | null; onPick: (pick: ChipPick) => void }) {
  const { t } = useI18n();
  if (insertions === null) return <menu.Label className="menu-label">{t("common.loading")}</menu.Label>;
  const nodes: ReactNode[] = [];
  for (const group of GROUPS) {
    const items = insertions.filter((insertion) => insertion.group === group && (insertion.parts.every((part) => !part.includes("{row}")) || insertion.rows.length > 0));
    if (items.length === 0) continue;
    if (nodes.length > 0) nodes.push(<menu.Separator className="menu-separator" key={`${group}-separator`} />);
    nodes.push(<menu.Label className="menu-label" key={`${group}-label`}>{t(`insert.group.${group}` as MessageKey)}</menu.Label>);
    for (const insertion of items) {
      const text = label(t, `insert.${insertion.name}`, insertion.summary);
      if (insertion.rows.length === 0) {
        nodes.push(
          <menu.Item className="menu-item" key={insertion.name} title={insertion.parts.join("…")} onSelect={() => onPick(insertionPick(insertion))}>
            <span className="menu-item-check" />
            <span className="menu-item-label">{text}</span>
          </menu.Item>,
        );
        continue;
      }
      nodes.push(
        <menu.Sub key={insertion.name}>
          <menu.SubTrigger className="menu-item">
            <span className="menu-item-check" />
            <span className="menu-item-label">{text}</span>
            <UiIcon icon="chevronRight" size="xs" className="menu-item-chevron" />
          </menu.SubTrigger>
          <menu.Portal>
            <menu.SubContent className="menu-content menu-content-scroll" sideOffset={4} alignOffset={-5}>
              {insertion.rows.map((row) => (
                <menu.Item className="menu-item" key={row.row} title={insertion.parts.join("…").replaceAll("{row}", String(row.row))} onSelect={() => onPick(insertionPick(insertion, row.row))}>
                  <span className="menu-item-check" />
                  <span className="menu-item-label">{row.name}</span>
                </menu.Item>
              ))}
            </menu.SubContent>
          </menu.Portal>
        </menu.Sub>,
      );
    }
  }
  return <>{nodes}</>;
}

/** Reads the insertions each time a menu opens, so the game's rows are the open project's. */
function useInsertions(): [readonly MacroInsertionDto[] | null, (open: boolean) => void] {
  const [insertions, setInsertions] = useState<readonly MacroInsertionDto[] | null>(null);
  const load = (open: boolean) => {
    if (!open) return;
    macroInsertions().then(setInsertions, () => setInsertions([]));
  };
  return [insertions, load];
}

/** A button that opens the insertion menu for an editor. */
export function InsertMacroButton({ editor, disabled }: { editor: React.RefObject<MacroEditorApi | null>; disabled?: boolean }) {
  const { t } = useI18n();
  const [insertions, load] = useInsertions();
  return (
    <DropdownMenu.Root onOpenChange={load}>
      <DropdownMenu.Trigger asChild disabled={disabled ?? false}>
        <IconButton icon="plus" label={t("insert.menu")} disabled={disabled} />
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content className="menu-content" align="end" sideOffset={4}>
          <InsertionItems menu={DropdownMenu} insertions={insertions} onPick={(pick) => editor.current?.apply(pick)} />
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

/** The editor's context menu: the clipboard, then the insertions. */
export function InsertMacroContextMenu({ editor, disabled, children }: { editor: React.RefObject<MacroEditorApi | null>; disabled?: boolean; children: ReactNode }) {
  const { t } = useI18n();
  const [insertions, load] = useInsertions();
  const copy = async (cut: boolean) => {
    const selected = editor.current?.selection() ?? "";
    if (!selected) return;
    await navigator.clipboard?.writeText(selected);
    if (cut) editor.current?.apply({ insert: "" });
  };
  const paste = async () => {
    const text = await navigator.clipboard?.readText().catch(() => null);
    if (text) editor.current?.apply({ insert: text });
  };
  return (
    <ContextMenu.Root onOpenChange={load}>
      <ContextMenu.Trigger asChild disabled={disabled ?? false}>{children}</ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content className="menu-content">
          {([["cut", () => void copy(true)], ["copy", () => void copy(false)], ["paste", () => void paste()]] as const).map(([action, run]) => (
            <ContextMenu.Item className="menu-item" key={action} onSelect={run}>
              <span className="menu-item-check" />
              <span className="menu-item-label">{t(`insert.${action}` as MessageKey)}</span>
            </ContextMenu.Item>
          ))}
          <ContextMenu.Separator className="menu-separator" />
          <InsertionItems menu={ContextMenu} insertions={insertions} onPick={(pick) => editor.current?.apply(pick)} />
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}
