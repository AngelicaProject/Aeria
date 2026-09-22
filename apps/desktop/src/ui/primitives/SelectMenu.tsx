import { useEffect, useRef, useState } from "react";
import { UiIcon } from "./UiIcon";

export type SelectOption = { value: string; label: string; disabled?: boolean };

type SelectMenuProps = {
  value: string;
  options: readonly SelectOption[];
  onChange: (value: string) => void;
  label: string;
  disabled?: boolean;
};

export function SelectMenu({ value, options, onChange, label, disabled = false }: SelectMenuProps) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(() => Math.max(0, options.findIndex((option) => option.value === value)));
  const rootRef = useRef<HTMLDivElement>(null);
  const selected = options.find((option) => option.value === value) ?? options[0];

  useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    }
    window.addEventListener("pointerdown", handlePointerDown);
    return () => window.removeEventListener("pointerdown", handlePointerDown);
  }, []);

  function handleKeyDown(event: React.KeyboardEvent<HTMLButtonElement>) {
    if (event.key === "Escape") {
      setOpen(false);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setOpen(true);
      const direction = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((current) => (current + direction + options.length) % options.length);
    }
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (!open) setOpen(true);
      else {
        const option = options[activeIndex];
        if (option && !option.disabled) onChange(option.value);
        setOpen(false);
      }
    }
  }

  return (
    <div className={open ? "select-menu open" : "select-menu"} ref={rootRef}>
      <button
        className="select-menu-button"
        type="button"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={label}
        disabled={disabled}
        onClick={() => {
          setActiveIndex(Math.max(0, options.findIndex((option) => option.value === value)));
          setOpen((current) => !current);
        }}
        onKeyDown={handleKeyDown}
      >
        <span>{selected?.label ?? "Select"}</span>
        <UiIcon icon="chevronDown" size="xs" />
      </button>
      {open ? (
        <div className="select-menu-list" role="listbox" aria-label={label}>
          {options.map((option, index) => (
            <button
              className={option.value === value ? "select-option active" : index === activeIndex ? "select-option focused" : "select-option"}
              type="button"
              role="option"
              aria-selected={option.value === value}
              disabled={option.disabled}
              key={option.value}
              onMouseEnter={() => setActiveIndex(index)}
              onClick={() => {
                if (option.disabled) return;
                onChange(option.value);
                setOpen(false);
              }}
            >
              {option.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
