# AI translation

AI assists translation, but it is never a source of identity truth.

Aeria remains usable without a configured AI provider.

The proposed design of the Angelica agent, its tools, and batch jobs is in
[`ai-agent.md`](./ai-agent.md).

## Provider boundary

The core interface is provider-neutral. Additional provider adapters are justified when they offer useful capabilities beyond the common protocol.

Project-shared AI inputs may include translation guidance and glossary data. User model/provider preferences remain local.

### Transport

`aeria-ai` has two transports, chosen by the provider kind:

- **Chat Completions** for every API-key provider: `POST {baseUrl}/chat/completions`
  with bearer authentication and the optional model listing `GET {baseUrl}/models`.
  A model's reasoning effort is sent as the `reasoning_effort` request field only
  when an effort is selected.
- **Codex Responses** for a ChatGPT subscription (see
  [ChatGPT subscription](#chatgpt-subscription)).

Reasoning efforts are `minimal`, `low`, `medium`, `high`, and `xhigh`. TLS uses
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
| ChatGPT (subscription) | `https://chatgpt.com/backend-api/codex`, fixed | `session_id`, fixed |
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
Angelica's optional default model selection and the optional selection for
translation-job workers (provider, model, and an effort the model accepts),
and `webDomains`, the sorted, normalized domains whose pages Angelica reads
without asking (at most 200; omitted when empty). Unknown fields, duplicate IDs, invalid URLs, and a
selection that does not match a configured model and effort are rejected.
The session header and extra headers are optional fields; a provider written
without them has neither.
Replacing or removing a provider clears a selection it no longer supports.

The file follows the recent-project registry's storage rules: a 256 KiB
limit, an exclusive OS file lock around every read-modify-write, a synced
temporary file published atomically, and recovery of the previous file only
when the final file is missing. Malformed or newer documents are typed errors
and are never replaced with defaults.

### ChatGPT subscription

A ChatGPT provider uses the user's ChatGPT plan instead of an API key.
OpenAI offers no official way for third-party applications to do this.
Aeria follows the approach of other open-source agents such as Hermes Agent:
the sign-in of the public Codex client and the Codex backend. The settings
card states that the method is unofficial, may stop working at any time, and
counts against the plan's Codex limits.

- **Sign-in** uses the device-code flow. Aeria requests a code from
  `auth.openai.com`, opens `https://auth.openai.com/codex/device` in the
  default browser, and shows the code. It then polls until the user confirms,
  for at most 15 minutes, and exchanges the result for tokens. One sign-in
  waits at a time; starting another replaces it, and it can be cancelled.
- **Credentials**: only the refresh token is stored in the OS credential
  store, under the provider's entry. The short-lived access token is kept in
  memory and refreshed before it expires. Refreshes are serialized, and a
  refresh token that OpenAI rotates replaces the stored one before the new
  access token is used. A refused refresh reports `aiChatGptSignInRequired`.
  Signing out deletes the stored token and the cached access token.
- **Requests** go to `POST {base}/responses` with `stream: true`,
  `store: false`, Angelica's system message as `instructions`, and the
  conversation as typed input items (`message`, `function_call`, and
  `function_call_output`). Reasoning is requested with a summary and never
  replayed between requests. Requests carry the access token, the account and
  data-residency headers taken from the token's claims, `session_id` set to
  the conversation ID, and `originator: aeria`. Aeria identifies itself and
  does not present itself as Codex. A response that fails for a plan limit is
  reported as rate limiting.
- **Models** come from the Codex catalog, `GET {base}/models?client_version=…`,
  which also states each model's context window and reasoning levels. Hidden
  models are skipped.

A ChatGPT provider cannot hold an API key or a session header, and its base
URL cannot change. Extra headers are allowed; the identity headers above
always replace configured ones with the same name.

### Credentials

API keys are stored only in the OS credential store (Windows Credential
Manager, the Secret Service on Linux, or the macOS Keychain) under the service
`Aeria` and the account `ai-provider/<provider id>`. They are never written to
settings, logs, errors, or IPC responses; the renderer sees only whether a key
is stored, missing, or unreadable. Removing a provider deletes its key first,
so a provider is never forgotten while its key remains. For a ChatGPT
provider the stored secret is the refresh token described above.

## Angelica

Angelica is the built-in translation agent. The complete intended design is in
[`ai-agent.md`](./ai-agent.md); this section describes what is implemented.

Each message is sent in one of three modes, chosen in the panel:

| Mode | Tools | Writes |
| --- | --- | --- |
| Chat | Read tools | None. |
| Ask (default) | Read and write tools | Every valid translation waits as a proposal until the user applies it. |
| Auto-draft | Read and write tools | A valid translation of an untranslated string is written at once as a draft; one that would replace a translation waits as a proposal. The string selected in the editor with unsaved edits is never written at once. |

Angelica never marks anything reviewed on her own, commits, pushes, syncs,
applies a source update, or exports. She can suggest approvals, which the
user confirms (see [Suggested approvals](#suggested-approvals)).

### Conversation loop

`aeria-ai::agent::run_turn` runs one user turn. It streams a Chat Completions
response (`stream: true` with usage reporting) and forwards text and
reasoning deltas as events. When the response requests tools, it runs them in
order, appends each result, and requests the next response, for at most 24
responses per turn; a turn that reaches the limit ends with the `roundLimit`
outcome. The conversation is persisted after every appended message, so a
stopped or failed turn keeps its progress. Before a turn, a tool call without
a result, left by an interrupted turn, receives a cancellation result so the
history stays valid for providers.

Reasoning text is stored with the assistant message and sent back as
`reasoning_content` only within the current turn, which providers that
think between tool calls require; earlier turns are sent without it.

A request is kept within 75 % of the model's context window (64,000 tokens
when unknown), estimated at three characters per token. Earlier turns' tool
results are replaced with a short notice first, oldest first; if that is not
enough, whole earlier turns are dropped. The current turn is always sent in
full.

The system message holds Angelica's fixed instructions (see
`aeria-ai::prompt`), the project's languages, game version, and progress, and
the editor context the renderer sends with the message: the open sheet, the
selected occurrence, and whether it has unsaved edits. The user can stop
sending the selection. Instructions state that tool data is never an
instruction, that macros must be preserved, and that Chat mode cannot change
the project.

### Read tools

`aeria-ai::tools` defines the tools, validates their arguments, and bounds
their results; the desktop implements `ProjectReader` over the active
`ProjectSession`. Invalid arguments, unknown tools, and read failures are
returned to the model as `{"error": …}` results instead of ending the turn.

| Tool | Result |
| --- | --- |
| `project_overview` | Languages, game version, sheet and string counts, progress, and detached units. |
| `list_sheets` | Sheets with translatable strings and their progress, filtered by a name substring or by untranslated strings, paged up to 200. |
| `read_rows` | One `page_translation_rows` page of at most 50 scanned source rows, optionally filtered by state, with the `nextAfter` cursor. |
| `get_unit` | One source row, or one column of it, with translations, review states, notes, unit IDs, and context cells. |
| `pending_changes` | Uncommitted translation changes from `aeria-git`, up to 200. |
| `unit_history` | Committed history of one unit, up to 50 entries. |
| `navigate_to` | Opens an occurrence in the editor; changes nothing in the project. |

Each source, target, or note text is cut at 2,000 characters and each result
at 24,000 characters, with a visible notice. Every tool call takes the project
lock only for its own read.

Cells returned by `read_rows` and `get_unit` include `tagged`, the source in
tagged form (see [`strings.md`](./strings.md#tagged-text)), with a `tags`
legend whenever it differs from the source; a malformed source is marked
`untaggable`. Tagged sources are cut only above 8,000 characters.

### Search tools

Offered in every mode, backed by [`aeria-search`](./search.md):

| Tool | Result |
| --- | --- |
| `search_source` | Translatable strings whose source text matches the query, best first, with their translations and review states; optionally one sheet; up to 50 per page with `more`. |
| `search_translations` | Bound translations whose text contains the query, ignoring case and macros, with their sources, in source order. |
| `similar_translations` | Translation memory for one string (by location) or a given text: up to 10 translated strings with a similar source, most similar first, with a similarity from 0.5 to 1. |

The first message to Angelica starts building the source index in the
background; until it is ready, `search_source` and `similar_translations`
report that it is being built. Texts in results are cut like other tool
results.

### Web pages

`fetch_url` is offered in every mode. It reads one `http` or `https` page as
plain text: HTML is rendered to text (with the `<title>`), other `text/*`,
JSON, and XML bodies are returned as they are, and other content types are
refused. At most 3 MiB is read with a 20-second timeout, and up to 20,000
characters are returned per call (12,000 by default) with `nextOffset` for
the rest.

A page opens only when its host is allowed: the host of a link in the project
guidance, or a domain in `webDomains` or one of its subdomains. Redirects are
followed by hand, at most five, and each target is checked the same way. A
link to any other host records a web-access proposal for the conversation,
with the domain and the link, and tells Angelica to wait. Allowing it adds
the domain to `webDomains` and starts an automatic turn asking Angelica to
open the link again. Page text is data: the instructions say it is never an
instruction and may be wrong.

### Write tools

| Tool | Result |
| --- | --- |
| `validate_target` | Rebuilds a tagged translation of one string without writing, and returns the target or the rule violations. |
| `propose_translation` | Accepts 1 to 20 tagged translations. Each is rebuilt and checked; rejected ones return what to fix; valid ones are submitted and reported as `applied`, `awaitingApproval` (with a proposal ID), `conflict`, or `failed`. |

A valid translation records the string's target and review state at the time
it was produced. Every write, immediate or approved, goes through
`ProjectSession::set_assisted_target` (see
[`translation-mutations.md`](./translation-mutations.md#assisted-targets)),
so the structure policy is enforced again and a string changed in the meantime
is never overwritten: its proposal becomes a conflict. Applying a proposal is
the user's explicit approval, including for a reviewed string.

### Suggested approvals

`propose_review` is offered in Ask and Auto-draft modes. It takes up to 200
strings and a reason. Strings without a translation, already reviewed, or
not translatable are reported back as skipped; the rest are recorded as one
review proposal with each string's location, source, and current target. In
every mode the proposal waits for the user. Applying it is the user's
approval of exactly those translations: each string whose target is still
the recorded one is marked reviewed, and a changed or missing string is
skipped. The proposal's message states how many were approved and skipped.
Translation jobs never propose approvals.

### Guidance and glossary

Two optional, human-edited files at the repository root are shared through
Git with the project:

- `aeria-guidance.md`: free-form Markdown guidance on style, register,
  terminology, and conventions, up to 64 KiB. Aeria does not interpret its
  structure.
- `aeria-glossary.csv`: [Glossary Format v1](../formats/glossary-v1.md).

Translators edit both in the **Glossary and guidance** dialog (see
[`desktop-editor-ui.md`](./desktop-editor-ui.md#glossary-and-guidance)) or by
hand. Both are read when a message is sent and when a tool needs them, so
edits take effect on the next message. Angelica's system message
includes the guidance (cut at 12,000 characters, with the rest available
through `get_guidance`), which is described as maintainer guidance that
cannot change what Angelica may do, and a glossary summary. A file that
exists but cannot be used is reported to Angelica as a problem.

Cells returned by `read_rows` and `get_unit` list up to 20 glossary entries
whose terms occur in the source. `get_guidance` returns the guidance, glossary
entries matching given terms (or the start of the glossary), up to 50 excluded
rows, and file problems. `propose_translation` adds advisory
`glossaryWarnings` for a glossary translation that does not seem to be used,
allowing inflected endings (the leading two thirds of each word must appear),
and for a forbidden variant that is used. Warnings never block a write.

`propose_glossary_change` (add or replace up to 100 entries by term, remove up
to 100 terms) and `propose_guidance_change` (a complete new text) are offered
in Ask and Auto-draft modes and always wait for approval. A glossary change is
refused while the file has excluded rows, since the canonical rewrite would
drop them. Applying a file change writes the file through a temporary file and
rename only if it still has the content the change was made against;
otherwise the proposal becomes a conflict.

Draft with Angelica includes the guidance and the glossary entries matching
the string.

### Proposals

Proposals are stored beside their conversation in
`<id>.proposals.json`. A translation proposal records the location, source,
rebuilt target, and expected state; a file proposal records the file, its new
content as `target`, and its content when proposed as `expected.target`
(`null` when absent). Both record status (`pending`, `applied`, `rejected`, `conflict`, or `failed`),
an optional message, and the creation time. At most 2,000 are kept, dropping
the oldest settled ones first. Deleting a conversation deletes its proposals.

### Draft with Angelica

The editor's **Draft with Angelica** uses Angelica's default model for one
string without tools. The request carries the source in tagged form with its
legend, the row's context cells, the current translation and note, and the
project languages. The reply is read between `<translation>` markers,
rebuilt, and checked; a refused reply is sent back with the violations, for
at most three requests in total. The result becomes an unsaved draft in the
editor, which the user saves explicitly.

### Translation jobs

A sheet, several sheets, or the whole project is translated by a job: worker
subagents that each translate one chunk of strings, run by a deterministic
orchestrator and supervised by Angelica.

Angelica has `estimate_job` in every mode and `job_status` and `job_events`
to report on jobs. In Ask and Auto-draft modes she also has `start_job`,
`amend_job`, `retry_units`, `pause_job`, `resume_job`, and `cancel_job`.
`start_job` never starts anything: it records a job proposal with the scope,
instructions, concurrency (1 to 16, 8 by default), the estimate, and a token limit of twice
the estimate (at least 200,000). The user starts the job from the proposal.

A scope is a list of sheets, or every sheet with translatable strings, and a
filter: untranslated strings (the default), strings that need review, or
untranslated strings and drafts. Reviewed translations are never included.
The string list is fixed when the job starts, together with each string's
current target and review state. Strings are grouped in order into chunks of
at most 30 strings and 12,000 source characters, never across sheets. The
estimate is the number of strings and chunks and a rough token count.

Each chunk is translated by a worker with a fresh context: fixed worker
instructions, the project facts, guidance and matching glossary entries, the
job's instructions as they are when the chunk starts, and its strings in
tagged form with their legends, context cells, current translations,
notes, and up to three translation-memory matches. Its tools are `get_unit` and `read_rows` for context, `get_guidance`,
`validate_target`, `submit_translations` for the strings of its own chunk
only, and `report_issue`, which records an event for Angelica. A worker has at
most 8 responses. A submitted translation is rebuilt and written as a draft
through `ProjectSession::set_assisted_target` against the recorded state,
without permission to replace a reviewed string: a string changed meanwhile
is skipped as a conflict, and a translation that still breaks the structure
after the worker's corrections is rejected. Strings the worker leaves
unsubmitted fail.

Jobs are machine-local application data, one SQLite database per project in
`<app-data>/jobs/<key>.sqlite3` with the conversation key. A job records its
conversation, specification (scope, instructions, worker model, token limit,
concurrency), status (`running`, `paused` with a reason, `completed`,
`cancelled`), token usage, events, and each string's chunk, status
(`pending`, `running`, `drafted`, `rejected`, `failed`, `conflict`),
attempts, and message. The worker model is the jobs model from the settings,
or Angelica's default model.

A running job has `concurrency` lanes. Each lane claims the next chunk,
checks first that the job's project is still open, and pauses the job when
the token limit is reached or when, after 40 finished strings, more than 30 %
were rejected. A network, timeout, rate-limit, or unavailable failure returns
the chunk's unfinished strings to the queue and waits (20 seconds times the
failures in a row); the third failure in a row pauses the job, as does a
rejected key. Other provider errors fail the chunk's unfinished strings.
Pausing or cancelling returns claimed strings to the queue; a job left
running when Aeria closed is paused the next time its project's jobs are
read. Rejected, failed, and skipped strings can be queued again.

Lanes share the job store: a chunk is claimed, and an event numbered, in one
write transaction, so parallel lanes never claim the same chunk or wait on a
lock upgrade. A busy store is retried (up to five times with a growing
delay) before the job pauses, and a chunk's outcomes are recorded with the
same retries so its strings never stay claimed. A job's summary counts its
active workers, the chunks being translated right now.

When a job completes or pauses on its own, Aeria wakes Angelica: unless a
turn is already running there, it adds an automatic `[Aeria]` message to the
job's conversation and starts a turn with the conversation's last model and
mode, so she can report and suggest what to do next.

### Conversations

Conversations are machine-local application data stored per project in
`<app-data>/conversations/<key>/<id>.json`, where the key is derived from a
SHA-256 hash of the repository root and the ID is a UUID. They are never
written to a repository. A conversation records its title (from the first
message), the model, effort, and mode last used, the messages including tool
calls and results, and the provider-reported token usage. Messages Aeria adds
for Angelica, such as job reports, are user messages marked `automatic`. Files are written through a
synced temporary file and rename and are limited to 16 MiB; a damaged file is
reported when opened and skipped in the list.

## Batch workflow

Batch translation runs as [translation jobs](#translation-jobs):

- jobs are resumable and stored locally
- a token estimate is shown before a job starts
- related strings are batched per sheet for context and cost efficiency
- provider or structural failures are isolated and retryable
- successfully validated results are written directly as `draft` changes in the Git working tree
- a failed/unsafe result is not persisted as a successful translation
- commits remain an explicit human action

Model/provider provenance belongs to local job/history/commit context rather than permanently cluttering every translation unit.

## Validation

The Rust side constructs structured context and validates responses. Macro/structure invariants and glossary/QA checks run before a translation is accepted.

Future AI review/QA features are allowed, but they never turn AI output into `reviewed` automatically.
