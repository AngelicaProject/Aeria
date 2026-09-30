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

"Checkpoint" and "contribution branch" name Aeria's operations in this
documentation and in code. The interface calls them what Git users know: a
checkpoint is a commit (the button is labelled Commit), and a contribution
branch is a branch for a pull request.

## Git in the desktop

- The **Git dock** is the whole everyday view and follows the repository on
  its own (below): the branch, which can be switched there, sync state and
  Sync, the uncommitted changes (strings grouped by sheet, and project files
  grouped by area), the checkpoint composer, and the project history with a
  commit graph. Clicking a commit opens it in a document tab.
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

## Collaboration policy

Translations reach the main branch (`main` or `master`) **only through pull
requests**. Nobody commits to it directly: a checkpoint on the main branch
starts a contribution branch, and Aeria never pushes local commits to the
published main branch. There is no direct mode.

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
--only`): `po/` and the project files `aeria.json`, `.gitattributes`,
`aeria-collaboration.json`, `aeria-pack.json`, `aeria-fonts.json`, the
`fonts/` directory of source fonts, the `aeria-knowledge/` directory of
[project knowledge](../formats/knowledge-v1.md), and the workflows
`.github/workflows/harmonia-feed.yml` and `.github/workflows/aeria-check.yml`
(`PROJECT_PATHS`); a path without a file is left out, since Git has no empty
folders. Unrelated staged or modified files are left untouched and listed in
Settings → Repository as other files. A blank message is replaced by a
deterministic summary: the string changes, for example `Translate 3 strings,
update 1 translation (Addon, Quest)`, followed by the changed project areas
(`; update terms, game fonts`), or `Update glossary, pack settings` when no
string changed.

Whether a checkpoint first moves to a contribution branch follows the
collaboration policy committed in `HEAD`, not the working copy, so committing
a policy change does not move its author to a contribution branch.

### Project file changes

Uncommitted changes (`HEAD` against the working tree) and each commit's
changes (against its first parent) are shown for project files in readable
form, computed in the desktop (`project_changes.rs`):

- the glossary is compared by term: added, removed, and changed entries with
  translation, note, and forbidden translations;
- the style and `.gitattributes` are compared by line;
- project, pack, font, and collaboration settings are compared by field, with
  list entries keyed by their `id` or `font` (`fonts › MiedingerMid › source:
  tektur → unbounded`);
- font files are reported as added, replaced, or removed with their size;
- the workflows are reported as added, updated, or removed.

A file that cannot be parsed is reported as changed but unreadable.

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

### Sync

Sync is fetch, integrate, push:

1. Fetch the current branch's remote (its configured remote, else `origin`,
   else the only remote).
2. Integrate incoming commits: the branch's upstream and, on a contribution
   branch under the pull-request policy, the remote main branch.
   Uncommitted changes in `po/` block integration. A branch that is only
   behind fast-forwards; a diverged branch is merged. PO files that conflict
   textually are joined per string (see [Per-string merge](#per-string-merge)).
   Any other conflicted file, or a string changed differently on both sides
   without an explicit resolution, aborts the merge and leaves the repository
   unchanged. After everything merged, the result must still be for the open
   game version (`aeria.json` `gameVersion`); otherwise the branch is reset
   to its starting commit with `git reset --merge`, and the project must
   first be updated to that game version.
3. Push local commits. A branch without an upstream is published to the
   sync remote and tracked. A push rejected by the remote (for example a
   protected branch) is reported as a Git failure.

Sync never rebases or force-pushes. The editor's writes are held while it
changes the working tree, and the editor reads a file again when it changed,
so the merged strings show up without reopening the project.

The steps are also available one by one, with the same rules: **Fetch** runs
step 1 only; **Pull** runs steps 1 and 2, including the per-string merge and
explicit conflict resolutions, and never pushes; **Push** runs step 3 and
refuses while the upstream, as last fetched, has commits the branch lacks,
so a push never needs to be forced.

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
  version supplied to a repeated sync; Aeria never chooses on its own.

Joined files are written in the canonical format and committed as the merge
commit. Conflicting strings are reported with their base, local, and incoming
versions and their coordinate in the game. Command-line `git merge` merges PO
files as text; a real conflict there is resolved by editing the file.

## Branches and contributions

The Git dock switches between local branches. A switch requires checkpointed
translations and is undone when the branch holds a project for another game
version.

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
- The hosting service merges pull requests as plain text. When another
  contribution changed the same strings, syncing the contribution branch
  joins main per string and asks for the choices; the hosting service's own
  conflict resolution would commit conflict markers into PO files. While
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
  a merge with the per-string merge of PO files), checks the result, and
  deletes the merged branch. A rejected result resets the main branch and
  returns to the contribution branch. As soon as the repository has a remote,
  this action is unavailable and pull requests are the only way in.

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
its website, without Aeria's checks. A text merge that happens to apply can
still leave conflict markers, broken macros, or removed translations. The
merge check catches that before the merge.

`aeria-check` (crate `aeria-check`) runs the checks Aeria runs when it saves
a translation, without the game, in three stages:

1. **Integrity**: no Git conflict markers in PO files or any project file;
   `aeria.json` reads; Collaboration, Pack, and Font Settings are valid, and
   every font file the font settings name exists.
2. **Translations**: every PO file reads without a problem; every `msgctxt`
   is an identity once per file; every translation passes the checks of a
   translation against its `msgid` (see
   [`po-project.md`](./po-project.md#checking)); the knowledge files read.
3. **Merge**: compared with the base revision (`--base`), a change of the
   project languages or of the game version, and translations the base has
   that the result lost, are warnings for the reviewer.

Errors fail the check; warnings and notices do not. In GitHub Actions the
findings become annotations on the files and each stage adds a section to
the job summary. Checks that need the game, such as whether a `msgid` is
still the game's text, stay with Aeria.

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
Sync or Pull in Aeria achieves with the per-string merge; GitHub's merge then
produces exactly the branch's tree, and the check remains a second line of
defense.

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
opens it in a document tab with its string changes and project file
changes; one unpinned commit tab is reused while browsing.
