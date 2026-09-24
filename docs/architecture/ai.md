# AI translation

AI assists translation, but it is never a source of identity truth.

Aeria remains usable without a configured AI provider.

The proposed design of the Angelica agent, its tools, and batch jobs is in
[`ai-agent.md`](./ai-agent.md).

## Provider boundary

The core interface is provider-neutral. Initial transports should prioritize services the community is likely to use, especially OpenRouter and OpenAI-compatible APIs. Additional provider adapters are justified when they offer useful capabilities beyond the common protocol.

Credentials are stored locally using appropriate OS secret storage and never committed.

Project-shared AI inputs may include translation guidance and glossary data. User model/provider preferences remain local.

## Batch workflow

A batch job may target selected units, new units, changed units, or untranslated units.

- jobs are resumable and stored locally
- cost/token estimates are shown before large work
- related units may be batched for context/cost efficiency
- provider or structural failures are isolated and retryable
- successfully validated results are written directly as `draft` changes in the Git working tree
- a failed/unsafe result is not persisted as a successful translation
- commits remain an explicit human action

Model/provider provenance belongs to local job/history/commit context rather than permanently cluttering every translation unit.

## Validation

The Rust side constructs structured context and validates responses. Macro/structure invariants and glossary/QA checks run before a translation is accepted.

Future AI review/QA features are allowed, but they never turn AI output into `reviewed` automatically.
