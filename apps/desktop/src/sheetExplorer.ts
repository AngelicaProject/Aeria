import type { ProjectSheetDto } from "./types";

export type SheetTreeLeaf = {
  kind: "leaf";
  sheet: ProjectSheetDto;
  name: string;
  parentPath: string;
};

export type SheetTreeFolder = {
  kind: "folder";
  name: string;
  path: string;
  children: SheetTreeEntry[];
  descendantCount: number;
};

export type SheetTreeEntry = SheetTreeLeaf | SheetTreeFolder;

export type SheetTree = {
  children: SheetTreeEntry[];
  descendantCount: number;
};

export type SheetMatch = {
  sheet: ProjectSheetDto;
  basename: string;
  breadcrumb: string;
};

const naturalCollator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: "base",
});

export function formatSheetCount(count: number): string {
  return count.toLocaleString("fr-FR").replace(/\u202f/g, " ");
}

function compareNames(left: string, right: string): number {
  return naturalCollator.compare(left, right);
}

function basename(sheetName: string): string {
  const separator = sheetName.lastIndexOf("/");
  return separator < 0 ? sheetName : sheetName.slice(separator + 1);
}

function parentPath(sheetName: string): string {
  const separator = sheetName.lastIndexOf("/");
  return separator < 0 ? "" : sheetName.slice(0, separator);
}

function sortEntries(entries: SheetTreeEntry[]): SheetTreeEntry[] {
  return entries.sort((left, right) => {
    const leftName = left.name;
    const rightName = right.name;
    const kindOrder = left.kind === right.kind ? 0 : left.kind === "folder" ? -1 : 1;
    return compareNames(leftName, rightName) || kindOrder;
  });
}

export function buildSheetTree(sheets: readonly ProjectSheetDto[]): SheetTree {
  const root: SheetTree = { children: [], descendantCount: sheets.length };

  for (const sheet of sheets) {
    const parts = sheet.name.split("/").filter(Boolean);
    if (parts.length === 0) continue;

    let children = root.children;
    let path = "";
    for (let index = 0; index < parts.length - 1; index += 1) {
      const part = parts[index]!;
      path = path ? `${path}/${part}` : part;
      let folder = children.find(
        (entry): entry is SheetTreeFolder => entry.kind === "folder" && entry.path === path,
      );
      if (!folder) {
        folder = { kind: "folder", name: part, path, children: [], descendantCount: 0 };
        children.push(folder);
      }
      folder.descendantCount += 1;
      children = folder.children;
    }

    children.push({
      kind: "leaf",
      sheet,
      name: parts[parts.length - 1]!,
      parentPath: parentPath(sheet.name),
    });
  }

  function sortFolder(folder: SheetTreeFolder): void {
    sortEntries(folder.children);
    for (const child of folder.children) {
      if (child.kind === "folder") sortFolder(child);
    }
  }

  sortEntries(root.children);
  for (const child of root.children) {
    if (child.kind === "folder") sortFolder(child);
  }
  return root;
}

export function visibleSheetTreeEntries(tree: SheetTree, expandedFolders: ReadonlySet<string>): SheetTreeEntry[] {
  const visible: SheetTreeEntry[] = [];

  function visit(entries: readonly SheetTreeEntry[]): void {
    for (const entry of entries) {
      visible.push(entry);
      if (entry.kind === "folder" && expandedFolders.has(entry.path)) {
        visit(entry.children);
      }
    }
  }

  visit(tree.children);
  return visible;
}

export function expandSheetAncestors(
  expandedFolders: ReadonlySet<string>,
  sheetName: string,
): Set<string> {
  const next = new Set(expandedFolders);
  const parts = sheetName.split("/");
  let path = "";
  for (let index = 0; index < parts.length - 1; index += 1) {
    path = path ? `${path}/${parts[index]!}` : parts[index]!;
    next.add(path);
  }
  return next;
}

export function findSheetMatches(
  sheets: readonly ProjectSheetDto[],
  query: string,
): SheetMatch[] {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return [];

  return sheets
    .filter((sheet) => sheet.name.toLocaleLowerCase().includes(normalized))
    .sort((left, right) => compareNames(left.name, right.name))
    .map((sheet) => ({
      sheet,
      basename: basename(sheet.name),
      breadcrumb: parentPath(sheet.name).replaceAll("/", " › "),
    }));
}

export function collapseSheetTree(): Set<string> {
  return new Set();
}
