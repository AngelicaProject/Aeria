import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { open as openNativeDialog } from "@tauri-apps/plugin-dialog";
import { fontsImportFile, fontsOverview, fontsPreview, fontsSave, fontsUseRecommended, normalizeCommandError } from "../ipc";
import type { CommandError, FontCaseMapping, FontPreviewSizeDto, FontSettings, FontSource, FontTarget, FontsOverviewDto } from "../types";
import { useI18n } from "../ui/i18n";
import { Segmented } from "../ui/primitives/Segmented";
import { Select } from "../ui/primitives/Select";
import { UiIcon } from "../ui/primitives/UiIcon";
import { ErrorBanner } from "./ErrorBanner";

type FontSettingsSectionProps = {
  /** Disables editing while the export dialog works. */
  disabled: boolean;
  /** Called after the settings were saved, so the export state refreshes. */
  onSaved: () => void;
};

type Zoom = "1" | "2" | "3" | "auto";

const SAMPLES: Record<string, string> = {
  Jupiter: "Настройки персонажа «Ёжик» № Щит",
  TrumpGothic: "ЗАДАНИЕ ВЫПОЛНЕНО Съешь же ещё",
  MiedingerMid: "ЖУРНАЛ ЗАДАНИЙ Щит Ёж",
};

function sameJson(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function sourceId(file: string, taken: readonly FontSource[]): string {
  const stem = file.replace(/^.*\//, "").replace(/\.[^.]+$/, "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 56) || "font";
  let id = stem;
  for (let index = 2; taken.some((source) => source.id === id); index++) id = `${stem}-${index}`;
  return id;
}

function newTarget(font: string, source: string): FontTarget {
  return { font, source, axes: {}, scale: 1, widthScale: 1, baselineShift: 0, tracking: 0, caseMapping: font === "MiedingerMid" ? "upper" : "none", sizes: {} };
}

function zoomFor(zoom: Zoom, lineHeight: number): number {
  if (zoom !== "auto") return Number(zoom);
  return lineHeight <= 30 ? 3 : lineHeight <= 50 ? 2 : 1;
}

/** One size drawn as the game would, magnified, over the native metric guides. */
const PreviewCanvas = memo(function PreviewCanvas({ preview, zoom }: { preview: FontPreviewSizeDto; zoom: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useLayoutEffect(() => {
    const element = canvas.current;
    const context = element?.getContext("2d");
    if (!element || !context) return;
    const width = Math.max(preview.width, 1);
    const height = preview.lineHeight;
    element.width = width * zoom;
    element.height = height * zoom;
    const style = getComputedStyle(element);
    const ink = style.color;
    const guide = style.getPropertyValue("--font-guide").trim() || "rgba(128,128,128,0.5)";
    context.clearRect(0, 0, element.width, element.height);
    context.fillStyle = guide;
    const line = (row: number) => context.fillRect(0, row * zoom, element.width, Math.max(1, zoom / 2));
    line(preview.ascent);
    line(preview.ascent - preview.capHeight);
    context.fillStyle = ink;
    for (let y = 0; y < preview.height; y++) {
      for (let x = 0; x < preview.width; x++) {
        const value = preview.pixels[y * preview.width + x];
        if (!value) continue;
        context.globalAlpha = value / 255;
        context.fillRect(x * zoom, y * zoom, zoom, zoom);
      }
    }
    context.globalAlpha = 1;
  }, [preview, zoom]);
  return <canvas ref={canvas} className="font-preview-canvas" style={{ width: Math.max(preview.width, 1) * zoom, height: preview.lineHeight * zoom }} />;
});

/** Settings and preview of the glyphs Harmonia adds to the game fonts. */
export const FontSettingsSection = memo(function FontSettingsSection({ disabled, onSaved }: FontSettingsSectionProps) {
  const { t } = useI18n();
  const [overview, setOverview] = useState<FontsOverviewDto | null>(null);
  const [draft, setDraft] = useState<FontSettings | null>(null);
  const [font, setFont] = useState("Jupiter");
  const [samples, setSamples] = useState<Record<string, string>>(SAMPLES);
  const [zoom, setZoom] = useState<Zoom>("auto");
  const [preview, setPreview] = useState<FontPreviewSizeDto[] | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<CommandError | null>(null);

  const show = useCallback((next: FontsOverviewDto) => {
    setOverview(next);
    setDraft(next.settings);
  }, []);

  useEffect(() => {
    void fontsOverview().then(show).catch((reason: unknown) => setError(normalizeCommandError(reason)));
  }, [show]);

  const target = draft?.fonts.find((candidate) => candidate.font === font) ?? null;
  const sample = samples[font] ?? "";

  useEffect(() => {
    if (!draft || !target) { setPreview(null); setPreviewError(null); return; }
    let current = true;
    const timer = window.setTimeout(() => {
      fontsPreview(draft, font, sample)
        .then((sizes) => { if (current) { setPreview(sizes); setPreviewError(null); } })
        .catch((reason: unknown) => { if (current) setPreviewError(normalizeCommandError(reason).message); });
    }, 250);
    return () => { current = false; window.clearTimeout(timer); };
  }, [draft, target, font, sample]);

  const run = async <T,>(label: string, operation: () => Promise<T>): Promise<T | undefined> => {
    setBusy(label);
    setError(null);
    try {
      return await operation();
    } catch (reason) {
      setError(normalizeCommandError(reason));
      return undefined;
    } finally {
      setBusy(null);
    }
  };

  const saveWith = async (operation: () => Promise<FontsOverviewDto>) => {
    const next = await run(t("common.saving"), operation);
    if (!next) return;
    show(next);
    onSaved();
  };

  const updateTarget = (change: (current: FontTarget) => FontTarget) => setDraft((current) => current && ({
    ...current,
    fonts: current.fonts.map((candidate) => (candidate.font === font ? change(candidate) : candidate)),
  }));

  const toggleFont = (enabled: boolean) => setDraft((current) => {
    if (!current) return current;
    if (!enabled) return { ...current, fonts: current.fonts.filter((candidate) => candidate.font !== font) };
    const source = current.sources[0]?.id;
    if (!source) return current;
    const fonts = [...current.fonts, newTarget(font, source)].sort((a, b) => (a.font < b.font ? -1 : 1));
    return { ...current, fonts };
  });

  const addSource = async () => {
    const fontFile = await openNativeDialog({ multiple: false, directory: false, filters: [{ name: t("fonts.fontFilter"), extensions: ["ttf", "otf"] }] });
    if (typeof fontFile !== "string") return;
    const imported = await run(t("fonts.importing"), () => fontsImportFile(fontFile));
    if (!imported) return;
    if (!imported.isFont) { setError({ code: "fontSettingsInvalid", message: t("fonts.notAFont") }); return; }
    const licenseFile = await openNativeDialog({ multiple: false, directory: false, title: t("fonts.pickLicense"), filters: [{ name: t("fonts.licenseFilter"), extensions: ["txt", "md"] }] });
    if (typeof licenseFile !== "string") return;
    const license = await run(t("fonts.importing"), () => fontsImportFile(licenseFile));
    if (!license) return;
    setDraft((current) => {
      if (!current) return current;
      const source: FontSource = {
        id: sourceId(imported.file, current.sources),
        file: imported.file,
        family: imported.family ?? imported.file.replace(/^.*\//, ""),
        copyright: imported.copyright ?? imported.family ?? imported.file,
        license: "OFL-1.1",
        licenseFile: license.file,
      };
      return {
        ...current,
        sources: [...current.sources, source],
        fonts: current.fonts.map((candidate) => (candidate.font === font ? { ...candidate, source: source.id, axes: {} } : candidate)),
      };
    });
  };

  const updateSource = (id: string, change: Partial<FontSource>) => setDraft((current) => current && ({
    ...current,
    sources: current.sources.map((source) => (source.id === id ? { ...source, ...change } : source)),
  }));

  if (error && !overview) return <ErrorBanner title={t("fonts.error")} error={error} onDismiss={() => setError(null)} />;
  if (!overview) return <p className="muted">{t("common.loading")}</p>;

  const locked = disabled || busy !== null;
  const dirty = draft !== null && !sameJson(draft, overview.settings);
  const sourceInfo = overview.sources.find((source) => source.id === target?.source);
  const gameFont = overview.gameFonts.find((candidate) => candidate.name === font);
  const overridden = target ? Object.keys(target.sizes) : [];
  const slider = (label: string, value: number, min: number, max: number, step: number, onChange: (value: number) => void) => (
    <label className="field font-slider">
      <span className="field-label">{label}</span>
      <span className="export-inline">
        <input type="range" min={min} max={max} step={step} value={value} disabled={locked} onChange={(event) => onChange(Number(event.target.value))} />
        <input className="input font-number" type="number" min={min} max={max} step={step} value={value} disabled={locked} onChange={(event) => onChange(Number(event.target.value))} />
      </span>
    </label>
  );

  return (
    <>
      {error ? <ErrorBanner title={t("fonts.error")} error={error} onDismiss={() => setError(null)} /> : null}
      <p className="field-hint">{t("fonts.hint")}</p>
      {overview.settingsError ? <p className="ai-test-result failed"><UiIcon icon="circleAlert" size="xs" />{overview.settingsError}</p> : null}

      {draft === null ? (
        <div className="dialog-actions export-actions-start">
          <span className="muted">{t("fonts.none")}</span>
          <button className="button button-secondary" type="button" disabled={locked} onClick={() => void saveWith(fontsUseRecommended)}><UiIcon icon="sparkles" size="sm" />{t("fonts.useRecommended")}</button>
        </div>
      ) : (
        <>
          <Segmented<string> label={t("fonts.gameFont")} value={font} disabled={busy !== null} onChange={setFont} options={overview.gameFonts.map((candidate) => ({ value: candidate.name, label: candidate.name }))} />
          <label className="font-toggle">
            <input type="checkbox" checked={target !== null} disabled={locked || draft.sources.length === 0} onChange={(event) => toggleFont(event.target.checked)} />
            {t("fonts.enabled", { font })}
          </label>

          {target ? (
            <>
              <div className="export-grid">
                <label className="field">
                  <span className="field-label">{t("fonts.source")}</span>
                  <span className="export-inline">
                    <Select<string> value={target.source} disabled={locked} label={t("fonts.source")} onChange={(source) => updateTarget((current) => ({ ...current, source, axes: {} }))} options={draft.sources.map((source) => ({ value: source.id, label: source.family, hint: source.file }))} />
                    <button className="button button-ghost" type="button" disabled={locked} onClick={() => void addSource()} title={t("fonts.addSourceHint")}><UiIcon icon="plus" size="sm" />{t("fonts.addSource")}</button>
                  </span>
                  {sourceInfo?.error ? <span className="export-warning-inline">{sourceInfo.error}</span> : null}
                </label>
                <div className="field">
                  <span className="field-label">{t("fonts.caseMapping")}</span>
                  <Segmented<FontCaseMapping> label={t("fonts.caseMapping")} value={target.caseMapping} disabled={locked} onChange={(caseMapping) => updateTarget((current) => ({ ...current, caseMapping }))} options={[{ value: "none", label: t("fonts.caseNone") }, { value: "upper", label: t("fonts.caseUpper") }]} />
                </div>
                {sourceInfo?.axes.map((axis) => slider(
                  t("fonts.axis", { tag: axis.tag }),
                  target.axes[axis.tag] ?? axis.default,
                  axis.min,
                  axis.max,
                  axis.max - axis.min > 20 ? 1 : 0.1,
                  (value) => updateTarget((current) => ({ ...current, axes: { ...current.axes, [axis.tag]: value } })),
                ))}
                {slider(t("fonts.scale"), target.scale, 0.5, 2, 0.01, (scale) => updateTarget((current) => ({ ...current, scale })))}
                {slider(t("fonts.widthScale"), target.widthScale, 0.5, 2, 0.01, (widthScale) => updateTarget((current) => ({ ...current, widthScale })))}
                {slider(t("fonts.tracking"), target.tracking, -0.5, 1, 0.01, (tracking) => updateTarget((current) => ({ ...current, tracking })))}
                {slider(t("fonts.baselineShift"), target.baselineShift, -16, 16, 1, (baselineShift) => updateTarget((current) => ({ ...current, baselineShift })))}
              </div>
              {overridden.length > 0 ? <p className="field-hint">{t("fonts.overrides", { sizes: overridden.join(", ") })}</p> : null}

              <div className="export-inline">
                <input className="input" value={sample} spellCheck={false} aria-label={t("fonts.sample")} placeholder={t("fonts.sample")} onChange={(event) => setSamples((current) => ({ ...current, [font]: event.target.value }))} />
                <Segmented<Zoom> label={t("fonts.zoom")} value={zoom} onChange={setZoom} options={[{ value: "auto", label: t("fonts.zoomAuto") }, { value: "1", label: "1×" }, { value: "2", label: "2×" }, { value: "3", label: "3×" }]} />
              </div>
              {previewError ? <p className="export-warning"><UiIcon icon="circleAlert" size="xs" />{previewError}</p> : null}
              <div className="font-preview" aria-live="polite">
                {(preview ?? []).map((size) => {
                  const native = gameFont?.sizes.find((candidate) => candidate.size === size.size);
                  return (
                    <figure key={size.size} className="font-preview-size">
                      <figcaption className="font-preview-caption">
                        <strong>{`${font}_${size.size}`}</strong>
                        <span>{t("fonts.metrics", { lineHeight: size.lineHeight, capHeight: size.capHeight })}</span>
                        {size.generatedCapAdvance !== null && native ? <span title={t("fonts.advanceHint")}>{t("fonts.advance", { native: native.capAdvance.toFixed(1), generated: size.generatedCapAdvance.toFixed(1) })}</span> : null}
                        {size.missing.length > 0 ? <span className="export-warning-inline">{t("fonts.missing", { characters: size.missing.join(" ") })}</span> : null}
                      </figcaption>
                      {size.error ? <p className="export-warning"><UiIcon icon="circleAlert" size="xs" />{size.error}</p> : (
                        <div className="font-preview-scroll"><PreviewCanvas preview={size} zoom={zoomFor(zoom, size.lineHeight)} /></div>
                      )}
                    </figure>
                  );
                })}
              </div>
              <p className="field-hint">{t("fonts.guides")}</p>
            </>
          ) : null}

          <details className="font-details">
            <summary>{t("fonts.sources", { count: draft.sources.length })}</summary>
            {draft.sources.map((source) => (
              <div key={source.id} className="export-grid font-source">
                <label className="field"><span className="field-label">{source.file}</span>
                  <input className="input" value={source.copyright} disabled={locked} aria-label={t("fonts.copyright")} onChange={(event) => updateSource(source.id, { copyright: event.target.value })} />
                </label>
                <label className="field"><span className="field-label">{t("fonts.license", { file: source.licenseFile })}</span>
                  <input className="input" value={source.license} disabled={locked} onChange={(event) => updateSource(source.id, { license: event.target.value })} />
                </label>
              </div>
            ))}
            <label className="field">
              <span className="field-label">{t("fonts.characters")}</span>
              <textarea className="input export-changelog" value={draft.characters} disabled={locked} spellCheck={false} onChange={(event) => setDraft((current) => current && ({ ...current, characters: event.target.value }))} />
            </label>
          </details>

          <div className="dialog-actions">
            {busy ? <span className="muted">{busy}</span> : null}
            <button className="button button-ghost" type="button" disabled={locked} onClick={() => void saveWith(fontsUseRecommended)} title={t("fonts.resetHint")}>{t("fonts.reset")}</button>
            {dirty ? <button className="button button-ghost" type="button" disabled={locked} onClick={() => setDraft(overview.settings)}>{t("guide.revert")}</button> : null}
            <button className="button button-secondary" type="button" disabled={locked || !dirty} onClick={() => { if (draft) void saveWith(() => fontsSave(draft)); }}>{t("fonts.save")}</button>
          </div>
        </>
      )}
    </>
  );
});
