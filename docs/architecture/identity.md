# Translation identity

Aeria distinguishes snapshot coordinates from durable translation identity.

## Source coordinate

Within an HXS snapshot, a string occurrence is addressed by:

- sheet
- row
- subrow
- column

This binding is also the runtime lookup shape expected by the in-game consumer.

## Translation unit identity

A translation unit has an Aeria-owned stable identity that can survive a coordinate change across game versions. The current source coordinate is a binding of that unit, not the durable identity itself.

New identities must be generated deterministically from source facts so that independent branches encountering the same new source occurrence do not invent incompatible random IDs.

The exact canonical derivation is part of Workspace Format v1 and must be finalized before real translation data is committed.

## Rule

Never infer durable identity from textual similarity alone. Cross-version identity changes are decided by the deterministic rebase engine and surfaced as ambiguous when uniqueness cannot be established safely.
