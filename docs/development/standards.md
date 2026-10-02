# Development standards

## Toolchains

Pinned bootstrap baseline:

- Rust 1.98.1
- Node.js 24.21.0 LTS
- pnpm 12.4.2
- React 19.3
- TypeScript 6.0.3
- Vite 8.1
- Tauri 2.11.x

When changing these versions, verify the current stable upstream release first and update pins/lockfiles together.

## Reliability

Operations that can affect translation data should prefer transactional or atomic patterns:

- write temporary file
- validate/flush as appropriate
- atomically replace destination where supported

Game updates, per-string merges, and machine translation runs should create local recovery points when doing so materially improves recoverability beyond ordinary Git working-tree state.

Expected data/user failures are typed errors and user-visible diagnostics, not panics.

## Testing priorities

Highest-value regression coverage:

- SqPack and Excel reading fixtures and game source facts
- structured string parsing/serialization golden tests
- unknown/opaque macro preservation
- parser fuzz/property tests
- deterministic PO file serialization
- game update tests across real/synthetic source changes
- per-string merge cases
- export determinism/compatibility
- machine translation runs against a fake model server

CI should prefer synthetic fixtures or legally safe minimized fixtures. Real game content should not be required in public CI.

## Documentation

Repository documentation follows [`documentation.md`](./documentation.md). Keep canonical rules in the document that owns them and update documentation with the implementation when contracts change.
