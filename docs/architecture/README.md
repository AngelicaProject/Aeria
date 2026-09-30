# Architecture documentation

Start with [`overview.md`](./overview.md). It defines the major runtime boundaries and crate ownership. Then read the document for the subsystem being changed.

| Area | Canonical document | Primary implementation area |
| --- | --- | --- |
| Overall boundaries | [`overview.md`](./overview.md) | project-wide |
| Desktop application boundary | [`desktop-application-boundary.md`](./desktop-application-boundary.md) | `apps/desktop/src-tauri` |
| Desktop translation editor UI | [`desktop-editor-ui.md`](./desktop-editor-ui.md) | `apps/desktop/src` |
| The project as PO files: format, identity, editing, checks, game updates | [`po-project.md`](./po-project.md) | `aeria-po` |
| The installed game as source: sheets, permission, row keys | [`source.md`](./source.md) | `aeria-sqpack`, `aeria-source` |
| Structured strings and macros | [`strings.md`](./strings.md) | `aeria-se` |
| Git-backed collaboration | [`git.md`](./git.md) | `aeria-git` |
| Search and translation memory | [`search.md`](./search.md) | `aeria-search` |
| Project knowledge and translation rules | [`knowledge.md`](./knowledge.md) | `aeria-knowledge` |
| Machine translation with a ChatGPT subscription | [`translate.md`](./translate.md) | `aeria-model` |
| Runtime translation pack export | [`export.md`](./export.md) | `aeria-export` |

Architecture documents own boundaries and invariants, not low-level coding style. Implementation conventions belong under [`../development/`](../development/README.md), while serialized contracts belong under [`../formats/`](../formats/README.md).
