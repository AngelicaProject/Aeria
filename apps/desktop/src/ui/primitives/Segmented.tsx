import type { ReactNode } from "react";

export type SegmentedOption<T extends string> = {
  value: T;
  label: ReactNode;
  disabled?: boolean;
  title?: string;
  className?: string;
};

type SegmentedProps<T extends string> = {
  value: T | null;
  options: readonly SegmentedOption<T>[];
  onChange: (value: T) => void;
  label: string;
  disabled?: boolean;
  size?: "sm" | "md";
};

/** A single-choice button group; `null` means no option is current. */
export function Segmented<T extends string>({ value, options, onChange, label, disabled = false, size = "sm" }: SegmentedProps<T>) {
  return (
    <div className={`segmented segmented-${size}`} role="radiogroup" aria-label={label}>
      {options.map((option) => {
        const checked = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={checked}
            title={option.title}
            className={`segmented-option${checked ? " checked" : ""}${option.className ? ` ${option.className}` : ""}`}
            disabled={disabled || option.disabled}
            onClick={() => { if (!checked) onChange(option.value); }}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
