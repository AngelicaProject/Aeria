# Workspace Format v1 — design constraints

Status: **draft, not yet frozen**.

The first public format must satisfy these constraints before real projects rely on it:

- one target language per repository
- sparse translated-unit overlay
- stable deterministic translation-unit identity
- explicit current source snapshot/content binding
- current runtime coordinate binding per translated unit
- source fingerprint sufficient to detect incompatible/stale binding
- target structured-string representation
- `draft`, `reviewed`, and `needs-review` states
- optional translator notes
- deterministic serialization and ordering
- small, merge-friendly Git diffs
- format version and lossless migration path
- no machine-local source paths, credentials, provider choices, or cache data
- no requirement to commit the entire source corpus

## Finalized identity contract

The identity contract is final for v1 even though the overall workspace format remains draft. Newly created translation units use `tu1:<64 lowercase hexadecimal characters>`, where the digest is SHA-256 over:

1. the unframed UTF-8 bytes of `aeria.translation-unit.v1`;
2. the source language as a little-endian `u32` byte length followed by UTF-8 bytes;
3. the sheet name with the same framing;
4. row ID as little-endian `u32`;
5. subrow ID as little-endian `u16`;
6. column index as little-endian `u32`;
7. the raw 32-byte source macro-text hash.

Raw-value and row-technical hashes are fingerprints, not identity inputs. Target language, target text, review state, game version, `contentId`, and `snapshotId` are also excluded. The ID is derived once at unit creation and is retained across source-coordinate, source-text, and source-fingerprint changes; rebase must never recompute an existing unit ID.

The concrete file layout and serialization syntax must be chosen after testing Git diff/merge behavior with realistic large-project fixtures. Do not treat JSONL or a custom extension as decided merely because it was discussed as a candidate.
