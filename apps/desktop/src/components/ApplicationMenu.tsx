import { Menubar } from "radix-ui";
import { UiIcon } from "../ui/primitives/UiIcon";
import { useI18n } from "../ui/i18n";

export type ApplicationMenuCommandItem = {
  kind: "command";
  id: string;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  checked?: boolean;
  onSelect?: () => void;
};

export type ApplicationMenuRadioItem = {
  kind: "radio";
  id: string;
  label: string;
  value: string;
  items: readonly { value: string; label: string }[];
  onSelect: (value: string) => void;
};

export type ApplicationMenuSeparatorItem = {
  kind: "separator";
  id: string;
};

export type ApplicationMenuSubmenuItem = {
  kind: "submenu";
  id: string;
  label: string;
  disabled?: boolean;
  items: readonly ApplicationMenuItem[];
};

export type ApplicationMenuItem =
  | ApplicationMenuCommandItem
  | ApplicationMenuRadioItem
  | ApplicationMenuSeparatorItem
  | ApplicationMenuSubmenuItem;

export type ApplicationMenuDefinition = {
  id: string;
  label: string;
  items: readonly ApplicationMenuItem[];
};

function MenuItems({ items }: { items: readonly ApplicationMenuItem[] }) {
  return items.map((item) => {
    switch (item.kind) {
      case "separator":
        return <Menubar.Separator className="menu-separator" key={item.id} />;
      case "radio":
        return (
          <Menubar.RadioGroup value={item.value} onValueChange={item.onSelect} key={item.id}>
            <Menubar.Label className="menu-label">{item.label}</Menubar.Label>
            {item.items.map((option) => (
              <Menubar.RadioItem className="menu-item" value={option.value} key={option.value}>
                <span className="menu-item-check"><Menubar.ItemIndicator><UiIcon icon="check" size="xs" /></Menubar.ItemIndicator></span>
                <span className="menu-item-label">{option.label}</span>
              </Menubar.RadioItem>
            ))}
          </Menubar.RadioGroup>
        );
      case "submenu":
        return (
          <Menubar.Sub key={item.id}>
            <Menubar.SubTrigger className="menu-item" disabled={item.disabled ?? false}>
              <span className="menu-item-check" />
              <span className="menu-item-label">{item.label}</span>
              <UiIcon icon="chevronRight" size="xs" className="menu-item-chevron" />
            </Menubar.SubTrigger>
            <Menubar.Portal>
              <Menubar.SubContent className="menu-content" sideOffset={4} alignOffset={-5}>
                <MenuItems items={item.items} />
              </Menubar.SubContent>
            </Menubar.Portal>
          </Menubar.Sub>
        );
      case "command":
        if (item.checked !== undefined) {
          return (
            <Menubar.CheckboxItem className="menu-item" checked={item.checked} disabled={item.disabled || !item.onSelect} onSelect={() => item.onSelect?.()} key={item.id}>
              <span className="menu-item-check"><Menubar.ItemIndicator><UiIcon icon="check" size="xs" /></Menubar.ItemIndicator></span>
              <span className="menu-item-label">{item.label}</span>
              {item.shortcut ? <kbd className="menu-item-shortcut">{item.shortcut}</kbd> : null}
            </Menubar.CheckboxItem>
          );
        }
        return (
          <Menubar.Item className="menu-item" disabled={item.disabled || !item.onSelect} onSelect={() => item.onSelect?.()} key={item.id}>
            <span className="menu-item-check" />
            <span className="menu-item-label">{item.label}</span>
            {item.shortcut ? <kbd className="menu-item-shortcut">{item.shortcut}</kbd> : null}
          </Menubar.Item>
        );
    }
  });
}

export function ApplicationMenu({ menus }: { menus: readonly ApplicationMenuDefinition[] }) {
  const { t } = useI18n();
  return (
    <Menubar.Root className="app-menubar" aria-label={t("menu.label")}>
      {menus.map((menu) => (
        <Menubar.Menu key={menu.id}>
          <Menubar.Trigger className="app-menubar-trigger">{menu.label}</Menubar.Trigger>
          <Menubar.Portal>
            <Menubar.Content className="menu-content" align="start" sideOffset={4}>
              <MenuItems items={menu.items} />
            </Menubar.Content>
          </Menubar.Portal>
        </Menubar.Menu>
      ))}
    </Menubar.Root>
  );
}
