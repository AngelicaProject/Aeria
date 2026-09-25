# Format specifications

This section owns Aeria-defined persisted and exported contracts.

- [`workspace-v2.md`](./workspace-v2.md) — current translation workspace format version 2.
- [`workspace-v1.md`](./workspace-v1.md) — superseded workspace format version 1, read only for migration.
- [`workspace-v1-evaluation.md`](./workspace-v1-evaluation.md) — Git diff/merge evidence supporting the workspace layout decision, unchanged in version 2.
- [`glossary-v1.md`](./glossary-v1.md) — project-shared glossary (`aeria-glossary.csv`).
- [`collaboration-v1.md`](./collaboration-v1.md) — project-shared collaboration policy (`aeria-collaboration.json`).
- [`pack-v1.md`](./pack-v1.md) — compiled translation pack format version 1 for Harmonia.

HXS is produced by Harmonia Atlas and is an external source contract; Aeria's HXS behavior is documented in [`../architecture/source.md`](../architecture/source.md).

Once an Aeria format is released, incompatible changes require a new format version or an explicit lossless migration path. Do not silently reinterpret older data.
