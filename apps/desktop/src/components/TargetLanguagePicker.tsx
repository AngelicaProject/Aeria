import { useState } from "react";
import { commonTargetLanguages, isTargetLanguage, languageName } from "../targetLanguages";
import { useI18n } from "../ui/i18n";
import { Select } from "../ui/primitives/Select";

const OTHER = "__other";
/** The placeholder while no language is chosen; Radix reserves the empty value. */
const NONE = "__none";

/**
 * Chooses a project's target language: a common language from the list,
 * or any BCP 47 tag typed after "Other…". `value` is the chosen tag, or
 * null while none is chosen. With `commitTyped`, a typed tag is reported
 * on Enter or when the field loses focus instead of on every keystroke.
 */
export function TargetLanguagePicker({ id, value, onChange, disabled = false, commitTyped = false }: {
  id?: string;
  value: string | null;
  onChange: (tag: string | null) => void;
  disabled?: boolean;
  commitTyped?: boolean;
}) {
  const { t } = useI18n();
  const listed = value !== null && commonTargetLanguages.includes(value);
  const [typing, setTyping] = useState(value !== null && !listed);
  const [typed, setTyped] = useState(value !== null && !listed ? value : "");
  const options = [
    ...(value === null && !typing ? [{ value: NONE, label: t("targetLanguage.choose") }] : []),
    ...commonTargetLanguages.map((tag) => ({ value: tag, label: languageName(tag) })),
    { value: OTHER, label: t("targetLanguage.other") },
  ];
  const typedValid = isTargetLanguage(typed.trim());
  return (
    <div className="target-language-picker">
      <Select<string>
        {...(id ? { id } : {})}
        label={t("targetLanguage.label")}
        value={typing ? OTHER : value ?? NONE}
        disabled={disabled}
        onChange={(chosen) => {
          if (chosen === OTHER) {
            setTyping(true);
            onChange(typedValid ? typed.trim() : null);
          } else if (chosen !== NONE) {
            setTyping(false);
            onChange(chosen);
          }
        }}
        options={options}
      />
      {typing ? (
        <input
          className={`input mono${typed && !typedValid ? " is-invalid" : ""}`}
          aria-label={t("targetLanguage.tag")}
          placeholder="pt-BR"
          value={typed}
          disabled={disabled}
          spellCheck={false}
          autoComplete="off"
          onChange={(event) => {
            setTyped(event.target.value);
            if (commitTyped) return;
            const tag = event.target.value.trim();
            onChange(isTargetLanguage(tag) ? tag : null);
          }}
          onBlur={() => {
            if (commitTyped && typedValid) onChange(typed.trim());
          }}
          onKeyDown={(event) => {
            if (commitTyped && event.key === "Enter" && typedValid) onChange(typed.trim());
          }}
        />
      ) : null}
    </div>
  );
}
