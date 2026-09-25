import type { ReactNode } from "react";
import { Select as RadixSelect } from "radix-ui";
import { UiIcon } from "./UiIcon";

export type SelectOption<T extends string> = {
  value: T;
  label: ReactNode;
  /** A second, muted line under the label. */
  hint?: string;
  disabled?: boolean;
};

export type SelectGroup<T extends string> = {
  label?: string;
  options: readonly SelectOption<T>[];
};

type SelectProps<T extends string> = {
  value: T;
  onChange: (value: T) => void;
  /** Options, or `groups` for labelled sections. */
  options?: readonly SelectOption<T>[];
  groups?: readonly SelectGroup<T>[];
  /** The accessible name when no `<label htmlFor>` points at `id`. */
  label?: string;
  id?: string;
  disabled?: boolean;
  className?: string;
  /** `field` looks like an input; `quiet` like a text button. */
  variant?: "field" | "quiet";
  title?: string;
};

// Radix Select reserves the empty string, which Aeria uses for "none".
const EMPTY = "\u0000empty";
const encode = (value: string) => (value === "" ? EMPTY : value);
const decode = (value: string) => (value === EMPTY ? "" : value);

/** A themed single-choice dropdown replacing the native select element. */
export function Select<T extends string>({ value, onChange, options, groups, label, id, disabled = false, className, variant = "field", title }: SelectProps<T>) {
  const sections: readonly SelectGroup<T>[] = groups ?? [{ options: options ?? [] }];
  return (
    <RadixSelect.Root value={encode(value)} onValueChange={(next) => onChange(decode(next) as T)} disabled={disabled}>
      <RadixSelect.Trigger
        id={id}
        className={`select-trigger select-${variant}${className ? ` ${className}` : ""}`}
        aria-label={label}
        title={title}
      >
        <span className="select-value"><RadixSelect.Value /></span>
        <RadixSelect.Icon className="select-chevron"><UiIcon icon="chevronDown" size="xs" /></RadixSelect.Icon>
      </RadixSelect.Trigger>
      <RadixSelect.Portal>
        <RadixSelect.Content className="menu-content select-content" position="popper" sideOffset={4} collisionPadding={8}>
          <RadixSelect.Viewport className="select-viewport">
            {sections.map((section, index) => (
              <RadixSelect.Group key={section.label ?? index}>
                {index > 0 ? <RadixSelect.Separator className="menu-separator" /> : null}
                {section.label ? <RadixSelect.Label className="menu-label select-label">{section.label}</RadixSelect.Label> : null}
                {section.options.map((option) => (
                  <RadixSelect.Item key={option.value} value={encode(option.value)} disabled={option.disabled ?? false} className="menu-item select-item">
                    <span className="menu-item-check"><RadixSelect.ItemIndicator><UiIcon icon="check" size="xs" /></RadixSelect.ItemIndicator></span>
                    <span className="select-item-text">
                      <RadixSelect.ItemText>{option.label}</RadixSelect.ItemText>
                      {option.hint ? <span className="select-item-hint">{option.hint}</span> : null}
                    </span>
                  </RadixSelect.Item>
                ))}
              </RadixSelect.Group>
            ))}
          </RadixSelect.Viewport>
        </RadixSelect.Content>
      </RadixSelect.Portal>
    </RadixSelect.Root>
  );
}
