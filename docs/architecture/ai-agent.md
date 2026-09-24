# Angelica translation agent

> **Status: proposal.** Nothing in this document is implemented yet. It
> describes the intended design of Aeria's AI agent so that the
> implementation milestones can be reviewed against one plan. Decisions still
> open are listed in [Open questions](#open-questions). The general AI
> invariants in [`ai.md`](./ai.md) apply and take precedence.

## Purpose

Angelica is Aeria's built-in AI agent. It is not a "translate this list"
button: it works like an IDE agent specialized for Aeria's domain. It answers
questions about the project and the game text, reads project state through
Aeria-owned tools, drafts translations for one unit, a selection, a sheet, or
the whole project, and checks its own output against Aeria's structural
validation before anything is written.

The name `Angelica` is the agent's fixed code and display name. It is not
localized and does not change with the interface language. Angelica replies in
the language the user writes in; its system instructions are fixed and
maintained with the application.

Aeria remains fully usable without Angelica or any configured provider.

## Invariants

These follow from [`../product/principles.md`](../product/principles.md) and
[`ai.md`](./ai.md) and are enforced in Rust, not by the prompt:

1. Angelica produces drafts. It can never set `reviewed`, never demote or
   overwrite a `reviewed` unit without an explicit per-unit user approval,
   and never commit, push, sync, switch branches, apply a source update, or
   export.
2. Every target Angelica writes passes intrinsic `aeria-se` validation and
   the AI structure policy described in
   [Structured strings](#structured-strings), and an empty or
   whitespace-only target is rejected, exactly as for ordinary edits. A
   rejected result is reported as a failure, never persisted as a successful
   translation. A permitted structural change is never applied without a
   person's approval.
3. Writes go through `ProjectSession` mutation APIs and the HSG permission
   gate. Angelica has no filesystem, shell, SQL, or raw workspace access.
4. Identity, rebase, merge, migration, and export never consult Angelica.
5. Provider credentials stay in OS secret storage; provider and model choices
   are local. Only project guidance and glossary data are shared through the
   repository.
6. Text read from game data, repository files, or (later) the web is data. It
   cannot grant permissions, change the approval mode, or trigger writes that
   the user did not request.

## Architecture

```text
React AI dock (chat, proposals, approvals)
        |  typed Tauri IPC + event stream
Tauri adapter (apps/desktop/src-tauri)
        |  implements AgentHost over the active ProjectSession
aeria-ai
  ├── provider      OpenAI-compatible transport, streaming, usage
  ├── agent         conversation loop, context budget, cancellation
  ├── tools         tool schemas, argument validation, policy
  ├── jobs          translation jobs, worker subagents, supervision events
  └── validation    structural checks before any write
        |
ProjectSession (aeria-workspace) · aeria-se · aeria-git (read-only) · aeria-search (later)
```

`aeria-ai` does not depend on Tauri. It defines an `AgentHost` trait with
typed read and write operations; the desktop adapter implements the trait by
delegating to `ProjectSession` and `aeria-git` read APIs, holding the project
mutex only for each individual call, never across a provider request.

AI-authored writes use a dedicated session operation, proposed as
`ProjectSession::set_assisted_target(binding, target, expected)`, which
performs the ordinary `set_target` checks plus the AI structure policy and
a compare-and-set on the current target and review state. Putting the gate in
the session means no adapter or future tool can bypass it.

## Providers

The core transport is the OpenAI-compatible Chat Completions API with tool
calling and server-sent-event streaming. Presets are configuration over this
one transport, not separate code paths:

| Preset | Notes |
| --- | --- |
| OpenCode Go | First ready-made preset, with the `x-opencode-session` session header. |
| ChatGPT (subscription) | Unofficial sign-in with a ChatGPT plan through the Codex backend; see [`ai.md`](./ai.md#chatgpt-subscription). |
| OpenRouter | Common community choice. |
| Custom OpenAI-compatible | User-supplied base URL, for example a local server. |

A provider configuration holds a base URL, a keyring reference to the API key,
and a list of models. Each model records its context window, whether it
supports tool calling, parallel tool calls, and structured JSON output, which
reasoning-effort values it accepts (for example `low`, `medium`, `high`, sent
as the provider's `reasoning_effort` parameter), plus optional prices for cost
estimates. The effort control is hidden for models that accept none. Agent
chat and translation workers require tool calling. A
**Test connection** action performs one minimal request and reports the typed
failure.

Settings are stored in local application data. API keys are stored only in the
OS secret store and are never logged, sent to the renderer, or included in
error messages.

Milestones 1 to 5, except the `gender` construct, are implemented; their
current behavior is described in
[`ai.md`](./ai.md#angelica). Milestone 5 differs from this plan: scopes are
sheets or the project with an untranslated, needs-review, or
untranslated-and-drafts filter (selected units and changed-since-update
filters are not implemented); workers write through the per-unit
`set_assisted_target` instead of a bulk API; the cost ceiling is a token
limit; and jobs are shown in the Angelica panel rather than a separate Tasks
panel.

Milestone 1 implements context windows and efforts; tool-calling, output, and
price capabilities arrive with the milestones that use them. Current behavior is
described in [`ai.md`](./ai.md#provider-boundary).

## Context

Each request is assembled in Rust from bounded layers:

1. **Agent instructions**: Angelica's fixed role, the invariants above, and
   the macro-preservation rules.
2. **Project facts**: source and target language, game version, and whether
   project guidance and a glossary exist.
3. **Project guidance**: the repository's `aeria-guidance.md` (see
   [Guidance and glossary](#guidance-and-glossary)), when present.
4. **Editor context**: the active sheet, the selected occurrence, and whether
   it has an unsaved draft, sent by the renderer with each user message.
   Mentions such as `@sheet` or `@unit` attach further context explicitly.
5. **Conversation** with tool calls and results.

Tool results are bounded in size. When the conversation approaches the
model's context window, older tool results are replaced by short summaries
before the oldest messages are dropped. Angelica fetches data through tools
instead of receiving large dumps up front.

## Structured strings

Angelica may change macro structure when the target language needs it, but
only through a vocabulary of typed constructs that Rust checks and compiles.
She never writes raw Lumina macro syntax.

Structure is not decoration: in many target languages a faithful translation
needs a different structure from the source. Examples:

- word order moves the player's name or a number to another place in the
  sentence;
- a verb or adjective agrees with the player's gender, which the English
  source does not mark;
- a noun would take plural forms after a number, following the target
  language's rules rather than the source's `1` / other split (see
  [Plural forms](#plural-forms) for why this is usually rephrased instead);
- an English-only construct, such as an article selection, has no equivalent
  and is dropped.

Unrestricted edits are unsafe for other reasons. A changed item ID shows a
different item. A reference to a parameter the game does not pass to this
string shows garbage or crashes at runtime. A removed color end leaks the
color into the rest of the UI. An opaque construct that Aeria does not
understand cannot be reasoned about at all.

### Tagged text

Rust projects each source string into tagged text. Translatable prose is plain
text. Each protected construct becomes a tag with a stable ID and a legend
entry derived from `aeria-se::semantic_analysis`, such as "player name",
"integer parameter 2", "color start", or "item link":

```text
source:  <x id="1"/> obtained <x id="2"/> <x id="3"/>.
legend:  1 = player name · 2 = integer parameter 1 · 3 = item link (Item 5)
target:  <x id="1"/> <gender m="получил" f="получила"/>: <x id="3"/> ×<x id="2"/>.
```

Constructs with translatable branches, such as an existing conditional, become
paired tags whose branches are translated in place.

### Operations

Every difference between the source structure and the returned structure is
classified. The policy is owned by `aeria-se` and decided per construct kind,
not by the model:

| Operation | Rule |
| --- | --- |
| Move a tag within the same string | Allowed. |
| Repeat a runtime value, such as the player's name | Allowed. |
| Add a grammar construct from the vocabulary, such as `gender` | Allowed when every value it reads is already available to this string (see below). Structural change. |
| Drop a construct that is language-specific in the source, such as an English article selection | Allowed for construct kinds the policy marks as droppable. Structural change. |
| Drop a runtime value, a game reference, or formatting | Rejected by default. Structural change with a required reason when the policy permits it. |
| Change a game reference's ID or sheet, or a parameter index | Rejected. |
| Unbalanced or crossed formatting pairs | Rejected. |
| Change, move across a branch, or drop an opaque construct | Rejected. Opaque constructs are preserved exactly. |
| Invent a tag ID, or reference a parameter the source does not use | Rejected. |

A grammar construct may read only context the game client provides for every
string, such as the player's gender, or a value that the source string itself
already uses. It cannot introduce new game references or new parameters.

### Grammar constructs

Harmonia only replaces strings; it evaluates nothing. Every grammar construct
therefore compiles to native macros that the game client evaluates, and the
vocabulary is limited to what native macros can express. The model supplies
only the target-language forms; Rust owns the conditions.

Native macro expressions offer comparisons (`==`, `!=`, `<`, `>`, `<=`, `>=`)
between integers, parameters, and time values, plus conditional macros such as
`if`, `switch`, and `ifpcgender`. They have no arithmetic.

| Construct | Compiles to | Status |
| --- | --- | --- |
| `<gender m="…" f="…"/>` | The native gender condition for the local player, in the form the source corpus already uses for the same purpose. | Planned. The exact macro form is taken from verified source strings, not invented. |
| `<plural …/>` | — | Not offered; see below. |

#### Plural forms

Plural rules of languages such as Russian depend on the remainder of the
number (`1, 21, 31` / `2–4, 22–24` / `5–20, 25–30`). Without arithmetic, the
client can only compare the number with constants, so a correct form for an
arbitrary number cannot be expressed. A comparison chain would be correct
only for a small known range of values, and Aeria has no range metadata for
parameters.

Angelica is therefore instructed to use number-neutral phrasing, the usual
game-localization practice, for example `Получено: <item> ×5` or
`Количество предметов: 5` instead of an inflected noun after the number.
Project guidance can state the preferred pattern. A `plural` construct can be
reconsidered if source metadata such as EXDSchema provides value ranges.

### Outcomes

After compiling, Rust rebuilds the target macro string and runs intrinsic
validation, then checks the result against the policy a second time on the
rebuilt syntax tree. The result is one of:

- **Same structure** (possibly reordered or with repeated values): an
  ordinary validated draft.
- **Structural change**: valid, but never applied without a person. In chat
  it is always shown as a proposal card, including in Auto-draft mode, with
  the structural change highlighted. In a job it is held as a pending
  proposal in the job report, where the user can apply changes one by one or
  in groups.
- **Rejected**: a diagnostic naming the tag and the rule goes back to the
  model for a bounded number of corrections; after that the unit is reported
  as rejected.

Units whose source is malformed have no safe projection and are excluded from
AI translation with a visible reason.

This replaces the plain strict structure comparison described in
[`strings.md`](./strings.md) as the AI acceptance rule. That document and
[`workspace.md`](./workspace.md#target-validity) change when the policy is
implemented in `aeria-se`.

## Tools

Tools are Aeria-specific and typed. Arguments are validated before execution;
invalid calls return a typed error to the model rather than failing the
conversation.

### Read tools (no approval)

| Tool | Returns |
| --- | --- |
| `project_overview` | Languages, game version, sheet count, overall progress, detached-unit count. |
| `list_sheets` | Sheets with translatable-cell counts and progress, optionally filtered by name. |
| `read_rows` | One bounded page of `page_translation_rows` for a sheet, optionally filtered by review state. |
| `get_unit` | One occurrence: source, target, note, review state, row context cells, protected-structure legend. |
| `validate_target` | A dry-run of the write validation for a proposed target, without writing. |
| `unit_history` | Git history of one unit's target (read-only `aeria-git`). |
| `pending_changes` | Uncommitted translation changes, grouped by sheet. |
| `get_guidance` | Project guidance text and glossary entries matching given terms. |
| `search_source`, `search_translations`, `similar_translations` | Added when `aeria-search` provides a backend. Until then they are not offered to the model. |

### Action tools

| Tool | Effect | Approval |
| --- | --- | --- |
| `navigate_to` | Opens a sheet or occurrence in the editor. | None (presentation only). |
| `propose_translation` | Validates targets for a small set of units (at most one row page) and writes them as `draft`. | Per the approval mode. |
| `set_note` | Sets a translator note. | Per the approval mode. |
| `flag_needs_review` | Sets `needs_review` on a unit with a reason. | Per the approval mode. |
| `propose_glossary_change` | Adds or edits glossary entries. | Always asks, shown as a diff. |
| `propose_guidance_change` | Edits project guidance. | Always asks, shown as a diff. |
| `estimate_job` | Counts units and estimates tokens and cost for a job scope. | None. |
| `start_job` | Starts a translation job. | Always asks. |
| `job_status`, `job_events` | Progress, rejected units, and worker reports. | None. |
| `amend_job` | Adds instructions for the job's remaining chunks. | None; shown in the conversation. |
| `retry_units` | Requeues rejected or failed units, optionally with extra instructions. | None within the approved scope. |
| `pause_job`, `resume_job`, `cancel_job` | Controls a job. | None. |

Work larger than one row page, such as a whole sheet, several sheets, or the
project, is never done through `propose_translation`. Angelica turns it into a
job.

There are no tools for commit, push, sync, branch changes, source update,
export, settings, deletion, arbitrary files, shell, or arbitrary network
access.

### Approval modes

The user picks one mode per conversation in the AI dock:

- **Chat**: read tools only. Useful for questions and review.
- **Ask** (default): every action tool call is shown as a card with the
  source, current target, proposed target, and a word diff. The user applies
  or rejects each card, or a whole group.
- **Auto-draft**: validated drafts, notes, and `needs_review` flags are
  applied immediately and listed in the conversation with an undo action.
  Overwriting an existing translation still asks, and overwriting a
  `reviewed` unit always asks.

Every applied write uses the compare-and-set `expected` value from the moment
Angelica read the unit. If a person saved the unit in between, or the unit has
an unsaved draft in the editor, the write is refused as a conflict and shown to
the user instead of overwriting their work.

## Translation jobs

Translating a sheet or the whole project is a job run by translation
subagents that Angelica supervises. One long chat cannot do this: its context
would overflow, it would be expensive, and it could not be resumed reliably.

```text
Angelica (supervisor, user's conversation)
   │ start_job / amend_job / retry_units / job_events
   ▼
Job orchestrator (Rust, deterministic)
   │ owns scope, chunk queue, write gate, persistence, resume
   ├── worker subagent: chunk 1 ──► submit_translations ──► validation ──► draft
   ├── worker subagent: chunk 2 ──► …
   └── worker subagent: chunk N
```

### Job orchestrator

The orchestrator is ordinary Rust code in `aeria-ai`. It is the authority for
what a job may touch; neither Angelica nor the workers can widen it.

- **Scope**: selected units, a sheet, several sheets, or the project,
  filtered to untranslated, changed-since-source-update, or `needs_review`
  units. The unit list is fixed when the job starts. Existing translations are
  skipped unless the user approved retranslation when starting the job;
  `reviewed` units are included only with a separate explicit confirmation.
- **Chunks**: units are grouped by sheet and adjacent rows so related text
  shares context, within a per-chunk token budget.
- **Writes**: a worker's result for a unit is accepted only if the unit
  belongs to that worker's chunk. It passes the same tagged-text and
  structural validation as a chat write and is written as `draft` through the
  bulk assisted-write API. An invalid result is recorded as rejected with its
  diagnostic.
- **State**: jobs, chunks, and per-unit outcomes are stored in local SQLite.
  A job can be paused, resumed after a restart, and cancelled. Unfinished
  chunks are restarted with fresh workers. Concurrency, a per-job cost
  ceiling, and retry limits are configurable.
- **Provenance**: the provider, model, and job ID are kept in local job
  history and may be offered as commit-message context. They are not written
  into translation units.

The user's approval for `start_job` covers exactly the approved scope, so
workers run without further prompts. They never produce `reviewed`, never
commit, and cannot write outside their chunk.

### Worker subagents

Each chunk is translated by a subagent with a fresh context: Angelica's job
instructions, project guidance, glossary entries matching the chunk, and the
chunk's units in tagged-text form. Its tools are restricted to:

- `get_unit` and neighbouring rows of its own sheet, for context;
- `get_guidance`, for glossary lookups;
- `validate_target`, to check itself before submitting;
- `submit_translations`, for units in its chunk only;
- `report_issue`, to escalate an ambiguous term, missing context, or a
  glossary gap to the supervisor.

A worker has a turn limit and a token budget. It can use a different model
and effort from Angelica; a cheaper default model for workers is configurable.

### Supervision

Angelica is not kept running while a job works, which would spend tokens for
nothing. The orchestrator wakes her with events: the job finished or paused,
the rejection rate crossed a threshold, a worker reported an issue, or the
cost ceiling is near. She can then amend instructions for the remaining
chunks, retry rejected units with extra guidance, propose a glossary entry,
pause the job, or ask the user. The user can also ask her about a running job
at any time.

### Bulk writes

Ordinary mutations persist one shard per unit. A bulk assisted-write API in
`ProjectSession` that validates each unit, groups the accepted units by
shard, and persists each affected shard once was planned for jobs. The first
implementation writes each submitted string through `set_assisted_target`
instead: chunks are small, and per-unit writes keep the compare-and-set and
structure checks identical to chat writes. A bulk API remains an option if
per-unit persistence proves slow.

## Guidance and glossary

Project guidance and the glossary are human-readable, human-edited files at
the repository root, next to `aeria-collaboration.json` and outside the
Aeria-managed `.aeria/` directory. They are reviewed and merged like any other
file through Git.

| File | Content |
| --- | --- |
| `aeria-guidance.md` | Free-form Markdown: style, register, tone, naming conventions, rules for specific sheets. Aeria reads it as text and passes it to the model within a size limit; it does not interpret its structure. |
| `aeria-glossary.csv` | One term per line, UTF-8, RFC 4180 CSV with a required header, for example `term,translation,note,forbidden`. Line-per-term keeps Git diffs and merges readable and lets translators edit it in a spreadsheet. |

Both files are optional. The glossary is an Aeria-parsed contract and gets its
own format document (`docs/formats/glossary-v1.md`) when implemented. Invalid
rows are reported as visible diagnostics and excluded; they are never guessed
at.

Glossary checks are advisory. A source term whose glossary translation does
not appear in the target is reported to the model and in the UI, but does not
block the write, because inflected target languages rarely keep the
dictionary form. Structural checks remain the only blocking checks.

The files can be edited by hand, in Aeria's editor, or by Angelica through
`propose_glossary_change` and `propose_guidance_change`, which always ask.

## Conversations

Conversations are local, stored per project in application data, and never
committed. The dock lists previous conversations for the open project and can
start a new one. Usage reported by the provider (tokens and cost) is shown per
conversation and per job. A running request can always be stopped.

## Desktop UI

### Angelica panel

The existing AI dock becomes the Angelica panel. It works like an IDE agent
chat:

- **Composer**: multiline input (Enter sends, Shift+Enter adds a line).
  Messages typed while Angelica is working are queued and sent after the
  current turn. **Stop** interrupts the current turn.
- **Model and effort pickers** in the composer toolbar. A change applies from
  the next message and is remembered per conversation; defaults come from
  settings. The effort picker shows only the values the selected model
  accepts.
- **Approval-mode picker** (Chat, Ask, Auto-draft) in the same toolbar.
- **Context**: chips for the automatically attached editor context, which the
  user can remove, and `@` mentions for a sheet, a unit, the current
  selection, or the glossary.
- **Commands**: `/` commands for common requests, such as translating the
  selection, reviewing a sheet, or starting a new conversation.
- **Messages**: streamed replies; collapsible reasoning when the provider
  returns it; collapsible tool-call cards with a one-line summary; proposal
  cards with Apply and Reject; a plan list for multi-step work; inline job
  cards with progress that link to the Tasks panel.
- **Usage**: context-window fill, tokens, and cost for the conversation.
- Editing and resending the last message, and retrying a failed turn.

### Elsewhere

- The translation editor gains **Draft with Angelica** for the selected
  occurrence, which runs a single-unit request and presents the result as a
  normal unsaved draft that the user saves explicitly.
- Settings gain a Providers section with presets, keys, models, default
  model and effort for Angelica and for workers, and **Test connection**.
- Jobs appear in the Tasks bottom panel with per-unit outcomes; rejected
  units open in the editor.

## Milestones

Each milestone is a separate change with its own documentation update.

1. **Providers**: `aeria-ai` transport, provider settings, keyring storage,
   OpenCode Go preset, model capabilities including effort, and connection
   test.
2. **Read-only agent**: agent loop, read tools, the Angelica panel with
   model and effort pickers, local conversations, Chat mode.
3. **Assisted writes**: `aeria-se` tagged projection, rebuild, and structure
   policy (move, repeat, droppable constructs), `set_assisted_target`,
   validation feedback loop, Ask and Auto-draft modes, **Draft with
   Angelica** in the editor. The `gender` construct follows as a separate
   change once its native form is confirmed against the source corpus.
4. **Guidance and glossary**: `aeria-guidance.md`, `aeria-glossary.csv` with
   its format document, glossary tools, and advisory checks.
5. **Translation jobs**: job orchestrator and store, worker subagents,
   supervision events, and job controls in the Angelica panel.
6. **Search tools**: source search and translation memory once
   `aeria-search` exists.

Later candidates: read-only web lookup with per-request approval (results are
untrusted data), and an AI QA pass that can only flag `needs_review`.

## Open questions

- The exact native form of the player-gender condition, confirmed against
  source strings that already vary by the player's gender.
- Which source construct kinds the policy marks as droppable.

- The exact glossary columns and matching rules (case, whole word, sheet
  restrictions).
- Default concurrency, retry, rejection-threshold, and cost-ceiling values
  for jobs.
- Whether conversations should be exportable for sharing with other
  translators.
