# Dependency policy and major dependencies

Aeria prefers a small set of mature dependencies over internal reinvention of generic infrastructure.

A dependency should have a concrete current use, a maintenance story, and compatible licensing. Remove unused dependencies rather than keeping speculative framework pieces.

## Approved architectural foundations

- Tauri 2: desktop shell and typed IPC boundary
- React: renderer UI
- TypeScript: strict frontend typing
- Vite: frontend build/dev tooling
- SQLite: local disposable indexes/cache/job state

Likely UI-level dependencies such as Tailwind, Radix primitives, CodeMirror, docking/layout libraries, and Git/SQLite Rust crates should be added when the first feature that needs them is implemented, after checking their current stable versions and licenses.
