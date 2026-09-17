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

Tailwind CSS and accessible primitive libraries such as Radix are approved directions, but dependencies should be added when their first real component is implemented.
