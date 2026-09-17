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

Aeria should expose Git author name/email configuration. Prefer established OS/Git credential mechanisms. Do not become a private-key/password manager unless a future requirement clearly justifies it.

## Semantic merge

Workspace serialization should be merge-friendly by construction. A semantic merge driver/resolver may later merge by translation-unit identity and present real same-unit conflicts in Aeria.
