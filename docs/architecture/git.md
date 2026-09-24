# Git collaboration

Git is Aeria's collaboration and history layer. Aeria does not require a proprietary collaboration server.

## Hosting

The core workflow must work with an ordinary Git remote. Forge-specific integrations such as GitHub/GitLab pull-request creation are optional adapters and must not become core requirements.

## Simple mode

Simple mode presents domain-oriented operations such as:

- Changes
- Checkpoint
- Sync
- Share
- Review

It still operates on a real Git repository.

## Advanced mode

Advanced mode exposes conventional source-control concepts:

- working tree
- staging/index
- commits
- branches
- remotes
- history
- merge/rebase/conflicts

## Collaboration policy

A project may declare a default policy such as:

- `direct`: work in the current branch and push directly.
- `pull-request`: create/use a contribution branch and guide the user toward review before integration.

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
are rejected. Git error messages name only the subcommand, never its
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
- **Clone** clones a remote into a new directory. The project is then opened
  through the ordinary open flow with a local HSP.

### Translator identity

The translator identity is the Git author identity. A name (`user.name`) is
required; the email is optional. Without an email, Aeria stores an explicitly
empty `user.email` and commits with `user.useConfigOnly=true`, so Git never
derives an address from the local account and host name; such commits show
the author as `Name <>`. The identity can be set for the project repository
or globally, and Aeria reports which configuration scope supplies it. Aeria
never invents an author.

### Checkpoint

A checkpoint stages and commits only Aeria-managed paths: `.aeria/`,
`.gitattributes`, and `aeria-collaboration.json` (`git commit --only`).
Unrelated staged or modified files are left untouched. A blank message is
replaced by a deterministic summary of the translation-unit changes, for
example `Translate 3 strings, update 1 translation (Addon, Quest)`.

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
   session source (see below); such changes are committed right after the
   merge with the message `Reconcile translations with the current game
   source`, so the pushed history is consistent.
3. Push local commits. A branch without an upstream is published to the
   sync remote and tracked. A push rejected by the remote (for example a
   protected branch) is reported as a Git failure.

Sync never rebases, force-pushes, or accepts incoming changes that bind the
workspace to different source content.

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

## Branches and collaboration policy

Advanced mode lists local and remote-tracking branches, creates a branch at
the current commit (uncommitted work moves with it), and switches branches.
A switch requires checkpointed translations, reloads the project, and is
undone when the reloaded project is not valid for the active source package.

The project-shared policy is stored in
[`aeria-collaboration.json`](../formats/collaboration-v1.md) and committed
with the project. Without the file the policy is `direct`.

- `direct`: checkpoints are committed to the current branch and sync pushes
  it.
- `pull-request`: a checkpoint on the main branch first creates a
  contribution branch named `translations/<name>-<UTC timestamp>` from the
  translator name (ASCII letters and digits; otherwise the email local part
  or `translator`) and commits there. Sync keeps the contribution branch up
  to date with the remote main branch and publishes it for review on the
  hosting service. The contribution status reports whether the branch is
  published and how many of its commits the remote main branch does not
  contain yet. **Finish contribution** switches back to the main branch,
  fast-forwards it, and deletes the contribution branch only when Git
  confirms it is merged; branches merged by squash are kept.

The policy guides Aeria's workflow; enforcement, such as protected branches,
belongs to the hosting service. Opening the pull request itself is left to
the hosting service or a future optional adapter.
