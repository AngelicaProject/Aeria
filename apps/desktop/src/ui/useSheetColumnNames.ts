import { useEffect, useState } from "react";
import { sheetColumnNames } from "../ipc";
import type { SheetColumnNamesDto } from "../types";

/**
 * The EXDSchema names of a sheet's columns, read once per sheet. `null`
 * while they load, when no schema of the game's version names the sheet, or
 * when EXDSchema cannot be reached: the names are presentation only, and
 * without them a column shows its index.
 */
export function useSheetColumnNames(sheetName: string | null): SheetColumnNamesDto | null {
  const [state, setState] = useState<{ sheet: string; names: SheetColumnNamesDto | null } | null>(null);
  useEffect(() => {
    if (!sheetName) return;
    let current = true;
    sheetColumnNames(sheetName).then(
      (names) => { if (current) setState({ sheet: sheetName, names }); },
      () => { if (current) setState({ sheet: sheetName, names: null }); },
    );
    return () => { current = false; };
  }, [sheetName]);
  return state && state.sheet === sheetName ? state.names : null;
}
