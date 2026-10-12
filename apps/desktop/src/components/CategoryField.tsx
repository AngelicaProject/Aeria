import { useEffect, useState } from "react";
import { Popover } from "radix-ui";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

type CategoryFieldProps = {
  id: string;
  /** The category now; empty while none is chosen. */
  value: string;
  /** The categories in use, offered to choose. */
  categories: readonly string[];
  disabled?: boolean;
  /** The category chosen or typed; never empty. */
  onCommit: (category: string) => void;
};

type Choice = { value: string; label: string; create: boolean };

/**
 * A category typed or chosen among those in use: the list follows what is
 * typed and offers to create a new one. A term always has a category, so
 * clearing the field keeps the one it had.
 */
export function CategoryField({ id, value, categories, disabled = false, onCommit }: CategoryFieldProps) {
  const { t } = useI18n();
  const [text, setText] = useState(value);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  useEffect(() => { setText(value); }, [value]);
  // The choice the arrow keys reach stays in view.
  useEffect(() => {
    if (open) document.getElementById(`${id}-choice-${active}`)?.scrollIntoView({ block: "nearest" });
  }, [active, id, open]);

  const typed = text.trim();
  const needle = typed.toLocaleLowerCase();
  const known = categories.some((category) => category.toLocaleLowerCase() === needle);
  const choices: Choice[] = [
    ...categories
      .filter((category) => needle === "" || known || category.toLocaleLowerCase().includes(needle))
      .map((category) => ({ value: category, label: category, create: false })),
    ...(typed !== "" && !known ? [{ value: typed, label: t("glossary.category.create", { name: typed }), create: true }] : []),
  ];

  const commit = (next: string) => {
    setOpen(false);
    if (next === "") { setText(value); return; }
    setText(next);
    if (next !== value) onCommit(next);
  };
  const show = () => { if (!disabled) { setActive(0); setOpen(true); } };

  return (
    <Popover.Root open={open && choices.length > 0} onOpenChange={setOpen}>
      <Popover.Anchor asChild>
        <div className="category-field">
          <input
            id={id}
            className="input"
            value={text}
            disabled={disabled}
            role="combobox"
            aria-expanded={open && choices.length > 0}
            aria-controls={`${id}-choices`}
            aria-autocomplete="list"
            autoComplete="off"
            spellCheck={false}
            onFocus={show}
            onClick={show}
            onChange={(event) => { setText(event.target.value); setActive(0); setOpen(true); }}
            onBlur={() => commit(typed)}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                event.preventDefault();
                if (!open) { show(); return; }
                if (choices.length === 0) return;
                const step = event.key === "ArrowDown" ? 1 : -1;
                setActive((current) => (current + step + choices.length) % choices.length);
              } else if (event.key === "Enter") {
                event.preventDefault();
                commit(open ? choices[active]?.value ?? typed : typed);
              } else if (event.key === "Escape" && open) {
                event.preventDefault();
                setText(value);
                setOpen(false);
              }
            }}
          />
          <UiIcon icon="chevronDown" size="xs" className="category-field-chevron" />
        </div>
      </Popover.Anchor>
      <Popover.Portal>
        <Popover.Content
          className="menu-content category-field-content"
          align="start"
          sideOffset={4}
          collisionPadding={8}
          onOpenAutoFocus={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => event.preventDefault()}
          // A modal dialog stops the wheel outside it, and the list is
          // portaled out of it: the list scrolls itself.
          onWheel={(event) => { event.currentTarget.scrollTop += event.deltaY; }}
        >
          <div id={`${id}-choices`} role="listbox">
            {choices.map((choice, index) => (
              <div
                key={`${choice.create ? "create:" : ""}${choice.value}`}
                id={`${id}-choice-${index}`}
                role="option"
                aria-selected={index === active}
                className={`menu-item category-field-item${index === active ? " active" : ""}`}
                // Chosen before the field's blur commits what is typed.
                onMouseDown={(event) => { event.preventDefault(); commit(choice.value); }}
                onMouseEnter={() => setActive(index)}
              >
                <span className="menu-item-check">
                  {choice.create ? <UiIcon icon="plus" size="xs" /> : choice.value.toLocaleLowerCase() === value.toLocaleLowerCase() ? <UiIcon icon="check" size="xs" /> : null}
                </span>
                <span>{choice.label}</span>
              </div>
            ))}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
