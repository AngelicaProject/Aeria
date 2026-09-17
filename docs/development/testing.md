# Testing strategy

Tests protect contracts, not implementation trivia.

Prioritize deterministic fixtures and regression cases around the highest-risk boundaries: source verification, structured strings, workspace persistence, rebase, Git merge behavior, AI validation, and export.

## Test layers

- Unit tests for pure domain rules and parsers.
- Golden/round-trip tests for structured strings and serialization.
- Property/fuzz tests for parsers and invariants.
- Integration tests over synthetic HXS/workspace repositories.
- Desktop IPC tests for capability contracts.
- Frontend component/workflow tests for high-value user flows.
- Packaging smoke tests on supported release targets.

Tests must not depend on a user's installed game, credentials, network availability, or private repository data.
