import { useEffect, useState } from "react";
import { translationNameSheets } from "../ipc";
import { useI18n } from "./i18n";
import { UiIcon } from "./primitives/UiIcon";

// The list is fixed for a build of Aeria: read once for the window.
let loaded: Promise<ReadonlySet<string>> | null = null;

function loadNameSheets(): Promise<ReadonlySet<string>> {
  loaded ??= translationNameSheets()
    .then((names) => new Set(names) as ReadonlySet<string>)
    .catch(() => {
      loaded = null;
      return new Set<string>();
    });
  return loaded;
}

/**
 * The sheets of names: machine translation sends their translations with
 * every request whose strings use them, and the model writes them exactly.
 */
export function useNameSheets(): ReadonlySet<string> {
  const [sheets, setSheets] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => {
    let live = true;
    void loadNameSheets().then((found) => { if (live) setSheets(found); });
    return () => { live = false; };
  }, []);
  return sheets;
}

/** Marks a sheet of names, whose translations machine translation spreads through the project. */
export function NameSheetMark() {
  const { t } = useI18n();
  return (
    <span className="name-sheet-mark" title={t("nameSheet.mark")} aria-label={t("nameSheet.mark")} role="img">
      <UiIcon icon="bookMarked" size="xs" />
    </span>
  );
}
