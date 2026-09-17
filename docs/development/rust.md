# Rust engineering

Rust owns the correctness-sensitive application core.

## Design preferences

- Prefer owned values at persistence, async, IPC, and subsystem boundaries.
- Prefer IDs over long-lived cross-object references.
- Keep pure domain logic separate from adapters.
- Use enums to make states and decisions exhaustive.
- Avoid broad `Arc<Mutex<AppState>>` designs; split authority into focused services/resources.
- Use async for real asynchronous I/O and long-lived concurrency, not by default.
- Keep batch/data processing bounded in memory through iterators/chunks/queries.
- Errors expected from user data, repositories, providers, or source compatibility are recoverable typed results.
- `unsafe` is forbidden by workspace lint in first-party crates.

Correctness and readability for future maintainers take precedence over clever abstractions.
