# Format specifications

This section owns Aeria-defined persisted and exported contracts.

- [`workspace-v1.md`](./workspace-v1.md) — translation workspace format version 1.
- [`workspace-v1-evaluation.md`](./workspace-v1-evaluation.md) — Git diff/merge evidence supporting the Workspace Format v1 layout decision.
- [`pack-v1.md`](./pack-v1.md) — compiled translation pack format version 1 for Harmonia.

HXS is produced by Harmonia Atlas and is an external source contract; Aeria's HXS behavior is documented in [`../architecture/source.md`](../architecture/source.md).

Once an Aeria format is released, incompatible changes require a new format version or an explicit lossless migration path. Do not silently reinterpret older data.
