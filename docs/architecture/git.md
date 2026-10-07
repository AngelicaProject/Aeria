# Git collaboration

Git is Aeria's collaboration and history layer. Aeria does not require a proprietary collaboration server.

## Hosting

The core workflow must work with an ordinary Git remote. Forge-specific integrations such as GitHub/GitLab pull-request creation are optional adapters and must not become core requirements.

## What gets committed, and when

A **checkpoint is how Aeria commits** everyday work. Saving a translation,
pack, font, or collaboration settings, editing the knowledge, recording a
signing key, or a machine translation run only write files; the files show up
as uncommitted changes, described in readable form, until the translator
creates a checkpoint. The one other commit Aeria makes is an update of the
project to a new game version, which is recorded as one commit of its own
(see [`po-project.md`](./po-project.md#game-updates)). The export requires the
files it builds from to be committed and points to the uncommitted changes
instead of committing them.

"Checkpoint" names Aeria's commit operation in this documentation and in
code. The interface calls it what Git users know: a commit (the button is
labelled Commit). As in other Git tools, **Commit** takes what is staged
when anything is (see [Working-tree operations](#working-tree-operations));
with nothing staged it is a checkpoint.

## Git in the desktop

- The **Git dock** is the whole everyday view and follows the repository on
  its own (below): the branch, which can be switched there, and its state
  against the upstream with Fetch, Pull, and Push above two tabs: Changes,
  the commit composer over the uncommitted files (see
  [Changed files](#changed-files)) with the operations of
  [Working-tree operations](#working-tree-operations), and History, the
  project history with a commit graph, a search, and a file filter.
  Clicking a commit opens it in a document tab.
- **Settings → Repository** holds the setup: remotes, the upstream, the main
  branch, the translator identity, local branches, and working-tree files
  outside the project.
- The **string history** (every committed change to a string, with its
  author) is a tab beside the note in the translation editor.

The dock polls a fingerprint of `git status --porcelain=v2 --branch` (branch,
`HEAD`, upstream, ahead/behind, changed files) every two seconds while the
window is visible and when it gains focus, and reloads only when the
fingerprint changed; it has no refresh button. Changes made outside Aeria
therefore show up by themselves.

## Branches are ordinary Git

Aeria has no collaboration policy of its own. A checkpoint commits on the
current branch, Push pushes the current branch, and Aeria neither creates
branches on its own nor merges one branch into another (Pull integrates only
the current branch's upstream). Which branch work goes to,
and whether it reaches another branch through a pull request, is up to the
translators and the hosting service. Protecting a branch on the hosting
service (for example GitHub branch protection) is how a team requires pull
requests; a push the remote refuses is reported as a Git failure.

## Attribution and credentials

Aeria exposes the Git author name and optional email as the translator
identity. Who changed a string and when is the history of its PO file (see
[String changes and history](#string-changes-and-history)); Aeria keeps no
authorship of its own. Prefer established OS/Git credential mechanisms. Do not
become a private-key/password manager unless a future requirement clearly
justifies it.

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
  is configured) and appends `*.po text eol=lf` to the root
  `.gitattributes`, so PO files stay LF-only even with
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
--only`): `po/` and the project files `aeria.json`, `.gitattributes`, `aeria-pack.json`, `aeria-fonts.json`, the
`fonts/` directory of source fonts, the `aeria-knowledge/` directory of
[project knowledge](../formats/knowledge-v1.md), and the workflows
`.github/workflows/harmonia-feed.yml` and `.github/workflows/aeria-guard.yml`
(`PROJECT_PATHS`); a path without a file is left out, since Git has no empty
folders. Unrelated staged or modified files are left untouched and listed as
other files. A blank message is replaced by a
deterministic summary: the string changes, for example `Translate 3 strings,
update 1 translation (Addon, Quest)`, followed by the changed project areas
(`; update terms, game fonts`), or `Update glossary, pack settings` when no
string changed.

A file `aeria-collaboration.json` left by an earlier Aeria is not a project
file: Aeria neither reads nor commits it, and it can be deleted with Git.

### Changed files

Uncommitted changes (from `git status`) and each commit's changes (against
its first parent, from `git diff-tree`) are shown as the files Git reports,
each with its status (added, modified, deleted, renamed, copied, type
changed, untracked, or conflicted) (`file_changes.rs`). The uncommitted
files are split as Git splits them: **Staged Changes**, what the index
holds against `HEAD`, and the working tree's changes against the index, in
three groups by what a checkpoint does with them:

- **Translations**: the PO files of `po/`, each with how many of its strings
  changed and, opened, those changes (see
  [String changes and history](#string-changes-and-history));
- **Project files**: the files of [Checkpoint](#checkpoint) besides the
  translations, each with its readable change (below);
- **Not committed**: every other file. A checkpoint leaves them as they
  are; a commit takes them only when they are staged.

A file partly staged appears in both lists, each with its own side of the
change. A commit's files use the same three groups.

### Working-tree operations

The dock offers the operations Git tools offer on uncommitted files and
recent commits, each a single Git operation (`aeria-git::worktree`). Paths
are project-relative and read literally (`--literal-pathspecs`):

- **Stage** (`git add --all`) and **Unstage** (`git restore --staged`;
  before the first commit, `git rm --cached`) one file, a group, or every
  staged file.
- **Discard** gives a file back its staged or committed content (`git
  restore --worktree`) and deletes an untracked file, after a confirmation
  that names what cannot be restored. Staged changes stay. A conflicted
  file is not discarded. The editor's writes wait while files change, and
  its views read them again.
- **Commit** takes what is staged when anything is (`git commit`), with a
  blank message replaced by a summary of the staged string changes;
  otherwise it is a checkpoint.
- **Commit (Amend)** replaces the last commit with one that also holds the
  new changes, what is staged or else what a checkpoint would take; a blank
  message keeps the commit's message, and without changes only the message
  changes. **Undo Last Commit** (`git reset --soft HEAD~1`, or removing the
  branch's only commit) brings the last commit's changes back to the index
  and its message back to the composer. Both refuse a merge commit, a merge
  in progress, and a commit any remote-tracking branch contains
  (`git branch --remotes --contains`), since changing it would need a forced
  push.
- **Revert** commits the undoing of a commit that is not a merge (`git
  revert --no-commit`, then a commit `Revert "<subject>"`). It requires
  committed translations, as Pull does; PO files that conflict are joined
  per string (see [Per-string merge](#per-string-merge)) with no
  resolutions, and a string changed again since, any other conflict, or a
  result that is not a project for the open game version leaves the branch
  as it was.
- **Create branch** at a commit creates the branch there and switches to
  it, with the requirements and validation of a branch switch (see
  [Branches](#branches)); a failed switch deletes the new branch again.
- **Copy** a commit's ID or subject, and **Open on server**: the commit's
  page on the hosting service of the remote the branch syncs with,
  `https://<host>/<path>/commit/<id>` (GitLab `/-/commit/`, Bitbucket
  `/commits/`) for an HTTP(S), SSH, or scp-like remote URL without its
  credentials; the action is named after the remote (**Open on origin**).
  A local path has no page. The address comes from the remote,
  never from the renderer.

### Project file changes

Project files are shown in readable form, computed in the desktop
(`project_changes.rs`):

- the glossary is compared by term: added, removed, and changed entries with
  translation, note, other forms, and folder;
- the style and `.gitattributes` are compared by line;
- project, pack, and font settings are compared by field, with
  list entries keyed by their `id` or `font` (`fonts › MiedingerMid › source:
  tektur → unbounded`);
- font files are reported as added, replaced, or removed with their size;
- the workflows are reported as added, updated, or removed.

A terms or settings file that does not read as its format (a terms file of
an older format, invalid JSON) is compared by line instead, and says so. A
file that is not text, or too large to compare by line, is reported as
changed without its content.

### String changes and history

The PO files hold one entry per string, found by its `msgctxt`, so string
views are derived directly from Git data (`aeria-git::entries`):

- **Pending changes** compare each changed PO file in the working tree with
  `HEAD` entry by entry and report each string whose translation, fuzzy mark,
  or note changed: translated (it had none), changed, cleared, or marked
  (only the mark or the note changed), with the before and after state.
- **Commit changes** compare a commit with its first parent the same way.
- **String history** reads the last 200 commits of the string's file (`git
  log -- <file>`, merges left out, so a change is attributed to the commit
  that authored it) and lists, newest first, those that changed the string,
  with author, time, message, and its state before and after. The
  uncommitted change is reported separately.

The desktop maps each change to the string's coordinate in the game, so the
Git dock can open it; a string the game no longer has keeps only its
`msgctxt`.

### Fetch, Pull, and Push

The Git dock offers the three steps of exchanging commits with the remote of
the current branch (its configured remote, else `origin`, else the only
remote). None of them rebases or force-pushes.

- **Fetch** fetches that remote.
- **Pull** fetches, then integrates the incoming commits of the branch's
  upstream. Uncommitted changes in `po/` block it. A branch that is only
  behind fast-forwards; a diverged branch is merged. PO files that conflict
  textually are joined per string (see [Per-string merge](#per-string-merge)).
  Any other conflicted file, or a string changed differently on both sides
  without an explicit resolution, aborts the merge and leaves the repository
  unchanged. After everything merged, the result must still be for the open
  game version (`aeria.json` `gameVersion`); otherwise the branch is reset to
  its starting commit with `git reset --merge`, and the project must first be
  updated to that game version. The editor's writes are held while Pull
  changes the working tree, and the editor reads a file again when it
  changed, so the merged strings show up without reopening the project.
- **Push** pushes the local commits of the current branch, whichever branch
  it is. A branch without an upstream is published to the remote and
  tracked. Push refuses while the upstream, as last fetched, has commits the
  branch lacks, so a push never needs to be forced; a push rejected by the
  remote (for example a protected branch) is reported as a Git failure.

## Per-string merge

Two branches that translated different strings of one PO file change lines
that Git usually merges on its own. When Git reports a PO file as conflicted,
Aeria reads the base, local, and incoming versions from the index stages and
joins them entry by entry (`aeria-git::merge_file`):

- The two versions must hold the same entries in the same order with the same
  `msgid`, comments, header, and obsolete entries: only translations, fuzzy
  marks, and notes may differ. Otherwise (for example one side updated the
  project to a new game version) the file is reported as a conflict and the
  merge aborts.
- For each entry, a side equal to the base takes the other side; two equal
  sides take either. Two sides that changed the string differently are a
  conflict, resolved only by an explicit choice of the local or incoming
  version supplied to a repeated Pull; Aeria never chooses on its own.
- A string's state is its translation, fuzzy mark, note, and whether it is
  reviewed (see [`po-project.md`](./po-project.md#reviews)): a review on one
  branch and another text on the other are a conflict of the string. Git's
  own merge, which joins them without one, leaves a review mark whose
  fingerprint no longer matches, so the string is not reviewed.

Joined files are written in the canonical format and committed as the merge
commit. Conflicting strings are reported with their base, local, and incoming
versions and their coordinate in the game. Command-line `git merge` merges PO
files as text; a real conflict there is resolved by editing the file.

## Branches

The Git dock switches between branches: local branches, and remote branches
without a local one, which check out as tracking branches. A switch requires
committed translations and is undone when the branch holds a project for
another game version or in an older format. A new branch is created from
`HEAD` on request; upstreams need no setup, since the first Push of a branch
publishes it and sets its upstream. Settings → Repository can point a branch
with commits at another fetched remote branch, and explains when there is
nothing to choose (no commits yet, or an empty remote).

Branches are deleted only on request, from the branch switcher or Settings →
Repository, after a confirmation, and never the current branch:

- A **local branch** is deleted with `git branch -D`. When its upstream is a
  branch on a remote, the confirmation offers to delete that branch too
  (`git push <remote> --delete`); the remote branch goes first, so a remote
  that refuses (GitHub refuses to delete the repository's default branch)
  leaves the local branch in place.
- A **remote branch** without a local one is deleted on its remote. Git then
  drops its remote-tracking branch, and a recorded default branch of the
  remote (`<remote>/HEAD`) that named it.
- A deletion that would lose commits, meaning commits that no other local
  branch, remote-tracking branch, or tag contains (`git rev-list` of the
  branch excluding every other ref), names how many and needs an explicit
  "Delete anyway". Remote branches are judged by their remote-tracking
  branches as last fetched.

## Aeria Guard

A Git host merges a pull request as plain text and lets files be edited on
its website or in any editor, without Aeria's checks. A text merge that
happens to apply can still leave conflict markers or broken macros, and a
file edited outside Aeria can change the game's text, which only a game
update in Aeria may change. Aeria Guard is the gate of the main branch
against both, and it summarizes every change for the person who merges it.

### Stages

`aeria-guard` (crate `aeria-guard`) runs, without the game, in three stages:

1. **Integrity**: no Git conflict markers in PO files, the knowledge files,
   or any project file; `aeria.json` reads; Pack and Font Settings are
   valid, and every font file the font settings name exists.
2. **Translations**: every PO file reads without a problem; every `msgctxt`
   is an identity once per file; every translation passes the checks of a
   translation against its `msgid` (see
   [`po-project.md`](./po-project.md#checking)); the knowledge files read.
3. **Changes**, compared with the base revision (`--base`):
   - While the game version and the project languages stay, the game's data
     stays as the base has it: no PO file is added or removed, and in every
     PO file the header, the set of strings, and each string's `msgid` and
     `#.` notes are unchanged, kept strings of removed game text included.
     Any difference is an error. Translations, notes, fuzzy marks, reviews,
     and term exceptions may change.
   - A change of the game version or the languages is a warning that the
     game's text was not compared and that the update must come from Aeria.
   - Translations the base has that the change loses, changed project
     settings or fonts (they change the pack every player gets), and changed
     files under `.github/` (they run with the repository's permissions once
     merged) are warnings; other files outside the project's data are
     notices.

Errors fail the guard; warnings and notices do not. In GitHub Actions the
findings become annotations on the files, and each stage adds a section to
the run's summary. Checks that need the game, such as whether a `msgid` is
still the game's text, stay with Aeria. `aeria-guard --review --base REV`
prints the review below on its own.

### Review

With a base, the run's summary also gets a review of the change: the
strings whose translation, note, fuzzy mark, or review changed, file by
file in folded tables with the source and the text before and after; the
terms added, removed, or changed by headword; how many lines of the style
changed; and every other file changed. Long lists are cut to stay within
GitHub's summary size.

### Workflow and action

The repository runs Aeria Guard through `.github/workflows/aeria-guard.yml`,
which the Git dock writes (`aeria_git::render_guard_workflow`). It is a
project file: writing it commits nothing, a checkpoint commits it, and the
dock offers an update when the file differs from what this Aeria writes.
The workflow:

- runs on `pull_request_target` for pull requests into the remote's default
  branch, on pushes to that branch, and by hand. With
  `pull_request_target` the workflow comes from the base branch, so a pull
  request cannot change or switch off the check that judges it; its token
  can only read (`permissions: contents: read`);
- has one job, **Aeria Guard**, the status check the branch requires;
- uses the action `AngelicaProject/Aeria/guard@<commit>`, naming by its
  commit the Aeria the dock's build was made from (`AERIA_COMMIT` of the
  release build; `main` for a development build), with the version as a
  comment. Updating the workflow, by hand or with Dependabot, changes that
  line.

The action (`guard/action.yml` in this repository) builds `aeria-guard`
from the commit it is taken from, before anything of the project is checked
out, so nothing in a pull request reaches the build, and the project runs
exactly the code it named. It then checks out the change without
credentials: for a pull request GitHub's test merge with its base
(`refs/pull/<number>/merge`), for a push the pushed commit, two commits
deep, so the first parent is the base either way, and runs every stage
against it. The project's files are only read; Git runs no filter or
textual conversion of theirs, since those need repository configuration
that a checkout does not carry.

GitHub reads workflows only from the repository's top folder, so the dock
offers the workflow only for a project there.

### Protection on GitHub

Aeria Guard decides only when GitHub requires it. A translation repository
is protected by two rulesets, which the dock saves as files to import on
GitHub (Settings → Rules → Rulesets → New ruleset → Import a ruleset;
`aeria_publish::branch_ruleset` and `tag_ruleset`):

- **Main branch** (the default branch): no deletion, no force push, changes
  only through pull requests that merge with a merge commit (it keeps each
  branch's history, which the [per-string merge](#per-string-merge) reads),
  and the Aeria Guard check must pass on a branch that is up to date with
  the base. Nobody bypasses it: maintainers work in branches too.
- **Pack releases** (`refs/tags/harmonia/**/*`): only maintainers and
  administrators create release tags, and nobody moves or deletes them, so
  a published pack stays what players were given.

The dock reads whether the repository has these rules from GitHub's API
without signing in, which works for public repositories, and reuses the
answer for ten minutes; a private repository, classic branch protection, or
no connection leaves the answer unknown.

When the remote refuses a push to a protected branch (GitHub's `GH006` and
`GH013`), Push reports `gitBranchProtected` instead of Git's output: the
commits go on in a new branch created at the current commit, and reach the
main branch through a pull request. For a branch other than the guarded
one, the dock opens GitHub's page for a pull request from it; for a fork,
GitHub proposes the repository it was forked from.

## Remotes and upstream

Settings → Repository lists every remote with its URL, which can be edited
(`git remote set-url`) or removed (`git remote remove`; nothing is deleted on
the server), and add a remote. The upstream of the current branch, which Pull
receives from and Push pushes to, is chosen from the remote-tracking branches
after fetching every remote (`git fetch --all --prune`) and set with
`git branch --set-upstream-to`.

## History

The project history lists commits of `HEAD` newest first in topological
order in the Git dock, loaded in pages of 100 as the list scrolls, with branch and tag names
(`%D`). A project at the repository top level shows the complete history,
merges included; a project in a subdirectory shows its path-limited history
with rewritten parents (`--parents`) so the graph stays connected. The graph
lanes are laid out in the renderer (`commitGraph.ts`). Clicking a commit
opens it in a document tab with its changed files; one unpinned commit tab
is reused while browsing.

A commit of `HEAD` that no remote-tracking branch contains, as last
fetched (`git rev-list HEAD --not --remotes`), is marked as not pushed yet,
in the list and in its commit tab: these are the commits Push would publish,
a merge commit of a Pull among them. Without a remote nothing is marked.

History can be searched and filtered by file: the newest 50,000 commits of
`HEAD` (path-limited to the project, or to one file) are matched by subject,
author name or email, or ID prefix, ignoring case. A narrowed history shows
short IDs instead of the graph, whose lanes would not connect.
