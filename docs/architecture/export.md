# Runtime export

Aeria exports a versioned translation pack consumed by the in-game Harmonia plugin.

The runtime consumer primarily needs deterministic lookup by source coordinate:

- sheet
- row
- subrow
- column

The pack format may be designed specifically for fast, reliable runtime lookup and does not need to mirror the editable Git workspace representation.

## Requirements

- explicit format version
- deterministic output for identical project/source inputs
- validation before emission
- no partially written final pack
- atomic replacement where supported
- enough source/snapshot identity to reject obviously incompatible packs

The exact Pack Format v1 is intentionally not frozen until the first exporter/consumer implementation is designed together.
