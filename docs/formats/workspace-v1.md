# Workspace Format v1

Status: **superseded by [Workspace Format v2](./workspace-v2.md); read only
for migration**.

Aeria no longer writes v1 and never edits a v1 workspace. Opening a v1
workspace requires a source update, which migrates it to v2 without loss; see
[`workspace-v2.md`](./workspace-v2.md#migration-from-v1).

## Differences from v2

v1 uses the same repository layout, sharding, record ordering, canonical
JSON, and reader tolerances as v2. It differs only in these facts:

- The manifest has `"formatVersion": 1` and an additional required
  `snapshotId` field after `contentId`. The reader validates it as a canonical
  `sha256:` HXS ID and discards it; v2 identifies the source by content only.
- Unit records have no `sourceStatus`, `sourceLayout`, or `sourceRowKey`
  field. Every v1 unit is bound, and its source layout and row key are
  unknown.
- Current bindings are unique across the whole workspace.

A v1 manifest:

```json
{
  "formatVersion": 1,
  "sourceLanguage": "en",
  "targetLanguage": "fr",
  "contentId": "sha256:<64 lowercase hexadecimal characters>",
  "snapshotId": "sha256:<64 lowercase hexadecimal characters>"
}
```

A v1 unit record:

```json
{"id":"tu1:<hex>","sourceBinding":{"sheetName":"...","rowId":0,"subrowId":0,"columnIndex":0},"sourceFingerprint":{"macroTextHash":"<hex>","rawValueHash":null,"rowTechnicalHash":"<hex>"},"targetMacro":"...","reviewState":"draft","translatorNote":null}
```

A v1 manifest must be accompanied by v1 records only; a record in the other
shape fails validation.

The v1 golden fixture is kept at
`crates/aeria-workspace/tests/fixtures/workspace-v1/` for migration tests.
