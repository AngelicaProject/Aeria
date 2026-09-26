# Git collaboration

Git is Aeria's collaboration and history layer. Aeria does not require a proprietary collaboration server.

## Hosting

The core workflow must work with an ordinary Git remote. Forge-specific integrations such as GitHub/GitLab pull-request creation are optional adapters and must not become core requirements.

## What gets committed, and when

A **checkpoint is the only way Aeria commits.** Nothing else — saving pack,
font, or collaboration settings, editing the glossary or guidance, recording a
signing key, or reconciling merged translations after a sync — creates a
commit. Those actions only write files; the files show up as uncommitted
changes, described in readable form, until the translator creates a
checkpoint. The export requires the files it builds from to be committed and
points to the uncommitted changes instead of committing them.

"Checkpoint" and "contribution branch" name Aeria's operations in this
documentation and in code. The interface calls them what Git users know: a
checkpoint is a commit (the button is labelled Commit), and a contribution
branch is a branch for a pull request.

## Git in the desktop

- The **Git dock** is the whole everyday view and follows the repository on
  its own (below): the branch, which can be switched there, sync state and
  Sync, the uncommitted changes (translations grouped by sheet, and project
  files grouped by area), the checkpoint composer, and the project history
  with a commit graph. Clicking a commit opens it in a document tab.
- **Settings → Repository** holds the setup: remotes, the upstream, the main
  branch, the translator identity, local branches, the
  [command-line merge driver](#command-line-merge-driver), and working-tree
  files outside the project.
- The **string history** (who translated and reviewed a string, and every
  committed change to it) is a tab beside the note in the translation editor.

The dock polls a fingerprint of `git status --porcelain=v2 --branch` (branch,
`HEAD`, upstream, ahead/behind, changed files) every two seconds while the
window is visible and when it gains focus, and reloads only when the
fingerprint changed; it has no refresh button. Changes made outside Aeria
therefore show up by themselves.

## Collaboration policy

Translations reach the main branch (`main` or `master`) **only through pull
requests**. Nobody commits to it directly: a checkpoint on the main branch
starts a contribution branch, and Aeria never pushes local commits to the
published main branch. There is no direct mode.

## Attribution and credentials

Aeria exposes the Git author name and optional email as the translator identity. Prefer established OS/Git credential mechanisms. Do not become a private-key/password manager unless a future requirement clearly justifies it.

## Implementation

`aeria-git` drives the Git command-line client instead of embedding a Git
library. Repositories, hooks, SSH configuration, and OS/Git credential helpers
therefore behave exactly as they do for other Git tools, and Aeria never
stores credentials. Every invocation runs with the project root as its working
directory, English diagnostics, `GIT_TERMINAL_PROMPT=0`, no stdin, and (on
Windows) no console window. Porcelain formats are parsed; user-supplied
remote names, URLs, branch names, identity values, and revisions are
validated before they reach Git, and remote-helper transports such as `ext::`
are rejected. A remote URL may contain spaces only when it is a local path
(an absolute Unix path, a Windows drive path, or a UNC path), because folder
names may; paths of any script are accepted. Git error messages name only the subcommand, never its
arguments, so remote URLs with embedded credentials are not echoed.

The project root may be the repository top level or a subdirectory of it.
Status and history are restricted to the project root, and paths are reported
relative to it.

### Git runtime

Translators are not expected to install Git. The executable is selected in
this order:

1. `AERIA_GIT_PATH`, an explicit development/test override;
2. the Git runtime bundled with Aeria;
3. `git` on `PATH`.

On Windows the installer bundles a pinned MinGit distribution (Git for
Windows) as the `git/` application resource and uses `git/cmd/git.exe`.
MinGit includes Git Credential Manager and configures
`credential.helper=manager`, so HTTPS remotes on GitHub, GitLab, Bitbucket,
and Azure DevOps sign in through the browser and store credentials in the
Windows credential store. The bundled OpenSSH uses the user's `~/.ssh`
configuration. On Linux, Git is a declared package dependency of the `.deb`
and `.rpm` bundles and is taken from `PATH`; other distributions must provide
it. The overview reports the Git version and where it came from, or that Git
is unavailable. Packaging details are in
[`../development/releases.md`](../development/releases.md).

### Repository setup

- **Initialize** runs `git init` (branch `main` unless `init.defaultBranch`
  is configured) and appends `/.aeria/** text eol=lf` to the root
  `.gitattributes`, so workspace files stay LF-only even with
  `core.autocrlf=true` (the MinGit default). Nothing is committed.
- **Clone** clones a remote into a new folder named after the repository, as
  `git clone` would (the last URL segment without `.git`); an existing
  non-empty folder is never overwritten. The parent folder is optional and
  defaults to `Documents/Aeria`. The launcher's Clone project form takes only
  the remote URL and the optional parent, then opens
  the clone through the same flow as Open project (see
  [`desktop-editor-ui.md`](./desktop-editor-ui.md#launcher)). When the clone
  succeeds but opening fails, the launcher continues in the Open project form
  with the cloned repository, so the remote is never cloned twice.

### Translator identity

The translator identity is the Git author identity. A name (`user.name`) is
required; the email is optional. Without an email, Aeria stores an explicitly
empty `user.email` and commits with `user.useConfigOnly=true`, so Git never
derives an address from the local account and host name; such commits show
the author as `Name <>`. The identity can be set for the project repository
or globally, and Aeria reports which configuration scope supplies it. Aeria
never invents an author.

### Checkpoint

A checkpoint stages and commits only Aeria-managed paths (`git commit
--only`): `.aeria/` and the project files `.gitattributes`,
`aeria-collaboration.json`, `aeria-pack.json`, `aeria-fonts.json`, the
`fonts/` directory of source fonts, `aeria-glossary.csv`,
`aeria-guidance.md`, and the feed workflow
`.github/workflows/harmonia-feed.yml` (`PROJECT_PATHS`). Unrelated staged or modified files are
left untouched and listed in Settings → Repository as other files. A blank
message is replaced by a deterministic summary: the translation-unit changes,
for example `Translate 3 strings, update 1 translation (Addon, Quest)`,
followed by the changed project areas (`; update glossary, game fonts`), or
`Update glossary, pack settings` when no translation changed.

Whether a checkpoint first moves to a contribution branch follows the
collaboration policy committed in `HEAD`, not the working copy, so committing
a policy change does not move its author to a contribution branch.

### Project file changes

Uncommitted changes (`HEAD` against the working tree) and each commit's
changes (against its first parent) are shown for project files in readable
form, computed in the desktop (`project_changes.rs`):

- the glossary is compared by term: added, removed, and changed entries with
  translation, note, and forbidden translations;
- guidance and `.gitattributes` are compared by line;
- pack, font, and collaboration settings are compared by field, with list
  entries keyed by their `id` or `font` (`fonts › MiedingerMid › source:
  tektur → unbounded`);
- font files are reported as added, replaced, or removed with their size;
- the feed workflow is reported as added, updated, or removed.

A file that cannot be parsed is reported as changed but unreadable.

### Semantic changes, history, and attribution

Because the workspace format stores one unit per JSONL line in a shard chosen
by its stable ID, unit-level views are derived directly from Git data:

- **Pending changes** compare each changed shard in the working tree with
  `HEAD` and report added, modified, and removed units with before/after
  target, review state, and note.
- **Commit changes** compare a commit with its first parent the same way.
- **Unit history** lists the commits that changed one unit's record, newest
  first, with author, time, message, and the record before and after
  (`git log -G<id> -p` on the unit's shard). Merge commits are not listed; a
  change is attributed to the commit that authored it. A historical record
  that is not valid workspace data is reported as invalid, never
  repaired. The uncommitted working-tree change is reported separately.
- **Attribution** of a committed unit has three parts:
  - *translated by*: the newest commit whose change introduced the unit's
    current target text;
  - *reviewed by*: when the unit is `reviewed`, the newest commit that
    changed it to `reviewed` with its current target text;
  - *last changed by*: the newest commit that changed the unit record.

  Project-wide attribution reads the complete unit history once
  (`git log -p` over `.aeria/units`); the desktop caches it per `HEAD`.
- **Contributors** count current committed units per author: how many
  current texts each author translated and how many current reviews each
  author made.

Attribution records who authored repository changes. It never sets or
infers review state; review remains an explicit workspace operation.

All shard and record decoding goes through the workspace reader in
`aeria-workspace`; Git history never bypasses format validation. History may
predate [Workspace Format v2](../formats/workspace-v2.md), so historical
records are accepted in either the v2 or the v1 record shape.

### Sync

Sync is fetch, integrate, push:

1. Fetch the current branch's remote (its configured remote, else `origin`,
   else the only remote).
2. Integrate incoming commits: the branch's upstream and, on a contribution
   branch under the pull-request policy, the remote main branch.
   Uncommitted translation changes block integration. A branch that is only
   behind fast-forwards; a diverged branch is merged. Unit shards that
   conflict textually are merged per translation unit (see
   [Semantic merge](#semantic-merge)). Any other conflicted file, or a
   same-unit conflict without an explicit resolution, aborts the merge and
   leaves the repository unchanged. After everything merged the caller
   validates the resulting project (the desktop reloads the
   `ProjectSession`, including source compatibility and source guidance).
   If validation fails, the branch is reset to its starting commit with
   `git reset --merge`. Validation may reconcile merged units with the
   session source (see below); such changes are not committed. They stay in
   the working tree like any other change, the sync result says so, and they
   reach the remote with the next checkpoint and sync. Until then the next
   sync is blocked by the uncommitted translations.
3. Push local commits. A branch without an upstream is published to the
   sync remote and tracked. A push rejected by the remote (for example a
   protected branch) is reported as a Git failure.

Sync never rebases, force-pushes, or accepts incoming changes that bind the
workspace to different source content.

The steps are also available one by one, with the same rules: **Fetch** runs
step 1 only; **Pull** runs steps 1 and 2, including the per-unit merge,
explicit conflict resolutions, and project validation, and never pushes;
**Push** runs step 3 and refuses while the upstream, as last fetched, has
commits the branch lacks, so a push never needs to be forced. Pull is never
a plain `git pull`: translations always merge per unit.

Incoming translations made against an older game version, merged into a
branch that already applied the newer version, are not rejected and are not
trusted as current. Their units still record the old source facts, which the
reload detects as a source facts mismatch
([`rebase.md`](./rebase.md#when-an-update-is-required)). The desktop reloads
with `ProjectSession::reload_and_reconcile_workspace`, which applies the
deterministic source update to exactly that state: an edit made on the old
text keeps its target and is marked for review, and two units bound to one
occurrence keep one deterministic owner while the other is detached with its
target and note. No translation is ever overlaid on text it was not made for,
and no unit is removed. The same check runs when a project is opened, so
state merged outside Aeria (for example a pull request merged on the Git
host) is handled identically. Such changes are rejected and rolled
back until the local project is updated to that source through the explicit
[source update](./rebase.md) workflow. Because a source update is
deterministic, a collaborator who updates to the same source produces the
same files, and the following sync merges cleanly.

## Semantic merge

When Git reports a conflicted unit shard, Aeria reads the base, local, and
incoming versions from the index stages, decodes them with the workspace
reader, and merges them per `TranslationUnitId`. Merged shards are always
written in the current format. Edits to
different units therefore never conflict, even when their lines are adjacent.
For one unit, in order:

1. Identical sides, or a side equal to the base, take the other side.
2. Adding or removing the unit on one side while the other side changed it
   differently is a conflict.
3. The source facts (status, binding, fingerprint, layout, and row key) must be
   identical on both sides. Two sides that applied the same source update
   agree, and their remaining fields merge by the rules below. A source
   change on only one side conflicts with any other change on the other
   side.
4. Target and review state merge as one pair. When only one side changed the
   target, that side's target and review state win, because a new target
   invalidates a review of the old one. Different targets on both sides, or
   the same target with different review changes, are a conflict.
5. The translator note merges independently with the same three-way rule.

Merged shards are written in canonical form and committed as the merge
commit. Same-unit conflicts are reported with their base, local, and incoming
versions. They are resolved only by an explicit per-unit choice of the local
or incoming version, supplied to a repeated sync; Aeria never chooses on its
own. Conflicts in any other file, including `.aeria/manifest.json`, abort the
merge.

### Command-line merge driver

Aeria's own merges (Sync, Pull, and local merges into the main branch) merge
unit shards per unit as above. Command-line `git merge` and `git pull` merge
them as text unless the repository uses Aeria's merge driver, which
**Settings → Repository → Command-line merges** turns on:

- It sets `merge.aeria-units.name` and `merge.aeria-units.driver` in the
  repository's own configuration, local to the machine, to run the Aeria
  executable as `"<aeria>" merge-driver %O %A %B %P` (forward slashes, run by
  Git's shell). Opening a project points an enabled driver at the running
  executable again, so a moved or updated Aeria keeps working. Turning it
  off removes the configuration section.
- It appends `/.aeria/units/*.jsonl merge=aeria-units` to `.gitattributes`,
  which the next checkpoint commits. Where the driver is not configured,
  Git treats the unknown driver as a text merge, so the rule changes nothing
  for other collaborators or on a Git host.
- `merge-driver` (in `aeria` and in `aeria-check`) merges the three versions
  with the rules of this section (`merge_shard_for_driver`) and writes the
  canonical result. A same-unit conflict is not resolved: the unit's local,
  base, and incoming records are written between `<<<<<<< ours`,
  `||||||| base`, `=======`, and `>>>>>>> theirs`, and the driver exits 1,
  so Git reports the file as conflicted. A version that is not a valid shard
  exits 2 and leaves the local version for Git to report. An absent version
  is an empty file, as Git passes it.

A Git host never runs a repository's merge drivers.

## Branches and contributions

The Git dock switches between local branches. A switch requires checkpointed
translations, reloads the project, and is undone when the reloaded project is
not valid for the active source package.

The **main branch** is the one set in
[`aeria-collaboration.json`](../formats/collaboration-v1.md); without a
setting it is detected: the sync remote's default branch (`<remote>/HEAD`),
else a local or remote `main`, else `master`, else the current branch of a
repository without commits. Settings → Repository shows which applies and
can set it; saving writes the file, and the next checkpoint commits it. The
checkpoint decides by the main branch committed in `HEAD`, so committing a
setting does not redirect its own checkpoint.

- A checkpoint on the main branch first creates a contribution branch named
  `translations/<name>-<UTC timestamp>` from the translator name (ASCII
  letters and digits; otherwise the email local part or `translator`) and
  commits there. The only commit made on the current branch directly is the
  first commit of a repository; when the project names a main branch and the
  unborn branch has another name (for example `git init` chose `master` and
  the project says `main`), the first checkpoint starts the named branch.
- Upstreams need no setup: the first sync of a branch publishes it and sets
  its upstream. Settings → Repository can point a branch with commits at
  another fetched remote branch, and explains when there is nothing to choose
  (no commits yet, or an empty remote).
- Sync publishes the main branch only while the remote does not have it yet.
  When the published main branch has local commits, push is refused
  (`MainBranchProtected`) and the commits must move to a contribution branch.
- Sync keeps a contribution branch up to date with the remote main branch and
  publishes it for review on the hosting service. The contribution status
  reports whether the branch is published, how many of its commits the
  remote main branch does not contain yet, and how many commits of the
  remote main branch, as last fetched, the branch does not contain.
- The hosting service merges pull requests as plain text, so once another
  contribution reached the main branch, a pull request whose unit shards
  changed nearby shows textual conflicts even when no translation unit
  conflicts. Syncing the contribution branch merges main per unit and
  resolves them; the hosting service's own conflict resolution must not be
  used, because it would commit conflict markers into unit shards. While
  the Git dock shows a contribution branch with a remote, it therefore
  fetches only the remote main branch's ref every five minutes and when the
  window gains focus (at most once a minute), with credential prompts
  disabled so it never asks to sign in, and fetch failures are ignored.
  When main has commits the branch lacks and the branch still waits for
  review, the dock asks the translator to sync and warns against the
  hosting service's update and conflict-resolution buttons. **Finish contribution** switches
  back to the main branch, fast-forwards it, and deletes the contribution
  branch only when Git confirms it is merged; branches merged by squash are
  kept.

- **Without a remote** there is nowhere to open a pull request. The Git dock
  then offers **Merge into `<main>`** on a contribution branch, after an
  explicit confirmation: Aeria switches to the main branch, integrates the
  contribution (a fast-forward when the main branch has not moved, otherwise
  a merge with per-unit merging of translations), validates the reloaded
  project, and deletes the merged branch. A rejected project resets the main
  branch and returns to the contribution branch. As soon as the repository
  has a remote, this action is unavailable and pull requests are the only
  way in.

- Branches are deleted only on request. "Finish contribution" and "Merge
  into `<main>`" delete the branch they finish; otherwise Settings →
  Repository lists the other local branches, whether each is merged into the
  main branch (`merge-base --is-ancestor`), and deletes one after a
  confirmation. A branch with commits outside the main branch needs a second,
  explicit warning. Branches on the remote are never deleted by Aeria.

Several translators may share one contribution branch; they sync with each
other through it exactly as described under Sync. Protecting the main branch
on the hosting service (for example GitHub branch protection) is still
recommended, because Git tools other than Aeria are not bound by these rules.
Opening the pull request itself is left to the hosting service or a future
optional adapter.

## Merge check CI

A Git host merges a pull request as plain text and lets files be edited on
its website, without Aeria's per-unit merge or project validation. A text
merge of unit shards that happens to apply can still leave a project Aeria
cannot open, or remove translations. The merge check catches that before the
merge.

`aeria-check` (crate `aeria-check`) runs Aeria's own strict readers without
the game source, in three stages:

1. **Integrity**: no Git conflict markers in `.aeria/` or any project file;
   the manifest reads; Collaboration, Pack, and Font Settings are valid, and
   every font file the font settings name exists.
2. **Translations**: the workspace loads strictly, as when Aeria opens it:
   every unit and identity, unique bindings, and every target as a valid
   structured string. Every managed file must also be byte for byte what
   Aeria writes (`WorkspaceStore::non_canonical_files`), because a readable
   but non-canonical file was edited or merged outside Aeria.
3. **Merge**: compared with the base revision (`--base`), a change of the
   project languages or of the game source (a source update) and units the
   base has but the result lacks are warnings for the reviewer. Aeria never
   removes units, so removed units point to a bad merge or a manual edit.

Errors fail the check; warnings and notices do not. In GitHub Actions the
findings become annotations on the files and each stage adds a section to
the job summary. Checks that need the game source, such as tags matching the
original text, stay with Aeria, which runs them whenever it opens, pulls, or
syncs the project; a stage with the game source is not offered.

For a repository whose `origin` is on github.com and whose project is the
repository's top folder, the Git dock offers
`.github/workflows/aeria-check.yml`. The workflow checks out GitHub's test
merge of a pull request with its base (`fetch-depth: 2`, so `HEAD^1` is the
base), downloads the `aeria-check` archive built with the same Aeria release,
verifies its SHA-256, and runs the stages as separate steps; the merge stage
runs only for pull requests. It also runs on pushes. The URL and SHA-256 come
from the release build (`AERIA_CHECK_URL`, `AERIA_CHECK_SHA256`), so
development builds cannot offer the workflow. Like the feed workflow it is a
project file: installing only writes it, the next checkpoint commits it, and
the dock offers an update when the file differs from what this Aeria writes.
Making the check required is a branch protection setting on GitHub, which
Aeria cannot change; the dock links to the repository's branch settings and
also recommends "Require branches to be up to date before merging". With it,
a pull request merges only when its branch already contains the base, which
Sync or Pull in Aeria achieves by merging per unit; GitHub's merge then
produces exactly the branch's tree, so no text merge of shards happens on
the host at all and the check remains a second line of defense.

## Remotes and upstream

Settings → Repository lists every remote with its URL, which can be edited
(`git remote set-url`) or removed (`git remote remove`; nothing is deleted on
the server), and add a remote. The upstream of the current branch, which sync
receives from and pushes to, is chosen from the remote-tracking branches
after fetching every remote (`git fetch --all --prune`) and set with
`git branch --set-upstream-to`.

## History

The project history lists commits of `HEAD` newest first in topological
order in the Git dock, loaded in pages of 100 as the list scrolls, with branch and tag names
(`%D`). A project at the repository top level shows the complete history,
merges included; a project in a subdirectory shows its path-limited history
with rewritten parents (`--parents`) so the graph stays connected. The graph
lanes are laid out in the renderer (`commitGraph.ts`). Clicking a commit
opens it in a document tab with its translation changes and project file
changes; one unpinned commit tab is reused while browsing.
