# Machine translation of the project

Status: **implemented** in `aeria-model` and the desktop (Settings → Machine
translation, Translation → Machine translation).

Machine translation fills the untranslated strings of `po/` with a language
model. It is a tool a person runs: a program splits the work, sends requests
in parallel, checks every answer, writes what passes, and continues from where
it stopped. The model gets one small, finished task per request.

## Why

The game is 859,426 strings, 10.4 million English words. Translating it
through a model in a conversation fails: a model decides when its turn ends,
and a long run of similar translations ends it early. Tools that translate
whole games with language models are run by a person instead: they sign in to
a provider, choose what to translate, start the run, and review the result;
the program sends batches to the model, keeps each finished line, and
resumes. Glossaries come first and only the relevant entries go into a
request; answers are checked automatically and only failing lines are sent
again. Research on model translation agrees on the parts that matter:
retrieved example translations help more than term lists, and repeated
refinement passes change fluency and style but rarely fix meaning.

People keep the work that needs judgment: the style and terms (see
[`knowledge.md`](./knowledge.md)), the name sheets every later run follows,
reading and fixing the result (a fixed translation is an example for the next
batches of its file), fuzzy strings after a game update, and text that
deserves translation by hand.

## Provider

One provider: a ChatGPT subscription, through the sign-in of the public Codex
client and the Codex backend's Responses API, as open-source agents do. OpenAI
offers no official way for other applications to use a plan; the method may
stop working, and requests count against the plan's Codex limits. The
settings say so.

- **Signing in** (Settings → Machine translation) starts OpenAI's device
  sign-in: Aeria shows a code, opens OpenAI's page, and polls until the code
  is entered. Only the refresh token is stored, in the OS credential store;
  access tokens live in memory and are refreshed shortly before they expire,
  one refresh at a time, so a rotated refresh token is never used twice.
  Signing out deletes it.
- **Requests** go to `POST https://chatgpt.com/backend-api/codex/responses`
  with `stream: true`, `store: false`, `originator: aeria`, and the account's
  ID from the token's claims.
- **The model and reasoning effort** are chosen in the settings from the
  backend's catalog (`GET …/models`) and kept as preferences of this
  computer.

## What is translated

A run takes chosen sheets and folders of sheets (such as `quest/001/`), or
the whole project, and translates every string whose `msgstr` is empty and
that is not fuzzy. On request it also translates fuzzy strings, giving the
model the previous source and translation, and clears the mark of each one it
writes. Before starting, the dialog shows how many strings the chosen sheets
still need, from the project's progress.

A run can also take a list of strings by `msgctxt` (`Options::contexts`);
it then translates only those of them that are untranslated. Project
search uses it to translate found strings again after clearing them (see
[`search.md`](./search.md#translating-again)).

There is no job store and no state outside the files. What is left is exactly
the strings still untranslated, so a run after a stop, a usage limit, or a
failed batch continues where the last one stopped. One run goes at a time.

## Batches

Strings are grouped in file order, which keeps a scene's dialogue together and
a sheet's neighbouring rows together:

- the untranslated strings of a scene file (`quest/`, `cut_scene/`), when
  there are at most 150, are one batch;
- otherwise a file's untranslated strings are split into batches of 100.

A batch never spans files. A file whose untranslated strings are one batch
is packed into one request with the files after it, up to 150 strings and 12
files, so that the many small quests and leftovers of sheets do not each pay
for a request of their own; a batch of a file split into several is a
request alone. The batches of a scene go one after another: the next part of
a quest or cutscene is sent when the one before it has answered, so it sees
that part's translations among its examples. Other requests go side by side.

The batches of the name sheets go first, one sheet at a time in the order of
`NAME_SHEETS` in `aeria-model` (places, towns, races, and tribes; classes,
statuses, actions, and traits; items, mounts, and minions; titles, residents,
monsters, and objects; FATEs, duties, and quests), and the translated names
are read again after each of them. Everything else follows in one queue. So
an action named after a place, a quest named after a character, and every
line of dialogue use the names the same run just wrote, and one run of the
whole project needs no care about the order of sheets.

## The request

The instructions, identical for every request of a run, come first so the
provider serves them from its prompt cache: the
[translation rules](./knowledge.md#translation-rules) for the target language,
the macro authoring reference of [`strings.md`](./strings.md),
`aeria-knowledge/style.md`, and the form of the answer.

The task is JSON built from the files as they are when the request is sent:

- `names`: the game's names that occur in the request's sources or a quest's
  title as whole words,
  with their translations from the name sheets of `po/` (`Action`,
  `BNpcName`, `ENpcResident`, `Item`, `PlaceName`, `Quest`, `Status`, and
  others; short, capitalized strings without macros; a name translated
  several ways gives its most frequent translation), at most 80;
- `terms`: terms of `terms.csv` that occur in the sources, with their notes
  and forbidden variants, at most 60; a term goes only when it applies to a
  string of the request, not counting the strings with an exception for it;
- `files`: each file of the request on its own, with
  - `about`: the file's header comment: its sheet, a quest's title, and
    whether its strings are in play order;
  - `speakers`: the speaker labels of the file's strings (`ALPHINAUD`) whose
    letters match a translated name, with the name and its translation;
  - `examples`: translated strings of the same file, nearest to the batch
    first: up to 40 shared among the files, at least 5 each;
  - `strings`: each string's ID (unique in the request), source, and `#.`
    lines (the other client languages, speaker or kind, the row's other
    cells, macro legends; the legends are made again from the current macro
    catalog, so what the catalog learned since the file was made reaches the
    model before a game update rewrites the comments), `gendered`: the texts (`source`, `fr`, `de`)
    whose line has a condition on the player character's gender, so the
    translation most likely needs one too, `maxLength` for an interface
    label (below), the previous source and translation of a fuzzy one, and
    `termExceptions`: the terms of `terms` that do not apply to it (see
    [`po-project.md`](./po-project.md#term-exceptions)), which the
    instructions say to translate by their meaning.

The editor's string guide shows a person the same names, terms, speaker,
length, and gender marks of one string; `aeria_model::hints` reads them for
both (see
[`desktop-editor-ui.md`](./desktop-editor-ui.md#string-guide)).

All requests of a run share one `prompt_cache_key`. A probe of this provider
measured that requests with the same key and prefix got 99 % of the prompt
from the cache once one request had stored it, eight parallel requests on a
cold prefix 25 %, and requests each with its own key 13 %; so the first
request of a run goes alone and the others start when it has answered.

The answer is one JSON object from string IDs to pairs: the first words of
the string's source (up to three, macros may be left out), then the
translation. The first object in the answer is read. A string the model leaves
out stays for a later run.

In a long run of short, similar strings the model can slip and give a string
the next one's translation; numbered IDs alone do not show it, and the
translation passes every check. The repeated words do: an answer whose words,
compared by letters and digits only and case ignored, do not occur in its own
source in their order (macro tags included, since the model may copy from
them or skip a conditional opening such as `<if $n2>Lv. …</if>`; not
necessarily together, since it leaves out the conditions between them, as
in `you scan` of `<if ($gs1 == $gs2)>you<else>…</if> <if …>scan<else>scans</if>`), or that gives a bare
translation, is refused as belonging to another string
(`fit::matches_start`). Empty words are accepted only for a source that
begins with a macro. Macro tags are read whole: a `>` inside parentheses or
quotes, as in `<if ($n1 > 0)>`, does not end a tag.

## Interface labels

The game lays out its interface for the official localizations. A short
string (at most 40 characters shown, no line break) of an interface sheet
(`Addon`, `AddonTransient`, `BaseParam`, `ClassJob`, `ClassJobCategory`,
`Completion`, `ConfigKey`, `ContentType`, `ItemSearchCategory`,
`ItemUICategory`, `MainCommand`, `MainCommandCategory`) gets `maxLength`: the
characters shown by the longest of its English source and the German and
French lines of its context, macros not counted, `<nbsp>` as one, `<shy>` as
none (`fit::length_budget`). The request asks for a translation within that
length, with the language's usual abbreviations when needed, and a longer one
is refused. Names of items, characters, and places get no length: their
tooltips show them whole.

## Checking and writing

Every translation is checked like a save in the editor
([`po-project.md`](./po-project.md#checking)), after the answer's first
words and an interface label's length. One more check is machine
translation's alone: when the source chooses a word by whether a person of a
log message is the player (`<if ($gs1 == $gs2)>scan<else>scans</if>`), the
translation needs a comparison with that person whose branches both have
words, so that a word agreeing with the person is chosen with it, rather
than one form after the name that agrees with only "you" or someone else
(«Вы … осматривают»). Repeating a whole phrase in each branch always
satisfies it; a person's save is not held to it. Likewise a machine
translation may not keep a sound of the English localization as it is, such
as `\<sigh>` or `\<click>` (in macro text `\<` is a literal `<`): it renders
it in its language. An answer that is not valid JSON,
most often for a quote the model did not escape inside macro text, is read
entry by entry (`"id": ["first words", "translation"]`, a bare quote read as
part of its string); what is read this way is checked like any answer, so a
wrong reading is refused, never written. A string the answer has no
readable translation of is asked for again with the failing ones. A batch's
failing translations go back up to twice, each time in one request with each problem stated; what
still fails stays as it was and is listed in the run's status with its
problems and the model's last translation, which is never written. Advice
does not reject a translation. The listed strings can be translated again
in one run of exactly those strings (`translation_retry`), after a person
fixed what refused them, such as a term that does not apply (see
[`po-project.md`](./po-project.md#term-exceptions)).

A batch is written as soon as it is checked: its file is read, the `msgstr`
of each string that is still untranslated (or still fuzzy, when those are
translated) is set, and the file is replaced through a temporary file and a
rename. A string someone translated while the batch ran is kept. The editor
shows the new translations of the sheets it shows as they are written.

A run does not commit. The person reviews the changes and commits them like
any other edit.

## Pace and limits

The run sets its own pace. After the first answer it keeps up to 16 requests
in flight. A rate-limit answer halves that, to no fewer than two, and every
finished batch adds one back. A rate limit, network error, or timeout returns
the batch to the queue after the wait the service asked for, or 20 seconds
times the failures in a row; the third in a row stops the run. A response
that sends nothing for 180 seconds, its headers included, is given up, and so
is one not finished in 20 minutes: a reasoning model keeps its stream alive
for as long as it thinks. A connection is given 30 seconds to open and is checked every 20 seconds, so a
connection that died without closing fails its requests instead of holding
the run. Any other refusal leaves the batch's strings for the next run.

A usage-limit answer of the plan stops the run: what was written stays, and
the run reports the time the limit resets when the service says. Starting it
again later continues. A lost sign-in stops the run and asks to sign in
again.

## Progress

The dialog chooses what to translate in a tree of the project's sheets,
with a search and each sheet's and folder's untranslated strings, and with
quick choices: the name sheets, the quests, and every sheet with
untranslated strings. It shows, every second while it is open: strings
written of the
run's strings, strings refused by the checks, tokens used, the share of
prompt tokens served from the cache, the current pace, the requests the
service is answering with their file, strings, retry, and how long each has
waited, a hint about the reasoning depth when an answer takes over five
minutes, the last failure of the service while the run waits, and why the
run stopped. Tokens count as each response answers, so a batch's retries
show as they happen.

Each run writes a journal to `logs/translation-<start>.log` in the data
folder (`%APPDATA%\Aeria` on Windows), and the last 20 are kept: what the
run set out to translate, each request with its file, strings, retry, time,
and tokens, how many strings of each answer fail the checks with their most
frequent problems, each failure and wait, and why the run stopped. The texts
sent and received are not written: a problem is named only up to its first
quote or colon. The run goes on
when the dialog is hidden, and Stop drops the batches in flight at once,
without waiting for the service to answer them.
