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

The concrete file layout and serialization syntax must be chosen after testing Git diff/merge behavior with realistic large-project fixtures. Do not treat JSONL or a custom extension as decided merely because it was discussed as a candidate.
