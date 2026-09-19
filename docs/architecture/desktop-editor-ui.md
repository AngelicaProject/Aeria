# Desktop editor UI

The desktop renderer provides the first human translation-editing vertical slice over the existing Tauri application boundary.

## Flow

```text
project launcher
→ active project header
→ sheet selection
→ bounded translation entries
→ selected source/target editor
→ explicit target and note persistence
→ immediate review-state persistence
```

The launcher can open an existing project or initialize a project with a repository root, HXS source path, and free-text target language. Once a project is active, the renderer uses the sheets in `ProjectSummaryDto`, pages one sheet at a time with a limit of 100 entries, and offers an explicit **Load more** action.

The renderer owns only ephemeral drafts, navigation, paging, and loading/error presentation. Rust remains authoritative for source bindings, workspace state, validation, and mutation semantics. Target and note text do not autosave: both use explicit save actions. Review-state changes persist immediately. After every successful mutation, the renderer re-reads the bounded pages currently visible from Rust and restores the selected entry by its `SourceBinding`.

Unsaved target or note drafts are marked and protected by a discard confirmation when changing entries, changing sheets, closing the project, or performing a mutation that would refresh away another dirty draft. The UI distinguishes an absent translation overlay from an overlay whose target is explicitly empty.

This first editor intentionally has no file picker, search or filtering, virtualization, structured macro controls, source update/rebase UI, export, Git controls, or AI controls. Macro text is shown literally with whitespace preserved.
