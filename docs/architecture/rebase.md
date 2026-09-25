# Source update

Game updates are a primary product workflow. Every patch changes the content
of existing sheets and adds new ones, and occasionally changes a sheet's
column layout, removes rows, or removes sheets. Aeria cannot predict these
changes, so a source update must succeed for any verified new source, never
lose translation work, and never attach a translation to a string it was not
written for.

A source update moves a project from the source facts its units were last
bound to onto one new verified HSP. It is deterministic, conservative, and
auditable. The rules that decide where each unit goes are specified in
[`rebase-safety.md`](./rebase-safety.md).

## Guarantees

- **Nothing is lost.** A source update never removes a unit and never
  changes a unit's `TranslationUnitId`, target text, or translator note.
- **No false attachment.** A unit is attached only to an occurrence
  established by a deterministic rule. Similarity, ordering, and nearby
  coordinates never attach a unit; AI never takes part.
- **Always completable.** A unit without an established occurrence is
  *detached*: it keeps its last bound source facts and stays in the project.
  An update therefore never has to stop for manual reconciliation.
- **Visible outcome.** Changed source text marks the unit `needs-review`, and
  every detached unit is listed with its reason. A change of the raw source
  bytes alone keeps the review state, because the translation is written
  against the macro text.
- **Lines, not row numbers.** In sheets whose rows carry a stable text key,
  such as quest and cutscene dialogue, a translation follows its line when
  inserted lines renumber the rows.
- **Only the new source is required.** Planning uses the source facts
  persisted in the workspace and the new verified snapshot. The previous
  snapshot is not needed, because a game update overwrites the installation
  that produced it. A collaborator who never had the previous snapshot can
  update the project.
- **Idempotent and interruptible.** Planning the same inputs again produces
  the same plan. An interrupted update is detected and planned again on the
  next open.

## When an update is required

Opening a project compares the stored workspace with the chosen HSP. A
source update is required when any of these is true:

| Requirement | Condition |
| --- | --- |
| Format migration | The workspace uses Workspace Format v1. |
| Content changed | The manifest `contentId` differs from the HXS `contentId`. |
| Source facts mismatch | The content is unchanged, but a bound unit does not describe the source: its sheet, String column, or row is missing, its layout or fingerprint differs, or another bound unit claims the same occurrence. |
| Permission changed | The content is unchanged, but the HSP guidance no longer permits a bound unit. |

Ordinary editing and source updates never produce a source facts mismatch.
A Git merge does when it combines work done against different game
versions: for example, a branch that translated on the previous version is
merged into a branch that already applied the patch (in Aeria or on the Git
host). The merged manifest then records the new `contentId` while some
units still record the old source. Opening verifies every bound unit against
the source, reading only sheets that hold bound units through hash-only
pages, and reports the mismatch instead of loading the state as current. The
same deterministic update then reconciles it: a changed occurrence keeps its
target and is marked for review, a missing one is detached, and a binding
claimed twice keeps one deterministic owner while the other unit is detached
as `BindingConflict`. The result equals what the other branch would have
produced by applying the update itself, so collaborators converge on the same
files.

Compatibility is decided by `contentId`, not by `snapshotId`. A game version
whose extracted content is identical (a typical hotfix) opens directly
without an update, and collaborators on different game builds with identical
content can share one project. A different source language is not an update;
it is an incompatible project and fails to open.

`ProjectSession::open` and `open_from_source_package` never write. When an
update is required they fail with `SourceUpdateRequired` and the requirement.
`ProjectSession::preview_source_update` builds the plan without writing, and
`ProjectSession::open_with_source_update` applies it and opens the project.

## Planning

`aeria-rebase::plan_source_update` is pure. It borrows the workspace
metadata, the managed units, the new verified `HxsSnapshot`, and the
translation-permission predicate of the new HSG, and returns an owned
`SourceUpdatePlan`. It never mutates its inputs.

The planner reads only the sheets that contain managed units, through the
bounded hash-only String occurrence pages of `aeria-hxs`. Each unit, bound or
detached, receives exactly one outcome:

| Outcome | Meaning | Applied as |
| --- | --- | --- |
| `Unchanged` | Bound to an occurrence with the same macro text and raw value. | Source facts refreshed; review state kept. |
| `EncodingChanged` | Bound to an occurrence with the same macro text whose raw source bytes changed. | Source facts refreshed; review state kept. |
| `SourceChanged` | Bound to an occurrence whose macro text changed. | Source facts refreshed; review state `needs-review`. |
| `Detached(reason)` | No occurrence was established. | Status set to the reason; last bound facts kept. |

Each bound outcome records its continuity in two parts: the row (the same
row, or a row found by the unit's row key) and the column (the same column in
an unchanged schema, or a column established by a sheet-level mapping with
its evidence). The plan also lists every sheet whose units were bound in
another schema generation, with its column mappings. `SourceUpdateSummary`
counts unchanged, encoding-changed, source-changed, detached, newly detached,
reattached, column-mapped, row-moved, and changed units.

## Applying

`ProjectSession::open_with_source_update` applies a plan atomically per file
and consistently as a whole:

1. Read and validate the stored workspace in either supported format.
2. Require the same source language.
3. Plan the update against the new source and its guidance.
4. Build the next workspace: the new `contentId`, and each changed unit
   rebound (`Unchanged`, `EncodingChanged`, `SourceChanged`) or detached.
5. Publish every changed shard, and every shard when migrating the format,
   with atomic file replacement.
6. Publish the manifest last.
7. Reload the published state and require it to equal the planned workspace.

If the process stops before step 6, the manifest still names the previous
content, so the next open requires the update again. Units already rewritten
are recognized as current by their source layout and keep their new facts;
the repeated plan completes the remaining units under the same rules. Bound
units are unique by binding within one schema generation, which admits this
intermediate state; see
[`workspace-v2.md`](../formats/workspace-v2.md#reader-and-validation-contract).

Applying an update is not a Git operation. The changed files are ordinary
working-tree changes that the user reviews and checkpoints like any other
edit. Because the update never discards target text or notes, Git history
remains the recovery point for review states.

## Collaboration

Planning is deterministic, so every collaborator who updates the same
workspace to the same source produces identical files. Semantic merge accepts
a unit whose source facts changed identically on both sides and merges its
target, review state, and note as usual; see
[`git.md`](./git.md#semantic-merge). Sync never applies a source update
itself: incoming changes that bind the workspace to other content are
rejected until the local project is updated to that source.

## Desktop workflow

The desktop offers two ways to update:

- **Update from game.** The launcher runs Harmonia Atlas for an existing
  repository and the installed game, using the source language recorded in
  the project, then opens the project with the update applied.
- **Open with other content.** Opening a project from Recent projects with a
  package whose content differs fails with `sourceUpdateRequired`. Open
  project and Clone project build the package from the game when Aeria has
  none matching the workspace, and return the plan without writing when that
  package differs. In both cases the launcher previews the plan, shows the
  counts, and applies the update only after confirmation. This is how a
  collaborator starts on a cloned repository built from another game
  version.

After an update the workbench shows a summary. The status bar shows the
number of detached translations, and the detached list shows each unit's last
location, reason, and target text. Manual reattachment is not implemented;
detached units are re-evaluated by every later update.

## Row keys

Some sheets number their rows as a sequence and identify each line by a text
key in a String column, for example `TEXT_..._000_000` in quest dialogue.
Inserting a line renumbers every later row. Aeria detects such a *row key
column* deterministically (see
[`rebase-safety.md`](./rebase-safety.md#row-keys)), stores the key of the
unit's row with the unit, and on update finds the row by key instead of by
row ID. A key that no longer exists means the line was removed, and the unit
is detached even if its old row ID now holds a different line.

A unit gets its row key when it is created in a keyed sheet and from every
source update. Units without a key, such as units migrated from Workspace
Format v1, keep their row IDs in the first update and receive keys from it.
In a sheet without a row key column, inserted rows still shift text under
existing row IDs; that text is reported as `SourceChanged` and marked for
review rather than silently moved.

## Review suggestions

Bounded candidate suggestions for a detached unit are documented in
[`rebase-candidates.md`](./rebase-candidates.md). They require the previous
snapshot for text ranking and are review assistance only; they never change a
plan.
