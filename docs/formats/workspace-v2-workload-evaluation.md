# Workspace Format v2 workload evaluation

Status: **measurement; informs future format work, changes nothing**.

The [v1 layout evaluation](./workspace-v1-evaluation.md) compared layouts on
random edits to 1,000 units. This evaluation measures the current format
under the work Aeria projects actually see: a large, partly translated
corpus, AI jobs that draft a whole sheet at once, and translators working in
parallel. It also measures two alternatives for a possible future format.

## Reproduction

```text
cargo run --release -p aeria-workspace-workload-evaluation -- --scale 1
```

`tools/workspace-workload-evaluation` generates a deterministic corpus of
eight sheets shaped like a large game (Item 40,000 rows × 2 strings,
QuestText 60,000 × 1, Action 20,000 × 2, and smaller ones; 248,000 string
cells), translates half of each sheet for the base, and writes it with the
real `aeria-workspace` writer. Git runs with `core.autocrlf=false`; packed
sizes are after `git gc`. Per-unit merge results use Aeria's merge
(`merge_shard_for_driver`), the same rules as Sync, Pull, and the
command-line merge driver.

Layouts:

- **current (v2)**: 256 shards by unit ID, canonical records.
- **file per sheet**: one file per sheet, records sorted by row, the same
  canonical records.
- **compact records**: 256 shards by unit ID with positional records, hashes
  in base64, and the sheet schema hash in a per-sheet table.

## Results (scale 1, 124,235 units in the base)

A canonical record averages 836 bytes: hex hashes 35 %, JSON field names
31 %, and targets 25 %.

| Layout | Files | Working tree | Packed | AI job: files changed | AI job: lines added | AI job: packed growth |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| current (v2) | 257 | 99.1 MiB | 25.5 MiB | 256 | 39,892 | 32.0 MiB |
| file per sheet | 9 | 99.1 MiB | 24.1 MiB | 1 | 39,892 | 12.6 MiB |
| compact records | 258 | 47.8 MiB | 18.7 MiB | 256 | 39,892 | 5.5 MiB |

The AI job drafts every untranslated cell of Item (39,892 strings).

Parallel branches from the same base: files Git's text merge leaves
conflicted, and for the current layout the units Aeria's per-unit merge
leaves to a person.

| Scenario | current: text | current: per-unit | file per sheet: text | compact: text |
| --- | ---: | ---: | ---: | ---: |
| Two translators, different sheets (300 edits each) | 0 | 0 | 0 | 0 |
| Two translators, same sheet, different strings (300 each) | 1 | 0 | 1 | 1 |
| Two AI jobs, different sheets (Addon and Status) | 224 | 0 | 0 | 224 |
| Two AI jobs, halves of one sheet (Item) | 256 | 0 | 0 | 256 |
| Control: both edit the same 20 strings | 19 | 20 | 1 | 19 |

## Findings

- **Per-unit merge makes the layout's text conflicts irrelevant.** Aeria's
  merge left no conflicts except the real ones in the control scenario.
  Conflicts only matter where Git merges text: on a Git host, or on the
  command line without the [merge driver](../architecture/git.md#command-line-merge-driver).
- **Parallel AI jobs conflict almost everywhere as text** in any ID-sharded
  layout, because a job touches every shard. On GitHub this blocks the
  merge; it does not corrupt data. Requiring branches to be up to date
  before merging, with Sync or Pull in Aeria first, removes the host's text
  merge entirely (see [merge check CI](../architecture/git.md#merge-check-ci)).
- **One file per sheet** would avoid text conflicts between work on
  different sheets or different row ranges, would make a job's diff one
  file, and grows the repository by less per job. Its cost is very large
  files for large sheets and interleaved edits in one sheet still
  conflicting.
- **Compact records** halve the working tree and cut packed size by about a
  quarter and packed growth per job by about 80 %, at the price of records
  that no longer name their fields.
- At this scale the current format is workable: a 25.5 MiB packed
  repository for 124,000 translations, each job adding tens of MiB. Growth
  from repeated full-sheet jobs is the main long-term cost.

## Decision

No format change now. With per-unit merges in Aeria and on the command line,
and up-to-date branches on the host, the current layout's weakness (text
conflicts) does not reach users. Repository growth is the reason to revisit
it; a Workspace Format v3 would combine sheet-based files (possibly split by
row range for the largest sheets) with a compact record, and would need a
lossless migration like every format change.
