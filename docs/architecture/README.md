# Architecture documentation

Start with [`overview.md`](./overview.md). It defines the major runtime boundaries and crate ownership. Then read the document for the subsystem being changed.

| Area | Canonical document | Primary implementation area |
| --- | --- | --- |
| Overall boundaries | [`overview.md`](./overview.md) | workspace-wide |
| Desktop application boundary | [`desktop-application-boundary.md`](./desktop-application-boundary.md) | `apps/desktop/src-tauri` |
| Open project session ownership | [`project-session.md`](./project-session.md) | `aeria-workspace` |
| Bounded translation reads | [`translation-read.md`](./translation-read.md) | `aeria-workspace`, `aeria-hxs` |
| Transactional translation mutations | [`translation-mutations.md`](./translation-mutations.md) | `aeria-workspace` |
| HXS source snapshots and Atlas integration | [`source.md`](./source.md) | `aeria-hxs`, desktop source management |
| Structured strings and macros | [`strings.md`](./strings.md) | `aeria-se` |
| Translation unit identity | [`identity.md`](./identity.md) | `aeria-core`, `aeria-workspace`, `aeria-rebase` |
| Workspace state and persistence | [`workspace.md`](./workspace.md) | `aeria-workspace` |
| Source update and deterministic rebase | [`rebase.md`](./rebase.md), [`rebase-safety.md`](./rebase-safety.md), [`rebase-candidates.md`](./rebase-candidates.md) | `aeria-rebase` |
| Git-backed collaboration | [`git.md`](./git.md) | `aeria-git` |
| Translation assistance | [`ai.md`](./ai.md) | `aeria-ai` |
| Runtime translation pack export | [`export.md`](./export.md) | `aeria-export` |

Architecture documents own boundaries and invariants, not low-level coding style. Implementation conventions belong under [`../development/`](../development/README.md), while serialized contracts belong under [`../formats/`](../formats/README.md).
