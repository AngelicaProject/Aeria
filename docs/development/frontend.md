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

The project launcher uses a fixed shell so switching between Recent, Open, Create, and Settings does not resize the desktop window. Its integrated titlebar is 36px high, and only the content region may scroll. The Tauri launcher window is 900×560 with matching minimum dimensions. It opens centered and recenters when returning from the workbench.

Recent project headers and rows share one four-column grid: `1fr 110px 90px 140px`. Keep that template in one CSS custom property or shared class so labels and values cannot drift apart.

Tailwind CSS and accessible primitive libraries such as Radix are approved directions, but dependencies should be added when their first real component is implemented.

Ordinary renderer icons use the shared `UiIcon` mapping backed by `lucide-react`. Choose from its fixed `xs`, `sm`, `md`, and `lg` sizes; do not add inline SVGs, icon-font glyphs, emoji, or Unicode icon characters in feature components. Keep approved custom or native window icons centralized in the shared icon layer, and preserve semantic color through `currentColor`.
