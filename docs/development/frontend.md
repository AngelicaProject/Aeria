# Frontend engineering

The renderer is a full React/TypeScript application, not a thin HTML skin, but presentation is its authority boundary.

## Rules

- TypeScript runs in strict mode.
- Domain filesystem/Git/HXS/SQLite operations go through typed Tauri capabilities.
- Keep UI state close to the feature that owns it. Avoid a single global application store.
- Large source lists must be virtualized; never render the entire corpus into the DOM.
- Keyboard-first workflows are first-class.
- Dockable/resizable panels and persisted layouts are product goals, but introduce the layout library only after prototyping the actual editor surfaces.
- CodeMirror 6 is the preferred starting point for the structured translation editor; Monaco is not required unless future requirements justify its weight.
- Dark theme ships first; the design system must not make a future light theme prohibitively expensive.

## Project launcher layout

The Tauri launcher window is 900×560 with matching minimum dimensions and is not resizable. It opens centered and recenters when returning from the workbench. The launcher is one screen: a fixed-width action column and a panel that swaps between Recent projects and the Open, Clone, New, or Update project form. Switching views never resizes the window, and only the panel content scrolls.

## Styling and primitives

Styles live in `src/styles/` by surface (`base`, `primitives`, `chrome`, `launcher`, `workbench`, `editor`, `git`, `overlays`). Component CSS uses only the semantic tokens in `src/ui/theme/tokens.css` (`--panel`, `--line`, `--fg-muted`, `--accent`, `--review-*`, and so on), never raw theme palette values, so themes and appearances stay cheap to add. Theme palettes are applied to `<html>` so portalled popovers resolve them.

Renderer preferences go through `usePreferences` (`src/ui/preferences.tsx`, parsed by `preferencesModel.ts`); appearance goes through `useTheme`. Both validate stored values and fall back to defaults.

## Interface localization

The renderer interface ships in English and Russian. Only Aeria's own chrome is localized: menus, dialogs, labels, tooltips, accessible names, and messages the renderer composes itself. Game text, project data, sheet and theme names, Git data, and diagnostics returned by Rust commands (`CommandError.message`) are shown unchanged.

- Messages live in `src/i18n/`. `en.ts` is the source of message keys; every other catalog is typed as `Catalog` and must provide the same keys with the same `{placeholders}`. `tests/i18n.test.mjs` enforces key, placeholder, and plural-category parity.
- Components read `t` from `useI18n()` (`src/ui/i18n.tsx`). Do not put user-visible literals in components; add a key to every catalog instead. Non-React modules return `MessageKey`s or accept a `Translate` function.
- Plural messages are objects keyed by `Intl.PluralRules` categories and are selected by a numeric `count` param. Russian needs `one`, `few`, `many`, and `other`.
- Numeric params are formatted for the locale with digit grouping, so pass identifiers such as row ids, subrow ids, and column indexes as strings. Relative times, dates, and percentages use the interface locale, not the OS locale.
- The interface language is the `language` preference (`system`, `en`, or `ru`). `system` picks the first supported OS language and otherwise falls back to English. Resetting editor settings keeps the language. The resolved locale is written to `<html lang>`.

Window APIs used by the renderer must be granted in `src-tauri/capabilities/default.json`. In particular, `onCloseRequested` destroys the window after its handler runs, so closing requires `core:window:allow-destroy`, and read-only window getters come from `core:window:default`.

Menus, dialogs, tooltips, and selects use Radix primitives (`radix-ui`) styled with the shared `menu-*`, `dialog*`, and `tooltip` classes. Icon-only buttons use `IconButton`, which requires an accessible label and shows it as a tooltip. Any other hint is an ordinary `title`: `TitleTooltips`, mounted once in `App`, shows it as a themed tooltip after a short hover or on keyboard focus and moves it to `data-tooltip`, so the native tooltip never appears; an element whose only name was its title receives it as `aria-label`. Line breaks in a title are kept. Dropdowns use `ui/primitives/Select` (Radix Select in the shared `menu-*` style, with a `field` or `quiet` look, optional groups and hints, and the empty string allowed as a value); a test rejects native `<select>` elements. Richer pickers use a Radix menu or popover with the same classes. Source and target text use `MacroEditor` (CodeMirror 6); long lists use `@tanstack/react-virtual` or the existing fixed-height virtual tree.

Ordinary renderer icons use the shared `UiIcon` mapping backed by `lucide-react`. Choose from its fixed `xs`, `sm`, `md`, `lg`, and `xl` sizes (`xl` is for empty-state illustrations); do not add inline SVGs, icon-font glyphs, emoji, or Unicode icon characters in feature components. Keep approved custom or native window icons centralized in the shared icon layer, and preserve semantic color through `currentColor`.
