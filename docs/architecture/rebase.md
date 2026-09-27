# Source update

Game updates are a primary product workflow. Every patch changes the text of
existing sheets and adds new ones, and occasionally changes a sheet's String
columns, removes rows, or removes sheets. Aeria cannot predict these changes,
so a source update must succeed for any game version, never lose translation
work, and never attach a translation to a string it was not written for.

A source update moves a project from the game version its units were last
bound to onto the installed game. It is deterministic, conservative, and
auditable. The rules that decide where each unit goes are specified in
[`rebase-safety.md`](./rebase-safety.md).

## Guarantees

- **Nothing is lost.** A source update never removes a unit and never
  changes a unit's ID, target text, or translator note.
- **No false attachment.** A unit is attached only to a cell established by
  a deterministic rule. Similarity, ordering, and nearby coordinates never
  attach a unit; AI never takes part.
- **Always completable.** A unit without an established cell is *detached*:
  it keeps its last source facts and stays in the project. An update
  therefore never has to stop for manual reconciliation.
- **Visible outcome.** Changed source text marks the unit `needs-review`, and
  every detached unit is listed with its reason. The unit's previous source
  text remains in the Git history of its record.
- **Lines, not row numbers.** In sheets whose rows carry a stable text key,
  such as quest and cutscene dialogue, a translation follows its line when
  inserted lines renumber the rows.
- **Only the installed game is required.** Planning uses the source facts
  persisted in the workspace and the current game. A collaborator who never
  had the previous game version can update the project.
- **Forward only.** A project is updated only to a game version equal to or
  newer than the one it records. An older game never rewrites a project.
- **Idempotent and interruptible.** Planning the same inputs again produces
  the same plan. An interrupted update is detected and planned again on the
  next open.
- **Same checks as Harmonia.** A unit is bound by its text and the sheet
  layout, the same facts Harmonia checks (as bytes) before replacing a
  string, so an exported translation is applied exactly where Aeria bound it.

## When an update is required

Opening a project compares the manifest's `gameVersion` with the installed
game:

| Game | Result |
| --- | --- |
| older than the project | The project cannot be opened: `GameOutdated`. The user updates the game. |
| newer than the project | A source update is required: `GameUpdated`. |
| the same version | Every bound unit is verified against the game. |

Verification reads every sheet that holds bound units once. It requires a
source update when a bound unit does not describe the game: its sheet, String
column, row, layout, text, or key differs, another bound unit claims the same
cell (`SourceFactsMismatch`), or its cell is no longer translatable
(`PermissionChanged`).

Ordinary editing and source updates never produce a mismatch. A Git merge
does when it combines work done against different game versions: for
example, a branch that translated on the previous version is merged into a
branch that already applied the patch (in Aeria or on the Git host). The
merged manifest then records the new version while some units still describe
the old one. The same deterministic update reconciles it: a changed cell
keeps its target and is marked for review, a missing one is detached, and a
cell claimed twice keeps one deterministic owner while the other unit is
detached as `BindingConflict`. The result equals what the other branch would
have produced by applying the update itself, so collaborators converge on the
same files.

`ProjectSession::open` never writes. When an update is required it fails
with `SourceUpdateRequired` and the requirement.
`ProjectSession::preview_source_update` builds the plan without writing, and
`ProjectSession::open_with_source_update` applies it and opens the project.

## Planning

`aeria-rebase::plan_source_update` is pure. It borrows the workspace
metadata, the units, and the opened `GameSource`, and returns an owned
`SourceUpdatePlan`. It never mutates its inputs.

The planner reads only the sheets that hold units. Each unit, bound or
detached, receives exactly one outcome:

| Outcome | Meaning | Applied as |
| --- | --- | --- |
| `Unchanged` | Bound to a cell with the same text. | Source facts refreshed; review state kept. |
| `SourceChanged` | Bound to a cell whose text changed. | Source facts refreshed; review state `needs-review`. |
| `Detached(reason)` | No cell was established. | Status set to the reason; last facts kept. |

Each bound outcome records its continuity in two parts: the row (the same
row, or a row found by the unit's row key) and the column (the same column in
an unchanged layout, or a column established by a sheet-level mapping with
its evidence). The plan also lists every sheet whose units were bound in
another layout, with its column mappings. `SourceUpdateSummary` counts
unchanged, source-changed, detached, newly detached, reattached,
column-mapped, row-moved, and changed units.

## Applying

`ProjectSession::open_with_source_update` applies a plan atomically per file
and consistently as a whole:

1. Read and validate the stored workspace.
2. Require the same source language and a game version not older than the
   project's.
3. Plan the update against the game.
4. Build the next workspace: the game's version, and each changed unit
   rebound or detached.
5. Publish every changed shard with atomic file replacement.
6. Publish the manifest last.
7. Reload the published state and require it to equal the planned workspace.

If the process stops before step 6, the manifest still names the previous
version, so the next open requires the update again. Units already rewritten
describe the game and keep their new facts; the repeated plan completes the
remaining units under the same rules. Bound units are unique by binding
within one layout, which admits this intermediate state; see
[`workspace-v3.md`](../formats/workspace-v3.md#reader-and-validation-contract).

Applying an update is not a Git operation. The changed files are ordinary
working-tree changes that the user reviews and commits like any other edit.
Because the update never discards target text or notes, Git history remains
the recovery point for review states.

## Collaboration

Planning is deterministic, so every collaborator who updates the same
workspace to the same game version produces identical files. Semantic merge
accepts a unit whose source facts changed identically on both sides and
merges its target, review state, and note as usual; see
[`git.md`](./git.md#semantic-merge). Sync never applies a source update
itself: incoming changes that record another game version are rejected until
the local project is updated to that version.

## Desktop workflow

Opening a project that needs an update shows the plan counts and applies the
update only after confirmation. After an update the workbench shows a
summary. The status bar shows the number of detached translations, and the
detached list shows each unit's last location, reason, source text, and
target text. Manual reattachment is not implemented; detached units are
evaluated again by every later update.

## Row keys

Some sheets number their rows as a sequence and identify each line by a text
key, for example `TEXT_..._000_000` in quest dialogue. Inserting a line
renumbers every later row. Aeria detects such a *row key column*
deterministically (see [`source.md`](./source.md#row-keys)), stores the key
of the unit's row with the unit, and on update finds the row by key instead
of by row ID. A key that no longer exists means the line was removed, and the
unit is detached even if its old row ID now holds a different line.

In a sheet without a row key column, inserted rows still shift text under
existing row IDs; that text is reported as `SourceChanged` and marked for
review rather than silently moved.
