import { memo, useEffect, useState } from "react";
import { normalizeCommandError, sourceInOtherLanguages } from "../ipc";
import { languageName } from "../targetLanguages";
import type { CommandError, OtherLanguageTextDto, SourceBinding } from "../types";
import { useI18n } from "../ui/i18n";
import { useMacroView } from "../ui/useMacroView";
import { MacroEditor, type MacroPresentation } from "./MacroEditor";
import type { ChipPick } from "./macroChipsExtension";

type OtherLanguagesProps = {
  /** The source cell to show in the other client languages. */
  binding: SourceBinding;
  presentation: MacroPresentation;
  /** Adds a clicked tag to the translation, as in the source pane. */
  onPick?: ((pick: ChipPick) => void) | undefined;
};

/** The selected source text as the game's other client languages have it. */
export const OtherLanguages = memo(function OtherLanguages({ binding, presentation, onPick }: OtherLanguagesProps) {
  const { t } = useI18n();
  const [texts, setTexts] = useState<OtherLanguageTextDto[] | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const { sheetName, rowId, subrowId, columnIndex } = binding;

  useEffect(() => {
    let cancelled = false;
    setTexts(null);
    setError(null);
    sourceInOtherLanguages({ sheetName, rowId, subrowId, columnIndex })
      .then((next) => { if (!cancelled) setTexts(next); })
      .catch((caught: unknown) => { if (!cancelled) setError(normalizeCommandError(caught)); });
    return () => { cancelled = true; };
  }, [sheetName, rowId, subrowId, columnIndex]);

  if (error) return <p className="other-languages-empty muted">{error.message}</p>;
  if (!texts) return <p className="other-languages-empty muted">{t("common.loading")}</p>;
  return (
    <ul className="other-languages">
      {texts.map(({ language, text }) => (
        <li key={language}>
          <span className="chip" title={languageName(language)}>{language.toUpperCase()}</span>
          {text === null
            ? <p className="muted">{t("languages.missing")}</p>
            : <LanguageText language={language} text={text} presentation={presentation} onPick={onPick} />}
        </li>
      ))}
    </ul>
  );
});

function LanguageText({ language, text, presentation, onPick }: { language: string; text: string; presentation: MacroPresentation; onPick?: ((pick: ChipPick) => void) | undefined }) {
  const { t } = useI18n();
  const view = useMacroView(text);
  return <MacroEditor className="other-language-text" value={text} readOnly view={view} presentation={presentation} onPick={onPick} ariaLabel={t("languages.text", { language: languageName(language) })} placeholder={t("editor.emptySource")} />;
}
