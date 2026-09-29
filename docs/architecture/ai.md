# AI translation

AI assists translation, but it is never a source of identity truth.

Aeria remains usable without a configured AI provider.

The proposed design of the Angelica agent, its tools, and batch jobs is in
[`ai-agent.md`](./ai-agent.md).

The proposed [agent localization system](./localization-system.md) replaces
translation job workers and the guidance, glossary, and voice profile files
with localizer and critic subagents and agent-maintained project knowledge,
and lets agents write final translations.

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
Updating from the provider replaces the list and keeps the efforts, context
window, and image input of models it still reports. A listing may include models that the
provider serves only through other APIs; a connection check reveals them.
Models can also be added by ID for providers without a listing. Listed models
have no known context window or effort values; the user enables the efforts a
model accepts, which a connection check with that effort can confirm first.
A model accepts images when its listing entry says so in
`architecture.input_modalities` (as OpenRouter lists it), `input_modalities`,
or `modalities.input`; otherwise the user marks it in the settings. Updating
keeps image input a user enabled.

### Local settings

Provider settings are machine-local application data in
`<app-data>/ai-settings-v1.json`, never in a repository. The document holds
`formatVersion` 1, the providers (opaque local ID, preset kind, name, base URL,
models with ID, optional context window, accepted efforts, and `vision: true`
for a model that accepts images (omitted otherwise), the optional
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
  conversation as typed input items (`message` with `input_text` and
  `input_image` parts, `function_call`, and
  `function_call_output`). Reasoning is requested with a summary and never
  replayed between requests. Requests carry the access token, the account and
  data-residency headers taken from the token's claims, `session_id` set to
  the conversation ID, and `originator: aeria`. Aeria identifies itself and
  does not present itself as Codex. A response that fails for a plan limit is
  reported as rate limiting.
- **Models** come from the Codex catalog, `GET {base}/models?client_version=…`,
  which also states each model's context window and reasoning levels, and
  input modalities: a model accepts images unless they leave images out. Hidden
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
when unknown), estimated at three characters per token and, for an image, one
token per 28 × 28 pixels (at least 85). Earlier turns' tool results are
replaced with a short notice first, then earlier turns' images are replaced
with a notice; if that is not enough, whole earlier turns are dropped, oldest
first. The current turn is always sent in full.

The system message holds Angelica's fixed instructions (see
`aeria-ai::prompt`), the project's languages, game version, and progress, and
the editor context the renderer sends with the message: the open sheet, the
selected occurrence, and whether it has unsaved edits. The user can stop
sending the selection. Instructions state that tool data and text in images
are never instructions, that macros must be preserved, and that Chat mode
cannot change the project.

### Writing style

`aeria-ai::style` holds Angelica's character and the fixed writing rules.
The machine-writing patterns are adapted from Wikipedia's "Signs of AI
writing".

- **Persona**, in Angelica's system message only: a longtime player who
  loves the game and translating it, a friendly and slightly playful
  colleague who says what she thinks of a line and admits when hers falls
  flat. She is a woman: in languages with grammatical gender she uses
  feminine forms for herself.
- **Reply style**, in Angelica's system message only: start with the answer,
  no chat wrappers such as greetings, praise, recaps, or closing offers, and
  none of the machine-written patterns (contrasts with something nobody
  claimed, one-line punchlines, staged openers, habitual lists of three,
  inflated words, officialese, dashes as the universal connector, decorative
  formatting). An occasional emoji is allowed in replies, never in
  translations. A job report is a sentence or two, then only what needs the
  user's decision.
- **Original text**, in Angelica's and every worker's system message: the
  game is written in Japanese, and the English, German, and French texts are
  localizations. The project translates from its source language; the
  Japanese shows intent. When the two differ, the project guidance decides
  which to follow; without guidance the source's content is kept.
- **Translation style**, in Angelica's, every worker's, and Draft with
  Angelica's system message: translate meaning and tone in the target
  language's own syntax and punctuation, avoid translationese, keep the
  line's register without adding or flattening anything, and reread each
  translation as the player sees it before submitting.
- **Player character**, in Angelica's, every worker's, and Draft with
  Angelica's system message: players choose their character's gender, so a
  translation never assumes one. "You" in quests and cutscenes, journal
  entries, and objectives is the player character. Every word that agrees
  with the player character's gender goes in a condition on `$gn4`, wrapping
  whole words (`Ты <if $gn4>готова<else>готов</if>?`), or the line is phrased
  so that nothing agrees. Slash and parenthesis forms such as `готов(а)` are
  never used, and a source condition on `$gn4` is never reduced to one
  branch.

Persona and style are defaults: the project guidance is described as taking
precedence over them. They only guide the model; validation, glossary
warnings, and every write path are unchanged.

### Images

A user message can carry up to 6 images, pasted into the composer or chosen
from files. The renderer scales each one so that no side exceeds 2,048 pixels
and it has at most 1920 × 1080 pixels, then sends a PNG or JPEG file:
screenshots and other images stay PNG for legible text, photos stay JPEG, and
a PNG larger than the file limit is encoded as JPEG. `aeria-ai::images`
checks every file again: only PNG and JPEG, at most 3,750,000 bytes (so the
base64 form stays within the 5 MB common vision APIs accept), and each side 1
to 2,048 pixels, read from the file header. A refused image fails the message
with `angelicaInvalidImage`; nothing is stored.

Images are sent only to a model marked as accepting images. A message with
images for any other model is refused (`angelicaModelWithoutImages`), and the
composer says so before sending. When a conversation continues with a model
without image input, earlier images are left out and the message text says
how many were not shown, so Angelica can ask the user to describe them.

In a request, a message's images follow its text as `image_url` parts with a
`data:` URL (Chat Completions) or `input_image` parts (Codex Responses). The
text ends with the IDs of the images sent, in order, so Angelica can pass
them to a job, and names any image whose file is gone as no longer
available.

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
| `other_languages` | The same row's translatable strings, or one column, in the game's other client languages as macro text, each bounded like other cell text; `null` where a language has no such string. Context for intent (Japanese is the original), wording, and tag placement; the translation is still made from the source language. |
| `dialogue_context` | For a line of a quest or cutscene sheet (see [Dialogue context](#dialogue-context)): the quest's name and translation, its journal entries and objectives (up to 24 each), up to 40 spoken lines before and after the line (8 and 4 by default), each with its key, speaker label, source, translation, and review state, the voice profiles of their speakers and the speakers without one, and optionally the line in the other client languages. |
| `list_speakers` | Speaker labels of quest and cutscene speech, optionally containing a query or only those without a voice profile, the most lines first, up to 200 per page, each with its number of lines and whether it has a profile. |
| `speaker_lines` | One speaker label's lines across every quest and cutscene with translations, each with its position among the speaker's lines, and the speaker's voice profile: a page of up to 30 lines, or with `spread` up to 30 lines sampled evenly across all of them (line `i × total / count`). An unknown label returns up to 20 labels that contain it: those that start with it first, then those with the most lines. |
| `get_voices` | The voice profiles of given speaker labels, or every speaker label with a profile. |
| `pending_changes` | Uncommitted translation changes from `aeria-git`, up to 200. |
| `unit_history` | Committed history of one unit, up to 50 entries. |
| `navigate_to` | Opens an occurrence in the editor; changes nothing in the project. |

Each target or note text is cut at 2,000 characters, a translatable cell's
source at 8,000 (a partial source cannot be translated), and each result
at 24,000 characters, with a visible notice. Every tool call takes the project
lock only for its own read.

Cells returned by `read_rows` and `get_unit` include `constructs`, what each
macro of the source does and what a translation may do with it (see
[`strings.md`](./strings.md#constructs)); a malformed source is marked
`malformed`. Angelica writes translations as macro text.

### Search tools

Offered in every mode, backed by [`aeria-search`](./search.md):

| Tool | Result |
| --- | --- |
| `search_source` | Translatable strings whose source text matches the query, best first, with their translations and review states; optionally one sheet; up to 50 per page with `more`. |
| `search_translations` | Bound translations whose text contains the query, ignoring case and macros, with their sources, in source order. |
| `similar_translations` | Translation memory for one string (by location) or a given text: up to 10 translated strings with a similar source, most similar first, with a similarity from 0.5 to 1. |
| `glossary_candidates` | [Terminology candidates](./search.md#terminology-candidates) the glossary does not have yet, those in the most strings first, optionally containing a query, from one data sheet, or in at least a number of strings (3 by default): up to 100 per page, each with its string and occurrence counts and up to 3 cells where it is a name, with the project's translation there. See [Filling the glossary](#filling-the-glossary). |

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
| `validate_target` | Checks a translation of one string against the [assisted structure policy](./strings.md#assisted-structure-policy) without writing, and returns the rule violations, if any. |
| `propose_translation` | Accepts 1 to 20 translations. Each is checked; rejected ones return what to fix; valid ones are submitted and reported as `applied`, `awaitingApproval` (with a proposal ID), `conflict`, or `failed`. |
| `propose_voice_profile` | Sets up to 50 characters' voice profiles and removes speaker labels from profiles, as one change that always waits for approval (see [Voice profiles](#voice-profiles)). |

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
otherwise the proposal becomes a conflict. See
[Proposals](#proposals) for how a new glossary change builds on one still
waiting.

#### Filling the glossary

Angelica can fill the glossary from `glossary_candidates`. Finding the
candidates is deterministic; choosing which are terminology and how to
translate them is hers, and every entry reaches the file only through an
approved `propose_glossary_change`. Her instructions: take the most used
candidates first; keep names of characters, places, and factions, items and
their categories, actions, statuses, mechanics, and recurring interface
terms; skip ordinary words, generic labels, and one-off names; prefer the
project's existing translation of the name, then how existing translations
render it, and the Japanese when the meaning is unclear; add notes on kind,
gender, or declension where useful; propose up to 100 entries per change and
continue without waiting for the user, since the next change includes the
one still waiting.

Candidate pages are positions in a ranking that does not depend on the
glossary. Terms the glossary has are left out of each page, so continuing
from `nextOffset` after a glossary change neither repeats terms Angelica
skipped nor misses new ones. A page reports how many ranked terms the
glossary does not have (`notInGlossary`) and how many follow the page.

Draft with Angelica includes the guidance and the glossary entries matching
the string.

### Voice profiles

`aeria-voices.md` ([Voice Profiles Format v1](../formats/voices-v1.md)) is a
third optional, human-edited file at the repository root, shared through Git.
Each profile names a character by the speaker labels of the game's dialogue
keys and describes how the character speaks in the target language: register,
forms of address, pronouns, archaisms, and examples. It is edited in the
**Glossary and guidance** dialog or by hand, and read like the guidance, so
edits take effect on the next message.

Angelica's system message lists up to 100 speaker labels that have a profile.
`dialogue_context` and `speaker_lines` include the profiles of the speakers
they return, workers receive the profiles of their chunk's speakers, and Draft
with Angelica receives the profile of the string's speaker. Each profile's
text is cut at 2,000 characters in requests. Ignored profiles are reported as
project file problems.

`propose_voice_profile` is offered in Ask and Auto-draft modes. It takes up
to 50 profiles, each with its speaker labels and text, and labels to remove,
and records them as one file change: removals apply first, then each profile
replaces the one profile that names any of its labels or is added, and the
result is written in canonical form. A change is refused when one profile's
labels belong to different profiles, when a label is named twice, or when the
file has ignored profiles. The change always waits for approval and is applied
like a glossary change: only if the file still has the content it was proposed
against, and only if every profile of the new file is usable. A second change
proposed while the first waits builds on it and replaces it (see
[Proposals](#proposals)); her instructions still tell her to put every
profile of a turn in one call.

Angelica can write the profiles herself. Her instructions for many characters
are: take speakers from `list_speakers` without a profile, the most lines
first, skipping `SYSTEM`, choice labels, and labels with a number; read each
one's lines with `speaker_lines` and `spread`; group labels that belong to one
character; write rules with short examples in the target language; and
propose the turn's profiles in one change, then continue after the user
applied it.

### Dialogue context

Quest and cutscene strings are dialogue. Their structure is read from the row
keys by `aeria-source` (see [`source.md`](./source.md#dialogue)) and shaped by
`aeria-ai::dialogue`. It is context only: nothing about it is recorded, and
speaker labels, line order, and quest links never decide identity, validation,
or writes.

*Spoken lines* are speech and other lines; journal entries and objectives are
not. `dialogue_context` returns neighbours only for a spoken line. The
instructions tell Angelica to read a line's scene when its meaning, tone, or
addressee is unclear, to say when she inferred who is addressed, and that
`dialogue_context` can show the line in the other client languages. How the
Japanese original is used is described in [Writing style](#writing-style).

The first `list_speakers` or `speaker_lines` call builds the speaker index
(see [`source.md`](./source.md#dialogue)) outside the project lock.

### Proposals

Proposals are stored beside their conversation in
`<id>.proposals.json`. A translation proposal records the location, source,
rebuilt target, and expected state; a file proposal records the file, its new
content as `target`, and its content when proposed as `expected.target`
(`null` when absent). Both record status (`pending`, `applied`, `rejected`, `conflict`, or `failed`),
an optional message, and the creation time. At most 2,000 are kept, dropping
the oldest settled ones first. Deleting a conversation deletes its proposals.

The user settles proposals in the panel, which Angelica does not see, so
each turn's system message lists where the conversation's proposals stand:
every waiting proposal other than a translation (at most 20) with its ID and
subject, such as a glossary change with its entry counts before and after,
the latest 8 settled ones with their status and message, and counts of
translation proposals by status with up to 5 waiting locations. Her
instructions tell her to check this list instead of guessing what the user
decided.

A glossary or voice-profile change is built on the latest change to the same
file that is still waiting in the conversation and was proposed against the
file as it is now. The new proposal records the current file as its expected
content, so applying it applies both, and the replaced proposal becomes
`rejected` with a message saying it was replaced. The user therefore approves a
single change, and Angelica can keep proposing batches without waiting. A
guidance change is a whole new text and replaces nothing; its result tells
Angelica to wait for the user's decision before proposing another.

### Draft with Angelica

The editor's **Draft with Angelica** uses Angelica's default model for one
string without tools. The request carries the source with its list of
constructs, the row's context cells, the current translation and note, and the
project languages. For a quest or cutscene string it also carries the
string's speaker label and its scene as workers receive it, with the six
spoken lines before the string and the three after it. The reply is read between `<translation>` markers
and checked; a refused reply is sent back with the violations, for
at most three requests in total. The result becomes an unsaved draft in the
editor, which the user saves explicitly.

### Translation jobs

A sheet, several sheets, or the whole project is translated by a job: the
localizer translates one chunk of strings at a time as a unit of work, run by
a deterministic orchestrator and supervised by Angelica. The localizer's
design and the reasoning behind it are in
[`localization-system.md`](./localization-system.md); this section describes
what is implemented.

Angelica has `estimate_job` in every mode and `job_status` and `job_events`
to report on jobs. In Ask and Auto-draft modes she also has `start_job`,
`amend_job`, `retry_units`, `set_job_workers`, `raise_job_limit`, `pause_job`, `resume_job`, and
`cancel_job`.
`start_job` never starts anything: it records a job proposal with the scope,
instructions, concurrency, the estimate, and a token limit of twice
the estimate (at least 200,000). The user starts the job from the proposal.
`start_job` can also name up to 4 images of its conversation, for example a
screenshot showing where the strings appear; each is sent with the contract
and writer requests of every chunk.
An image ID the conversation does not have is refused, and so are images
while the jobs model does not accept them. The estimate does not include
them.

A scope is a list of sheets, or every sheet with translatable strings, and a
filter: untranslated strings (the default), strings that need review, or
untranslated strings and drafts. Reviewed translations are never included.
The string list is fixed when the job starts, together with each string's
current target and review state. Sheets are taken in domain order: names
first, since all other text refers to them, then actions and statuses, items,
interface text, lore, and quests and cutscenes last, keeping the given order
within a domain. Strings are grouped in order into chunks,
never across sheets. A quest or cutscene sheet is one scene, so its strings in
the job are one chunk of up to 240 strings; a larger one is split into chunks
of even size. Other sheets are grouped into chunks of at most 30 strings and
12,000 source characters. The
estimate is the number of strings and chunks and a token count. When
earlier jobs of the project with the same jobs model (provider, model, and
effort) finished at least three chunks together, the count is the chunks
times their average tokens per finished chunk (from the latest 20 such
jobs). Otherwise it is a formula measured on quests: 50,000 tokens per chunk
for instructions and project knowledge sent with each request, and 13 tokens
per source character for the script read by every step and the translation
written and fixed (`chunks × 50,000 + characters × 13`). The proposal says
which basis was used.

#### The localizer

`aeria-ai::worker::prepare_unit` turns a chunk into a unit of work, and
`aeria-ai::localizer::localize` translates it. A unit is a script: for a
quest or cutscene sheet, its lines in play order from the first chunk string
to the last with 20 lines on each side, each with its role (journal entry,
objective, or speaker label); the chunk's strings are marked for translation
and the other lines are context with their current translations. A chunk of
another sheet is its strings with their row context. Each string carries its
constructs, current translation, note, and up to two translation-memory
matches; every line carries the game's other client languages as evidence.
A line whose French or German text has a condition on `$gn4` that its source
lacks is marked as varying with the player character's gender. The unit
has its text domains: the roles of a quest's lines (journal, objectives,
system text, dialogue) or the domain of another sheet, read from its name
(names, items, actions, interface, or lore). It carries the
[project knowledge](../formats/knowledge-v1.md) that applies to it, human
entries first: the guidance, the style of its domains, lessons in use, the
terms that occur in its lines (at most 80), its speakers' profiles (at most
12), and the story of its sheet so far, and the job's instructions as they
are when the chunk starts. Writers get short notes for the conventions of
names, items, actions, interface text, and lore. The localizer's instructions tell it that content comes only from the
source line: the other languages decide tone, voice, address, and gender,
never content. A sheet whose dialogue cannot be read is translated as plain
strings.

#### Study

Before a job's first chunk, its runner studies the job's scope once and
records a `study` event (`aeria-ai::study`, driven by the desktop's
`job_study`). For each text domain of the scope whose agent style entry is
missing, and for `general`, a researcher reads up to 40 samples spread over
the scope (the lines of up to 40 quest and cutscene sheets, or the first
rows of up to 10 sheets of another domain) in every client language and
writes the domain's style entry. For each speaker label with at least three
lines in the scope's quests and cutscenes and no profile in either layer,
the most lines first and at most 200, a researcher reads up to 30 of the
speaker's lines sampled evenly across the game with their other languages
and writes a profile: who the character is and their gender, their voice in
the Japanese, their address from the French and German, and how they sound
in the target language, with examples. System text, player choices such as
`Q1`, and labels without letters are skipped. The study starts building the
source index, uses the job's model, counts toward the job's tokens, and
pauses the job with a reason when the provider fails.

Before each chunk's contract, a researcher lists the unit's terminology the
knowledge does not have (at most 40 terms), reads up to six existing
translations of each from the source index, and decides renderings; a
second request checks them for grammar and meaning, and the checked terms
are written to the agent layer with `study` in their note. The unit's
knowledge is then read again. A term another chunk wrote meanwhile keeps
that chunk's rendering.

After a quest or cutscene unit, the story part of its contract is added to
the sheet's entry in `story.md` (kept to its last 3,000 characters), so the
sheet's later chunks know what happened. Consistency findings that say the
knowledge itself is wrong are recorded as `knowledge` job events.

#### Steps

The localizer runs six steps; the requests of a step run in parallel, at
most 8 at a time per chunk and at most 24 at a time across all lanes of a
job:

0. **Terms**: the study of the unit's terms described above.
1. **Contract**: one request reads the whole script and writes the decisions
   every writer shares: the story and tone (between `<story>` tags), a table
   of address between
   speakers and toward the player character, genders, names and terms, and
   the form of journal entries, objectives, and system text.
2. **Writing**: the chunk's strings are split into parts of at most 40, in
   order and of even size, and each part is written by its own request,
   which sees the whole script and the contract and rereads its text before
   answering. Replies are lines of the form `L12: text`.
3. **Critics**: three requests per part check its lines. A blind reader sees
   only the target text (with the contract and four lines before the part);
   a fidelity check compares each line with its source; a check of the player
   character and address sees the source, French, German, and target with
   the knowledge and the contract; when the unit has knowledge, a
   consistency check compares the lines with its terms, characters, style,
   and lessons. Each returns flags with a line, a severity, a problem, and a
   hint.
4. **Fixes**: one request per part with flags corrects the flagged lines;
   changes to other lines are ignored.
5. **Recheck**: the critics read the changed and flagged lines again; a
   remaining major flag gets one more fix. A line whose major flag was not
   changed by that fix needs review.

After writing and after each fix, every line is checked as it would be
written: structure against the assisted structure policy, a speaker label or
role marker at its start, two versions joined by an arrow, and a repeat of
the previous line's translation although the sources differ. Refused lines
go back for correction up to twice; a line still refused is rejected.

Each role has the reasoning effort that served it best, clamped to the
efforts the model accepts: high for writers and the check of the player
character, medium for the contract, the blind reader, the consistency
check, and fixes, low for the
fidelity check and structure corrections. The jobs effort from the settings
is a ceiling for every role.

A job has a quality, chosen by Angelica with `start_job` and shown on the
job card: `fast` (the default) as described above, or `careful`, for story
quests whose scenes must hold together. A careful unit is written by one
writer, every role but structure corrections asks for a high effort, and the
recheck reads the whole unit again, twice, fixing its major findings each
time. A careful job's estimate, and so its token limit, is twice a fast
one's.

Nothing is written until the unit is done. Each translation is then written
through `ProjectSession::set_assisted_target` against the recorded state,
without permission to replace a reviewed string, followed by its review
state: a translation with no open major flag is written as `reviewed` and
counts as `finished`; one with an open flag is written as `needsReview`,
counts as `flagged` with the flag as its message, and is reported as a job
event. A string changed meanwhile is skipped as a conflict. The contract and
writer requests carry the job's images when the model accepts images; after
the job's conversation is deleted they are left out.

Jobs are machine-local application data, one SQLite database per project in
`<app-data>/jobs/<key>.sqlite3` with the conversation key. A job records its
conversation, specification (scope, instructions, worker model, token limit,
concurrency, and the references of its images), status (`running`, `paused` with a reason, `completed`,
`cancelled`), token usage, events, and each string's chunk, status
(`pending`, `running`, `finished`, `flagged`, `rejected`, `failed`,
`conflict`, and `drafted` for jobs of earlier versions),
attempts, and message. The worker model is the jobs model from the settings,
or Angelica's default model. Settings show an effort choice for jobs even
while they use Angelica's model; choosing an effort there stores Angelica's
current model with that effort as the jobs model.

Concurrency is 1 to 16 workers. When Angelica does not choose, a job of 100
chunks or more gets 16 and a smaller one 8; either way a job never gets more
workers than chunks. The proposal shows the count. The user can change it on
the job card and Angelica with `set_job_workers` (for example after rate-limit
errors), for a job that was not cancelled; this changes speed, not the
strings or the token limit.

A running job has `concurrency` lanes. Every two seconds the runner reads the
job's count and starts missing lanes when it grew; a lane above a lowered
count stops before claiming its next chunk. Each lane claims the next chunk,
checks first that the job's project is still open, and pauses the job when
the token limit is reached or when, after 40 finished strings, more than 30 %
were rejected. A network, timeout, rate-limit, or unavailable failure returns
the chunk's strings to the queue and waits (20 seconds times the failures in
a row); the third failure in a row pauses the job, as does a rejected key.
Other provider errors fail the chunk's strings. An interrupted chunk writes
nothing.
Pausing or cancelling returns claimed strings to the queue; a job left
running when Aeria closed is paused the next time its project's jobs are
read. Rejected, failed, and skipped strings can be queued again.

Lanes share the job store: a chunk is claimed, and an event numbered, in one
write transaction, so parallel lanes never claim the same chunk or wait on a
lock upgrade. A busy store is retried (up to five times with a growing
delay) before the job pauses, and a chunk's outcomes are recorded with the
same retries so its strings never stay claimed. A job's summary counts its
active workers, the chunks being translated right now.

A chunk's outcomes and token usage are recorded when the chunk ends. Usage is
summed as each request finishes, so a chunk the provider interrupts still
records the tokens it spent.

While a runner runs, each lane also reports its live activity: the chunk,
sheet, and first and last row it translates, its phase (claiming a chunk,
loading context, waiting for the provider, reasoning, writing, recording
results, waiting to retry, or stopped), the localizer step it is on (contract,
writing, critics, fixes, or recheck) and how many of the step's requests have
not answered yet, its strings written and tokens used in the chunk, its
chunks done, and when it last changed or received anything from the
provider. Strings count as written when the unit is done, because the
localizer writes a chunk at its end. Streaming reasoning and text of any of
the step's requests count as activity. This state
lives only in memory while the runner runs and is never stored. An expanded
job card polls it every second while its Workers tab is shown, and marks a
lane that has waited on the provider without data for 30 seconds as quiet
and for 90 seconds as stalled; the client's read timeout ends the request
after 180 seconds of silence, which requeues the chunk.

The job list in the Angelica panel shows each job as a compact card with its
status, progress, and controls; expanding it shows the Workers (while
running), Problems (filterable by outcome, with retry per retryable outcome;
strings left for review are listed with the reason), Events, and Job tabs. A
job that no longer runs (paused, completed, or cancelled) can be removed from
the list, which deletes its local record, strings, and events; the
translations it wrote stay in the project. A running job must be
paused or cancelled first.

A job's summary includes its chunk count, its finished chunks (chunks with
no pending or running strings), and, once a chunk finished, a projection:
the tokens used so far plus their average per finished chunk for each
unfinished chunk. The card shows the projection and warns when it exceeds
the limit. The user can change the limit of a job that was not cancelled to
any value above the tokens already used; a job paused at its limit shows a
limit field prefilled with the projection plus a quarter (rounded up to
10,000) and resumes with the new limit. Angelica cannot change a limit
herself: `raise_job_limit` records a proposal with the job's use,
projection, current and new limit, and approving it sets the limit and
resumes a job that paused at its old one.

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

A user message references its images by ID, format, width, and height. The
files are kept beside the conversation in `<id>.images/<image id>.png` or
`.jpg`, written through a synced temporary file and rename, and deleted with
the conversation. A file is checked against its reference when read.

## Batch workflow

Batch translation runs as [translation jobs](#translation-jobs):

- jobs are resumable and stored locally
- a token estimate is shown before a job starts
- related strings are batched per sheet for context and cost efficiency
- provider or structural failures are isolated and retryable
- validated results are written directly in the Git working tree, as `reviewed` when the localizer's critics left nothing open and as `needsReview` otherwise
- a failed/unsafe result is not persisted as a successful translation
- commits remain an explicit human action

Model/provider provenance belongs to local job/history/commit context rather than permanently cluttering every translation unit.

## Validation

The Rust side constructs structured context and validates responses. Macro/structure invariants and glossary/QA checks run before a translation is accepted.

Translation jobs write final translations as `reviewed` (see
[the localizer](#the-localizer)); Angelica's own translations stay drafts
until the user applies or approves them.
