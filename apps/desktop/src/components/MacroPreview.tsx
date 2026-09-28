import { useMemo } from "react";
import { useMacroIdioms } from "../macroIdioms";
import { idiomLabel } from "../macroChips";
import { segmentMacroText } from "../macroTokens";
import { useI18n } from "../ui/i18n";

const lineBreak = /^<br\s*\/?>$/i;

/**
 * Macro text with its tags tinted. On one line, line breaks read as spaces;
 * `multiline` breaks the line at each `<br>` and keeps the tag visible.
 */
export function MacroPreview({ text, empty, multiline = false }: { text: string | null; empty: string; multiline?: boolean }) {
  const { t } = useI18n();
  const idioms = useMacroIdioms();
  const segments = useMemo(() => text ? segmentMacroText(multiline ? text : text.replace(/\s*\n\s*/g, " "), idioms) : [], [text, multiline, idioms]);
  if (text === null) return <span className="lens-empty">{empty}</span>;
  if (text.length === 0) return <span className="lens-empty">{t("common.empty")}</span>;
  return (
    <>
      {segments.map((segment, index) => segment.kind === "idiom"
        ? <span className="lens-macro lens-idiom" key={index} title={segment.text}>{idiomLabel(t, segment.name, segment.summary)}</span>
        : segment.kind === "macro"
          ? <span className="lens-macro" key={index}>{segment.text}{multiline && lineBreak.test(segment.text) ? <br /> : null}</span>
          : <span key={index}>{segment.text}</span>)}
    </>
  );
}
