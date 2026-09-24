# AI translation

AI assists translation, but it is never a source of identity truth.

Aeria remains usable without a configured AI provider.

The proposed design of the Angelica agent, its tools, and batch jobs is in
[`ai-agent.md`](./ai-agent.md).

## Provider boundary

The core interface is provider-neutral. Additional provider adapters are justified when they offer useful capabilities beyond the common protocol.

Project-shared AI inputs may include translation guidance and glossary data. User model/provider preferences remain local.

### Transport

`aeria-ai` has one transport: the OpenAI-compatible Chat Completions API
(`POST {baseUrl}/chat/completions`, bearer authentication) plus the optional
model listing `GET {baseUrl}/models`. A model's reasoning effort is sent as the
`reasoning_effort` request field only when an effort is selected. TLS uses
rustls with the platform certificate verifier. Transport failures are
classified (network, timeout, unauthorized, not found, rate limited, rejected
request, provider failure, invalid response); provider error text is bounded
and never contains the API key.

A base URL must use HTTPS, except plain HTTP to `localhost`, `127.0.0.1`, or
`[::1]` for local servers. Credentials, queries, and fragments in the URL are
rejected, so no secret can reach settings through it. Base URLs are stored
without a trailing slash.

### Request headers

Every request carries the bearer key and the provider's extra static headers.
Chat Completions requests also carry the provider's session header, when one
is configured, set to a stable ID for the conversation; a connection check
uses a one-off ID. Header names are lowercase HTTP tokens and cannot replace
headers Aeria sets itself (`authorization`, `content-type`, `host`,
`user-agent`, and hop-by-hop headers). Values are visible ASCII without line
breaks. At most 16 extra headers are allowed. Extra header values are stored in
plain settings, so they must not hold secrets.

Extra headers exist so a provider's API change can be followed without an
Aeria release.

### Presets

Presets supply defaults for a new provider and do not change the transport:

| Preset | Base URL | Session header |
| --- | --- | --- |
| OpenCode Go | `https://opencode.ai/zen/go/v1` | `x-opencode-session`, which OpenCode Go uses for routing and prompt caching |
| OpenRouter | `https://openrouter.ai/api/v1` | None |
| OpenAI-compatible | Supplied by the user | None |

Presets carry no model list. Models come from the provider's own
`GET /models` listing, loaded when the first key is saved and on request.
Updating from the provider replaces the list and keeps the efforts and context
window of models it still reports. A listing may include models that the
provider serves only through other APIs; a connection check reveals them.
Models can also be added by ID for providers without a listing. Listed models
have no known context window or effort values; the user enables the efforts a
model accepts, which a connection check with that effort can confirm first.

### Local settings

Provider settings are machine-local application data in
`<app-data>/ai-settings-v1.json`, never in a repository. The document holds
`formatVersion` 1, the providers (opaque local ID, preset kind, name, base URL,
models with ID, optional context window, and accepted efforts, the optional
session header, and extra headers), and
Angelica's optional default model selection (provider, model, and an effort
the model accepts). Unknown fields, duplicate IDs, invalid URLs, and a
selection that does not match a configured model and effort are rejected.
The session header and extra headers are optional fields; a provider written
without them has neither.
Replacing or removing a provider clears a default selection it no longer
supports.

The file follows the recent-project registry's storage rules: a 256 KiB
limit, an exclusive OS file lock around every read-modify-write, a synced
temporary file published atomically, and recovery of the previous file only
when the final file is missing. Malformed or newer documents are typed errors
and are never replaced with defaults.

### Credentials

API keys are stored only in the OS credential store (Windows Credential
Manager, the Secret Service on Linux, or the macOS Keychain) under the service
`Aeria` and the account `ai-provider/<provider id>`. They are never written to
settings, logs, errors, or IPC responses; the renderer sees only whether a key
is stored, missing, or unreadable. Removing a provider deletes its key first,
so a provider is never forgotten while its key remains.

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
