import { useEffect, useRef, useState } from "react";

export type ApplicationMenuItem = {
  id: string;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  onSelect?: () => void;
};

export type ApplicationMenuDefinition = {
  id: string;
  label: string;
  items: readonly ApplicationMenuItem[];
};

type ApplicationMenuProps = {
  menus: readonly ApplicationMenuDefinition[];
};

export function ApplicationMenu({ menus }: ApplicationMenuProps) {
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [activeIndex, setActiveIndex] = useState(0);
  const rootRef = useRef<HTMLElement>(null);

  useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) setOpenMenu(null);
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") setOpenMenu(null);
    }
    window.addEventListener("pointerdown", handlePointerDown);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);

  const currentMenu = menus.find((menu) => menu.id === openMenu);

  function handleMenuKeyDown(event: React.KeyboardEvent<HTMLButtonElement>, menuIndex: number) {
    const menu = menus[menuIndex];
    if (!menu) return;
    if (openMenu === menu.id && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((current) => (current + direction + menu.items.length) % menu.items.length);
      return;
    }
    if (openMenu === menu.id && event.key === "Enter") {
      event.preventDefault();
      const item = menu.items[activeIndex];
      if (item && !item.disabled) {
        item.onSelect?.();
        setOpenMenu(null);
      }
      return;
    }
    if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
      event.preventDefault();
      const direction = event.key === "ArrowRight" ? 1 : -1;
      const next = (menuIndex + direction + menus.length) % menus.length;
      const nextMenu = menus[next];
      if (!nextMenu) return;
      setOpenMenu(nextMenu.id);
      setActiveIndex(0);
    }
    if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      setOpenMenu(menu.id);
      setActiveIndex(0);
    }
  }

  function handlePopupKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (!currentMenu) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const direction = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((current) => (current + direction + currentMenu.items.length) % currentMenu.items.length);
    }
    if (event.key === "Enter") {
      event.preventDefault();
      const item = currentMenu.items[activeIndex];
      if (item && !item.disabled) {
        item.onSelect?.();
        setOpenMenu(null);
      }
    }
  }

  return (
    <nav className="application-menu" aria-label="Application menu" ref={rootRef}>
      {menus.map((menu, menuIndex) => (
        <div className="application-menu-group" key={menu.id}>
          <button
            className={openMenu === menu.id ? "application-menu-button open" : "application-menu-button"}
            type="button"
            aria-haspopup="menu"
            aria-expanded={openMenu === menu.id}
            onClick={() => {
              setOpenMenu((current) => current === menu.id ? null : menu.id);
              setActiveIndex(0);
            }}
            onKeyDown={(event) => handleMenuKeyDown(event, menuIndex)}
          >
            {menu.label}
          </button>
          {openMenu === menu.id ? (
            <div className="application-menu-popup" role="menu" onKeyDown={handlePopupKeyDown} tabIndex={-1}>
              {menu.items.map((item, itemIndex) => (
                <button
                  className={itemIndex === activeIndex ? "application-menu-entry focused" : "application-menu-entry"}
                  type="button"
                  role="menuitem"
                  disabled={item.disabled}
                  key={item.id}
                  onMouseEnter={() => setActiveIndex(itemIndex)}
                  onClick={() => {
                    item.onSelect?.();
                    setOpenMenu(null);
                  }}
                >
                  <span>{item.label}</span>
                  {item.shortcut ? <kbd>{item.shortcut}</kbd> : null}
                </button>
              ))}
            </div>
          ) : null}
        </div>
      ))}
    </nav>
  );
}
