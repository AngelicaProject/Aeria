import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Icon } from "../ui/primitives/Icon";

export type ApplicationMenuCommandItem = {
  kind: "command";
  id: string;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  checked?: boolean;
  radioGroup?: string;
  onSelect?: () => void;
};

export type ApplicationMenuSeparatorItem = {
  kind: "separator";
  id: string;
};

export type ApplicationMenuSubmenuItem = {
  kind: "submenu";
  id: string;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  items: readonly ApplicationMenuItem[];
};

export type ApplicationMenuItem =
  | ApplicationMenuCommandItem
  | ApplicationMenuSeparatorItem
  | ApplicationMenuSubmenuItem;

export type ApplicationMenuDefinition = {
  id: string;
  label: string;
  items: readonly ApplicationMenuItem[];
};

type ApplicationMenuProps = {
  menus: readonly ApplicationMenuDefinition[];
};

function isSelectable(item: ApplicationMenuItem): item is ApplicationMenuCommandItem | ApplicationMenuSubmenuItem {
  return item.kind !== "separator" && !item.disabled;
}

function nextSelectableIndex(items: readonly ApplicationMenuItem[], current: number, direction: number): number {
  if (items.length === 0) return -1;
  for (let offset = 1; offset <= items.length; offset += 1) {
    const index = (current + direction * offset + items.length) % items.length;
    if (items[index] && isSelectable(items[index])) return index;
  }
  return current;
}

type MenuPopupProps = {
  items: readonly ApplicationMenuItem[];
  level: number;
  activeIndices: readonly number[];
  openSubmenuPath: readonly string[];
  setActiveIndex: (level: number, index: number) => void;
  openSubmenu: (level: number, itemId: string) => void;
  closeSubmenusFrom: (level: number) => void;
  onSelect: (item: ApplicationMenuCommandItem) => void;
  onClose: () => void;
  onRootNavigate: (direction: number) => void;
};

function MenuPopup({
  items,
  level,
  activeIndices,
  openSubmenuPath,
  setActiveIndex,
  openSubmenu,
  closeSubmenusFrom,
  onSelect,
  onClose,
  onRootNavigate,
}: MenuPopupProps) {
  const popupRef = useRef<HTMLDivElement>(null);
  const activeIndex = activeIndices[level] ?? 0;
  const isNested = level > 0;

  useEffect(() => {
    popupRef.current?.focus();
  }, []);

  function activate(index: number) {
    setActiveIndex(level, index);
    const item = items[index];
    if (item?.kind === "submenu" && !item.disabled) openSubmenu(level, item.id);
    else closeSubmenusFrom(level);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      activate(nextSelectableIndex(items, activeIndex, event.key === "ArrowDown" ? 1 : -1));
      return;
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      const item = items[activeIndex];
      if (item?.kind === "command" && !item.disabled) onSelect(item);
      else if (item?.kind === "submenu" && !item.disabled) openSubmenu(level, item.id);
      return;
    }
    if (event.key === "ArrowRight") {
      event.preventDefault();
      const item = items[activeIndex];
      if (item?.kind === "submenu" && !item.disabled) openSubmenu(level, item.id);
      else if (!isNested) onRootNavigate(1);
      return;
    }
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (isNested) closeSubmenusFrom(level - 1);
      else onRootNavigate(-1);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  }

  return (
    <div
      ref={popupRef}
      className={isNested ? "application-menu-popup application-menu-submenu-popup" : "application-menu-popup"}
      role="menu"
      tabIndex={-1}
      onKeyDown={handleKeyDown}
    >
      {items.map((item, index) => {
        if (item.kind === "separator") {
          return <div className="application-menu-separator" role="separator" key={item.id} />;
        }
        const focused = activeIndex === index;
        const submenuOpen = item.kind === "submenu" && openSubmenuPath[level] === item.id;
        if (item.kind === "submenu") {
          return (
            <div className="application-menu-submenu" key={item.id}>
              <button
                className={focused ? "application-menu-entry focused" : "application-menu-entry"}
                type="button"
                role="menuitem"
                aria-haspopup="menu"
                aria-expanded={submenuOpen}
                disabled={item.disabled}
                onMouseEnter={() => activate(index)}
                onClick={() => openSubmenu(level, item.id)}
              >
                <span>{item.label}</span>
                <span className="application-menu-entry-end">{item.shortcut ? <kbd>{item.shortcut}</kbd> : null}<Icon name="chevron" size={12} /></span>
              </button>
              {submenuOpen ? <MenuPopup {...{ items: item.items, level: level + 1, activeIndices, openSubmenuPath, setActiveIndex, openSubmenu, closeSubmenusFrom, onSelect, onClose, onRootNavigate }} /> : null}
            </div>
          );
        }
        const role = item.radioGroup ? "menuitemradio" : item.checked === undefined ? "menuitem" : "menuitemcheckbox";
        return (
          <button
            className={focused ? "application-menu-entry focused" : "application-menu-entry"}
            type="button"
            role={role}
            aria-checked={item.checked}
            disabled={item.disabled}
            key={item.id}
            onMouseEnter={() => activate(index)}
            onClick={() => onSelect(item)}
          >
            <span>{item.label}</span>
            {item.shortcut ? <kbd>{item.shortcut}</kbd> : null}
          </button>
        );
      })}
    </div>
  );
}

export function ApplicationMenu({ menus }: ApplicationMenuProps) {
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [activeIndices, setActiveIndices] = useState<number[]>([0]);
  const [openSubmenuPath, setOpenSubmenuPath] = useState<string[]>([]);
  const rootRef = useRef<HTMLElement>(null);

  useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) setOpenMenu(null);
    }
    window.addEventListener("pointerdown", handlePointerDown);
    return () => window.removeEventListener("pointerdown", handlePointerDown);
  }, []);

  function setActiveIndex(level: number, index: number) {
    setActiveIndices((current) => {
      const next = current.slice(0, level);
      next[level] = index;
      return next;
    });
  }

  function closeSubmenusFrom(level: number) {
    setOpenSubmenuPath((current) => current.slice(0, level));
  }

  function openSubmenu(level: number, itemId: string) {
    setOpenSubmenuPath((current) => [...current.slice(0, level), itemId]);
    setActiveIndex(level + 1, 0);
  }

  function openRootMenu(menuId: string) {
    setOpenMenu(menuId);
    setActiveIndices([0]);
    setOpenSubmenuPath([]);
  }

  function navigateRoot(direction: number) {
    const currentIndex = Math.max(0, menus.findIndex((menu) => menu.id === openMenu));
    const nextIndex = (currentIndex + direction + menus.length) % menus.length;
    const next = menus[nextIndex];
    if (next) openRootMenu(next.id);
  }

  function handleMenuKeyDown(event: KeyboardEvent<HTMLButtonElement>, menuIndex: number) {
    const menu = menus[menuIndex];
    if (!menu) return;
    if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
      event.preventDefault();
      const next = (menuIndex + (event.key === "ArrowRight" ? 1 : -1) + menus.length) % menus.length;
      if (menus[next]) openRootMenu(menus[next].id);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openRootMenu(menu.id);
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setOpenMenu(null);
    }
  }

  const currentMenu = menus.find((menu) => menu.id === openMenu);

  return (
    <nav className="application-menu" aria-label="Application menu" ref={rootRef}>
      {menus.map((menu, menuIndex) => (
        <div className="application-menu-group" key={menu.id}>
          <button
            className={openMenu === menu.id ? "application-menu-button open" : "application-menu-button"}
            type="button"
            aria-haspopup="menu"
            aria-expanded={openMenu === menu.id}
            onMouseEnter={() => { if (openMenu) openRootMenu(menu.id); }}
            onClick={() => openMenu === menu.id ? setOpenMenu(null) : openRootMenu(menu.id)}
            onKeyDown={(event) => handleMenuKeyDown(event, menuIndex)}
          >
            {menu.label}
          </button>
          {currentMenu?.id === menu.id ? (
            <MenuPopup
              items={menu.items}
              level={0}
              activeIndices={activeIndices}
              openSubmenuPath={openSubmenuPath}
              setActiveIndex={setActiveIndex}
              openSubmenu={openSubmenu}
              closeSubmenusFrom={closeSubmenusFrom}
              onSelect={(item) => { item.onSelect?.(); setOpenMenu(null); }}
              onClose={() => setOpenMenu(null)}
              onRootNavigate={navigateRoot}
            />
          ) : null}
        </div>
      ))}
    </nav>
  );
}
