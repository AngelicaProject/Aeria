import { useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Popover } from "radix-ui";
import { describeIssue, issueLabel } from "../issueText";
import type { IssueCountDto } from "../types";
import { useI18n } from "../ui/i18n";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ScopeChip } from "./SearchScope";

type SearchIssuePickerProps = {
  /** The issues of every string found by group, most strings first. */
  issues: readonly IssueCountDto[];
  /** The chosen group, or null for any issue. */
  value: string | null;
  onChange: (group: string | null) => void;
};

type Item = { group: string | null; label: string; title: string; count: number | null };

/**
 * Which issue the result keeps, as one chip that opens a filterable list:
 * a project has hundreds of groups, one per term, so they are not laid out
 * as chips. Terms are listed apart from the other kinds.
 */
export function SearchIssuePicker({ issues, value, onChange }: SearchIssuePickerProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const listRef = useRef<HTMLDivElement | null>(null);

  const items = useMemo(() => issues.map(({ issue, count }): Item & { term: boolean } => ({
    group: issue.group,
    label: issueLabel(issue, t),
    title: describeIssue(issue, t),
    count,
    term: issue.term !== null && issue.term !== "",
  })), [issues, t]);
  const needle = filter.trim().toLocaleLowerCase();
  const shown = needle === "" ? items : items.filter((item) => item.label.toLocaleLowerCase().includes(needle) || item.title.toLocaleLowerCase().includes(needle));
  const sections = [
    { key: "other", label: t("search.issue.other"), items: shown.filter((item) => !item.term) },
    { key: "terms", label: t("search.issue.terms"), items: shown.filter((item) => item.term) },
  ].filter((section) => section.items.length > 0);
  const chosen = items.find((item) => item.group === value);

  const choose = (group: string | null) => {
    onChange(group);
    setOpen(false);
    setFilter("");
  };

  const move = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const buttons = [...(listRef.current?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    if (buttons.length === 0) return;
    event.preventDefault();
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === "ArrowDown" ? Math.min(at + 1, buttons.length - 1) : at - 1;
    if (next < 0) (event.currentTarget.querySelector("input") as HTMLInputElement | null)?.focus();
    else buttons[next]?.focus();
  };

  const row = (item: Item) => (
    <button key={item.group ?? ""} type="button" className={item.group === value ? "issue-item on" : "issue-item"} title={item.title} onClick={() => choose(item.group)}>
      <span className="issue-item-check">{item.group === value ? <UiIcon icon="check" size="xs" /> : null}</span>
      <span className="issue-item-label">{item.label}</span>
      {item.count !== null ? <span className="issue-item-count">{item.count}</span> : null}
    </button>
  );

  return (
    <Popover.Root open={open} onOpenChange={(next) => { setOpen(next); if (!next) setFilter(""); }}>
      <Popover.Trigger asChild>
        <ScopeChip label={t("search.issues")} value={chosen ? chosen.label : t("search.issue.any", { count: issues.length })} active={value !== null} />
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content className="menu-content issue-picker" align="start" sideOffset={4} collisionPadding={8} onKeyDown={move}>
          <label className="issue-picker-search">
            <UiIcon icon="search" size="xs" />
            <input
              className="input"
              autoFocus
              value={filter}
              placeholder={t("search.issue.filter")}
              aria-label={t("search.issue.filter")}
              spellCheck={false}
              onChange={(event) => setFilter(event.target.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                const first = sections[0]?.items[0];
                if (first) choose(first.group);
              }}
            />
          </label>
          <div className="issue-picker-list" ref={listRef}>
            {needle === "" ? row({ group: null, label: t("search.issue.any", { count: issues.length }), title: "", count: null }) : null}
            {sections.map((section) => (
              <div key={section.key} role="group" aria-label={section.label}>
                <div className="issue-picker-section">{section.label}</div>
                {section.items.map(row)}
              </div>
            ))}
            {sections.length === 0 ? <div className="issue-picker-empty">{t("search.none")}</div> : null}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
