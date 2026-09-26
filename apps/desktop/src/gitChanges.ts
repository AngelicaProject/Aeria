import type { SourceBinding, UnitChangeDto } from "./types";

export type ChangeKind = UnitChangeDto["kind"];

/** What the change list shows: text to find and one kind, or every kind. */
export type ChangeFilter = { query: string; kind: ChangeKind | "all" };

export type ChangeGroup = {
  sheetName: string;
  changes: UnitChangeDto[];
  /** Whether the group's changes span more than one column, so rows name theirs. */
  multiColumn: boolean;
};

/** One line of the flattened list: a sheet header or a change under it. */
export type ChangeItem =
  | { type: "group"; group: ChangeGroup; shown: number; closed: boolean }
  | { type: "change"; change: UnitChangeDto; group: ChangeGroup };

/** Changes above which sheets start collapsed, so the list stays scannable. */
export const COLLAPSE_THRESHOLD = 50;

export function changeBinding(change: UnitChangeDto): SourceBinding | null {
  return (change.after ?? change.before)?.sourceBinding ?? null;
}

/** Groups changes by sheet, ordered by sheet name then row coordinate. */
export function groupChanges(changes: readonly UnitChangeDto[], unknownSheet: string): ChangeGroup[] {
  const groups = new Map<string, UnitChangeDto[]>();
  for (const change of changes) {
    const sheetName = changeBinding(change)?.sheetName ?? unknownSheet;
    const group = groups.get(sheetName);
    if (group) group.push(change);
    else groups.set(sheetName, [change]);
  }
  const order = (change: UnitChangeDto) => {
    const binding = changeBinding(change);
    return binding ? [binding.rowId, binding.subrowId, binding.columnIndex] : [0, 0, 0];
  };
  return [...groups].sort(([left], [right]) => left.localeCompare(right)).map(([sheetName, entries]) => {
    const sorted = entries.sort((left, right) => {
      const [a, b] = [order(left), order(right)];
      return a[0]! - b[0]! || a[1]! - b[1]! || a[2]! - b[2]!;
    });
    const columns = new Set(sorted.map((change) => changeBinding(change)?.columnIndex ?? 0));
    return { sheetName, changes: sorted, multiColumn: columns.size > 1 };
  });
}

export function isFilterActive(filter: ChangeFilter): boolean {
  return filter.kind !== "all" || filter.query.trim() !== "";
}

/** Whether a change matches the filter by kind and by its sheet, text, or
 * exact `row` / `row:subrow`. */
export function matchesChange(change: UnitChangeDto, sheetName: string, filter: ChangeFilter): boolean {
  if (filter.kind !== "all" && change.kind !== filter.kind) return false;
  const query = filter.query.trim().toLocaleLowerCase();
  if (query === "") return true;
  const binding = changeBinding(change);
  // `12` or `12:0` names a row exactly rather than every row containing it.
  const coordinate = /^(\d+)(?::(\d+))?$/.exec(query);
  if (coordinate && binding) {
    return binding.rowId === Number(coordinate[1]) && (coordinate[2] === undefined || binding.subrowId === Number(coordinate[2]));
  }
  const haystack = [sheetName, change.after?.targetMacro ?? "", change.before?.targetMacro ?? ""];
  return haystack.some((text) => text.toLocaleLowerCase().includes(query));
}

export function kindCounts(changes: readonly UnitChangeDto[]): Record<ChangeKind, number> {
  const counts: Record<ChangeKind, number> = { added: 0, modified: 0, removed: 0 };
  for (const change of changes) counts[change.kind] += 1;
  return counts;
}

/**
 * The visible lines: each sheet with matching changes, then its changes
 * unless it is closed. While a filter is active every matching sheet is open,
 * since the user is looking for something inside them.
 */
export function flattenChanges(groups: readonly ChangeGroup[], filter: ChangeFilter, isClosed: (sheetName: string) => boolean): ChangeItem[] {
  const active = isFilterActive(filter);
  const items: ChangeItem[] = [];
  for (const group of groups) {
    const matching = active ? group.changes.filter((change) => matchesChange(change, group.sheetName, filter)) : group.changes;
    if (matching.length === 0) continue;
    const closed = !active && isClosed(group.sheetName);
    items.push({ type: "group", group, shown: matching.length, closed });
    if (!closed) for (const change of matching) items.push({ type: "change", change, group });
  }
  return items;
}
