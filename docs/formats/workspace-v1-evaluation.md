# Workspace Format v1 layout evaluation

This document records the evidence supporting the frozen contract in
[`workspace-v1.md`](./workspace-v1.md). It is rationale, not a second format
specification.

## Reproduction

The evaluation code is isolated from production persistence in
`tools/workspace-format-evaluation`. It uses `aeria-core` domain values,
generates deterministic synthetic workspaces, writes temporary repositories,
and invokes the local Git executable for the experiments only. It does not add
a Git dependency to the Aeria runtime.

Run the automated smoke evaluation with:

```text
cargo test -p aeria-workspace-format-evaluation --locked
```

Reproduce the full scale and 1,000-unit diff/merge measurements with:

```text
cargo run --release -p aeria-workspace-format-evaluation -- --sizes 1000,50000,250000 --scenario-size 1000
```

The generator uses fixed seed `0x9e3779b97f4a7c15` and fixed constants for
all derived values. It includes plain and structured target strings, Unicode,
quotes, backslashes, actual newlines requiring JSON escaping, explicit empty
targets, absent and populated notes, all review states, varied coordinates,
and varied hashes. The 250,000-unit repositories are generated in temporary
directories and are not fixtures committed to Git. `--scale-only` can be used
to collect only the scale rows.

The scale measurements below are serialized working-tree bytes, excluding
`.git` object storage. Git object size was not treated as a portable
microbenchmark because pack state and filesystem allocation vary; tracked file
count and serialized bytes expose the structural overhead relevant to this
decision.

## Candidates

All candidates used the same manifest and JSON object field order. The
four layouts were:

1. One JSON record per unit at `.aeria/units/<shard>/tu1-<digest>.json`. The hyphen
   in the filename is required for Windows portability; the canonical ID with
   `tu1:` remains in the record.
2. One `.aeria/units.jsonl` file containing all records sorted by ID.
3. 256 sharded JSONL files named `.aeria/units/00.jsonl` through
   `.aeria/units/ff.jsonl`,
   with records sorted by full ID inside each shard.
4. The same `.aeria/units/` shards as candidate 3, using a multi-line JSON array of
   pretty-printed objects.

The logical dataset for every candidate contained only the domain facts now
defined by `aeria-core`: workspace metadata, translation-unit ID, source
binding, source fingerprint, target macro, review state, and optional note.
No source text or derived source-binding index was included.

## Scale results

| Units | One file/unit | Single JSONL | Sharded JSONL | Sharded pretty JSON |
| ---: | ---: | ---: | ---: | ---: |
| 1,000 | 1,001 files / 505,913 B | 2 / 505,913 B | 252 / 505,913 B | 252 / 613,913 B |
| 50,000 | 50,001 / 25,498,947 B | 2 / 25,498,947 B | 257 / 25,498,947 B | 257 / 30,898,947 B |
| 250,000 | 250,001 / 128,002,644 B | 2 / 128,002,644 B | 257 / 128,002,644 B | 257 / 155,002,644 B |

The 250,000-unit one-file run took several minutes on the Windows evaluation
environment while the sharded writes completed materially faster. The
one-file layout has excellent per-record locality but its file count grows
linearly with the workspace. The single JSONL layout has minimal file count
but places every record in one merge surface. Sharding bounds the file count
at 256 unit files while retaining JSONL byte density.

## Git diff results

The tool measured staged Git diffs after changing the target, review state,
note, source facts, adding/deleting one unit, changing 100 randomly selected
units, and adding 100 deterministic pseudo-random units. Counts are
`files changed / added lines / deleted lines` for a 1,000-unit dataset. Every
scenario had zero unrelated records reserialized.

| Candidate | Target | Review | Note | Source facts | Add 1 | Delete 1 | Change 100 | Add 100 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| One file/unit | 1/1/1 | 1/1/1 | 1/1/1 | 1/1/1 | 1/1/0 | 1/0/1 | 100/100/100 | 100/100/0 |
| Single JSONL | 1/1/1 | 1/1/1 | 1/1/1 | 1/1/1 | 1/1/0 | 1/0/1 | 1/100/100 | 1/100/0 |
| Sharded JSONL | 1/1/1 | 1/1/1 | 1/1/1 | 1/1/1 | 1/1/0 | 1/0/1 | 88/100/100 | 88/100/0 |
| Sharded pretty JSON | 1/1/1 | 1/1/1 | 1/1/1 | 1/8/8 | 1/17/0 | 1/0/17 | 88/160/160 | 88/1,702/0 |

The sharded JSONL result gives the desired one-record/one-line edit for
single-unit changes. Bulk changes touch only the shards containing changed
IDs. Pretty JSON makes individual records easier to inspect but expands each
unit to multiple lines and makes inserts/deletes rewrite much larger shard
hunks. Sorted insertion can shift later physical line positions, but the
unchanged JSONL records themselves are not reserialized.

## Git merge results

The tool created real temporary repositories with a base commit, two branch
commits, and a three-way merge. The 1,000-unit matrix tested independent edits
in different shards, independent edits in the same shard, nearby sorted IDs,
independent inserts in different and same shards, same-unit edits, and
delete-versus-edit.

| Candidate | Independent edits | Independent inserts | Same unit | Delete vs edit |
| --- | --- | --- | --- | --- |
| One file/unit | all three edit cases succeeded | both insert cases succeeded | conflict | conflict |
| Single JSONL | different-shard and same-shard cases succeeded; nearby conflicted | different-shard succeeded; same-shard conflicted | conflict | conflict |
| Sharded JSONL | different-shard and same-shard cases succeeded; nearby conflicted | different-shard succeeded; same-shard conflicted | conflict | conflict |
| Sharded pretty JSON | different-shard, same-shard, and nearby edit cases succeeded | different-shard succeeded; same-shard conflicted | conflict | conflict |

Conflicts for same-unit and delete-versus-edit are intentional and safer than
silently combining incompatible translation state. Nearby-record and
same-shard insertion conflicts are ordinary textual merge boundaries, not data
loss; a user must resolve them. The one-file layout avoids those same-file
textual boundaries, but its linear file-count cost is unacceptable at the
expected scale.

## Decision

Sharded JSONL is selected because it is the best ordered compromise:

- it bounds tracked unit files at 256 instead of creating one file per unit;
- it keeps a single-unit mutation to one compact JSONL record and one file;
- it preserves deterministic ordering and straightforward streaming parsing;
- it makes unrelated records in other shards independent Git merge surfaces;
- it avoids the substantially larger diffs and bytes of pretty JSON;
- it remains inspectable as ordinary UTF-8 JSON without a custom text language.

Single JSONL was rejected because one file is a merge bottleneck for inserts
and nearby edits. One file per unit was rejected because hundreds of thousands
of tracked files create obvious filesystem, checkout, indexing, and Git
metadata overhead. Pretty JSON was rejected because its human-readability
benefit did not justify multi-line record churn and larger repository state.

The exact selected layout, syntax, ordering, escaping, nullable fields, and
shard derivation are therefore frozen in `workspace-v1.md`. Production
reader/writer implementation is intentionally deferred to the next change.
