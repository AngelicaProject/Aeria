export type PaletteMode = "sheets" | "commands" | "goto" | "strings" | "help";

export type PaletteQuery = { mode: PaletteMode; term: string };

export const palettePrefixes: ReadonlyArray<{ prefix: string; mode: PaletteMode; label: string }> = [
  { prefix: ">", mode: "commands", label: "Run a command" },
  { prefix: ":", mode: "goto", label: "Go to a row in the current sheet" },
  { prefix: "#", mode: "strings", label: "Search strings in the project" },
  { prefix: "?", mode: "help", label: "Show available prefixes" },
];

/** Splits palette input into its mode prefix and search term. */
export function parsePaletteQuery(input: string): PaletteQuery {
  const entry = palettePrefixes.find((candidate) => input.startsWith(candidate.prefix));
  return entry ? { mode: entry.mode, term: input.slice(entry.prefix.length).trimStart() } : { mode: "sheets", term: input };
}

export type RowTarget = { rowId: number; subrowId: number; columnIndex: number | null };

/** Parses `row`, `row:subrow`, or `row:subrow:column`; returns null when invalid. */
export function parseRowTarget(term: string): RowTarget | null {
  const match = /^\s*(\d{1,10})(?:[:.](\d{1,5}))?(?:[:.](\d{1,5}))?\s*$/.exec(term);
  if (!match) return null;
  const rowId = Number(match[1]);
  const subrowId = match[2] === undefined ? 0 : Number(match[2]);
  const columnIndex = match[3] === undefined ? null : Number(match[3]);
  if (rowId > 0xffffffff || subrowId > 0xffff) return null;
  return { rowId, subrowId, columnIndex };
}
