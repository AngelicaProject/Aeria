import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { DropdownMenu, Popover } from "radix-ui";
import type { MessageKey } from "../i18n/translate";
import type { SearchCheck, SearchField, SearchState } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";

export const fieldLabels: Record<SearchField, MessageKey> = {
  translation: "search.field.translation",
  source: "search.field.source",
  note: "search.field.note",
  context: "search.field.context",
};

const stateLabels: Record<SearchState, MessageKey> = {
  untranslated: "search.state.untranslated",
  translated: "search.state.translated",
  fuzzy: "search.state.fuzzy",
};

const checkLabels: Record<SearchCheck, MessageKey> = {
  any: "search.check.any",
  problems: "search.check.problems",
  advice: "search.check.advice",
};

const checkHints: Partial<Record<SearchCheck, MessageKey>> = {
  problems: "search.check.problemsHint",
  advice: "search.check.adviceHint",
};

/** What a search looks in and which strings it keeps besides its text. */
export type Scope = {
  fields: readonly SearchField[];
  pathsText: string;
  nameSheetsOnly: boolean;
  states: readonly SearchState[];
  check: SearchCheck;
};

export const defaultFields: readonly SearchField[] = ["translation", "source"];

/** Whether the scope keeps fewer strings than a plain search does. */
export function scopeNarrowed(scope: Scope): boolean {
  return scope.pathsText.trim() !== "" || scope.nameSheetsOnly || scope.states.length > 0 || scope.check !== "any";
}

type SearchScopeProps = {
  scope: Scope;
  onChange: (change: Partial<Scope>) => void;
  /** Puts every filter back; the fields searched stay. */
  onClear: () => void;
  /** Chips that narrow the result further, after the checks. */
  children?: ReactNode;
};

function toggled<T>(list: readonly T[], value: T): T[] {
  return list.includes(value) ? list.filter((item) => item !== value) : [...list, value];
}

type ScopeChipProps = ButtonHTMLAttributes<HTMLButtonElement> & { label: string; value: string; active: boolean; warn?: boolean };

/** A chip that shows a filter's current value and opens its choices. */
export const ScopeChip = forwardRef<HTMLButtonElement, ScopeChipProps>(function ScopeChip({ label, value, active, warn, className, ...rest }, ref) {
  return (
    <button ref={ref} type="button" className={`scope-chip${active ? " active" : ""}${warn ? " warn" : ""}${className ? ` ${className}` : ""}`} aria-label={`${label}: ${value}`} {...rest}>
      <span className="scope-chip-value">{value}</span>
      <UiIcon icon="chevronDown" size="xs" />
    </button>
  );
});

function CheckItem({ checked, onChange, children }: { checked: boolean; onChange: () => void; children: ReactNode }) {
  return (
    <DropdownMenu.CheckboxItem className="menu-item" checked={checked} onCheckedChange={onChange} onSelect={(event) => event.preventDefault()}>
      <span className="menu-item-check">{checked ? <UiIcon icon="check" size="xs" /> : null}</span>
      <span className="menu-item-label">{children}</span>
    </DropdownMenu.CheckboxItem>
  );
}

/**
 * The scope of a search as a line of chips: where the text is looked for,
 * which sheets, which string states, and which checks. Each chip says what
 * it is set to, so the whole search reads at a glance; a chip that narrows
 * the search is marked.
 */
export function SearchScope({ scope, onChange, onClear, children }: SearchScopeProps) {
  const { t } = useI18n();
  const fieldsValue = scope.fields.length === 0
    ? t("search.where.none")
    : scope.fields.length === Object.keys(fieldLabels).length
      ? t("search.where.all")
      : scope.fields.map((field) => t(fieldLabels[field])).join(", ");
  const paths = scope.pathsText.split(/[,\s]+/).map((path) => path.trim()).filter(Boolean);
  const sheetsValue = scope.nameSheetsOnly ? t("search.nameSheets") : paths.length > 0 ? paths.join(", ") : t("search.sheets.all");
  const statesValue = scope.states.length === 0 ? t("search.states.any") : scope.states.map((state) => t(stateLabels[state])).join(", ");
  const fieldsDefault = scope.fields.length === defaultFields.length && defaultFields.every((field) => scope.fields.includes(field));

  return (
    <div className="search-scope" role="group" aria-label={t("search.scope")}>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger asChild>
          <ScopeChip label={t("search.fields")} value={fieldsValue} active={!fieldsDefault} warn={scope.fields.length === 0} />
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className="menu-content" align="start" sideOffset={4} collisionPadding={8}>
            <DropdownMenu.Label className="menu-label">{t("search.fields")}</DropdownMenu.Label>
            {(Object.keys(fieldLabels) as SearchField[]).map((field) => (
              <CheckItem key={field} checked={scope.fields.includes(field)} onChange={() => onChange({ fields: toggled(scope.fields, field) })}>
                {t(fieldLabels[field])}
              </CheckItem>
            ))}
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>

      <Popover.Root>
        <Popover.Trigger asChild>
          <ScopeChip label={t("search.paths")} value={sheetsValue} active={scope.nameSheetsOnly || paths.length > 0} />
        </Popover.Trigger>
        <Popover.Portal>
          <Popover.Content className="menu-content scope-sheets" align="start" sideOffset={4} collisionPadding={8}>
            <label className="field">
              <span className="field-label">{t("search.paths")}</span>
              <input
                className="input"
                autoFocus
                value={scope.nameSheetsOnly ? "" : scope.pathsText}
                placeholder={scope.nameSheetsOnly ? t("search.nameSheetsChosen") : t("search.pathsPlaceholder")}
                disabled={scope.nameSheetsOnly}
                spellCheck={false}
                onChange={(event) => onChange({ pathsText: event.target.value })}
              />
            </label>
            <label className="checkbox">
              <input type="checkbox" checked={scope.nameSheetsOnly} onChange={(event) => onChange({ nameSheetsOnly: event.target.checked })} />
              <UiIcon icon="bookMarked" size="xs" />{t("search.nameSheetsOnly")}
            </label>
          </Popover.Content>
        </Popover.Portal>
      </Popover.Root>

      <DropdownMenu.Root>
        <DropdownMenu.Trigger asChild>
          <ScopeChip label={t("search.states")} value={statesValue} active={scope.states.length > 0} />
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className="menu-content" align="start" sideOffset={4} collisionPadding={8}>
            <DropdownMenu.Label className="menu-label">{t("search.states")}</DropdownMenu.Label>
            {(Object.keys(stateLabels) as SearchState[]).map((state) => (
              <CheckItem key={state} checked={scope.states.includes(state)} onChange={() => onChange({ states: toggled(scope.states, state) })}>
                {t(stateLabels[state])}
              </CheckItem>
            ))}
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>

      <DropdownMenu.Root>
        <DropdownMenu.Trigger asChild>
          <ScopeChip label={t("search.check")} value={t(checkLabels[scope.check])} active={scope.check !== "any"} />
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className="menu-content" align="start" sideOffset={4} collisionPadding={8}>
            <DropdownMenu.Label className="menu-label">{t("search.check")}</DropdownMenu.Label>
            <DropdownMenu.RadioGroup value={scope.check} onValueChange={(value) => onChange({ check: value as SearchCheck })}>
              {(Object.keys(checkLabels) as SearchCheck[]).map((check) => {
                const hint = checkHints[check];
                return (
                  <DropdownMenu.RadioItem key={check} className="menu-item" value={check} title={hint ? t(hint) : undefined}>
                    <span className="menu-item-check">{scope.check === check ? <UiIcon icon="check" size="xs" /> : null}</span>
                    <span className="menu-item-label">{t(checkLabels[check])}</span>
                  </DropdownMenu.RadioItem>
                );
              })}
            </DropdownMenu.RadioGroup>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>

      {children}
      {scopeNarrowed(scope) ? (
        <button type="button" className="link-button scope-clear" onClick={onClear}>{t("search.clearFilters")}</button>
      ) : null}
    </div>
  );
}
