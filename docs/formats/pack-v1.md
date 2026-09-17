# Pack Format v1 — design constraints

Status: **draft, not yet frozen**.

The exported pack is a runtime artifact, not an editable collaboration format.

It must provide fast deterministic translation lookup by sheet/row/subrow/column and carry enough source compatibility metadata for the in-game consumer to reject incompatible data safely.

Aeria owns pack generation; the Harmonia in-game plugin will be implemented against the finalized format. The editable workspace and runtime pack are deliberately separate contracts.
