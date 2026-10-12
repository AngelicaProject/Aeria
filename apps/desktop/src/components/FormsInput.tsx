import { useState } from "react";
import { splitForms } from "../glossaryModel";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

/** Other forms of a term as chips, added by typing and removed one by one. */
export function FormsInput({ id, forms, onChange }: { id: string; forms: readonly string[]; onChange: (forms: string[]) => void }) {
  const { t } = useI18n();
  const [text, setText] = useState("");
  const commit = () => {
    const added = splitForms(text);
    if (added.length > 0) onChange([...forms, ...added]);
    setText("");
  };
  // A chip goes back into the field to be edited.
  const edit = (index: number) => {
    setText(forms[index] ?? "");
    onChange([...forms.filter((_, other) => other !== index), ...splitForms(text)]);
  };
  return (
    <div className="input glossary-forms" onMouseDown={(event) => { if (event.target === event.currentTarget) { event.preventDefault(); document.getElementById(id)?.focus(); } }}>
      {forms.map((form, index) => (
        <span key={index} className="glossary-form" title={t("glossary.form.edit")} onDoubleClick={() => edit(index)}>
          {form}
          <button type="button" className="glossary-form-remove" aria-label={t("glossary.form.remove", { form })} onClick={() => onChange(forms.filter((_, other) => other !== index))}><UiIcon icon="x" size="xs" /></button>
        </span>
      ))}
      <input
        id={id}
        className="glossary-form-input"
        value={text}
        placeholder={forms.length === 0 ? t("glossary.form.add") : undefined}
        onChange={(event) => setText(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === ";") {
            event.preventDefault();
            commit();
          } else if (event.key === "Backspace" && text === "" && forms.length > 0) {
            event.preventDefault();
            edit(forms.length - 1);
          }
        }}
      />
    </div>
  );
}
