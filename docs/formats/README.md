# Format specifications

This section owns Aeria-defined persisted and exported contracts.

- [`workspace-v3.md`](./workspace-v3.md) — current translation workspace format version 3. Earlier versions are not read.
- [`knowledge-v1.md`](./knowledge-v1.md) — project knowledge: the `aeria-knowledge` directory of style, terms, character voices, story, and lessons.
- [`glossary-v1.md`](./glossary-v1.md) — the project's terms (`aeria-knowledge/terms.csv`).
- [`voices-v1.md`](./voices-v1.md) — character voice profiles (`aeria-knowledge/characters.md`).
- [`collaboration-v1.md`](./collaboration-v1.md) — project-shared collaboration policy (`aeria-collaboration.json`).
- [`pack-settings-v1.md`](./pack-settings-v1.md) — project-shared pack identity and signing key fingerprint (`aeria-pack.json`).
- [`font-settings-v1.md`](./font-settings-v1.md) — project-shared source fonts for glyphs the game fonts lack (`aeria-fonts.json`).
- [`pack-v1.md`](./pack-v1.md) — compiled translation pack format version 1 for Harmonia (proposal).
- [`feed-v1.md`](./feed-v1.md) — pack update feed and publisher trust rules for Harmonia (proposal).

The source is the installed game; how Aeria reads it is documented in [`../architecture/source.md`](../architecture/source.md).

Once an Aeria format is released, incompatible changes require a new format version or an explicit lossless migration path. Do not silently reinterpret older data.
