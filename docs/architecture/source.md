# Source and HXS

HXS is Aeria's immutable source contract.

The Rust core consumes HXS and does not depend on how a snapshot was produced. The desktop source manager may invoke Harmonia Atlas to provide a one-click `Generate Source from FFXIV installation` experience.

## Import verification

Before a snapshot is accepted, Aeria should verify its supported format, schema/invariants, logical metadata, and hashes/identifiers. A previously verified file may be trusted through a local cache keyed by file identity/hash when safe to do so.

## Source cache

Snapshots live in a machine-local content-addressed source store, not in the translation repository.

The current and immediately previous snapshots are protected because rebase requires them. Older snapshots are eligible for LRU cleanup under a user-configurable cache budget.

## Degraded workspace

A project may be opened without its source snapshot. Git history, manifest, glossary, and persisted translations remain inspectable, but editing, reliable preview, rebase, AI translation, and export require a compatible verified source.

## Additional source languages

Additional official source languages are local user preferences. They are context, not project identity, and are not required to be committed to the translation repository.
