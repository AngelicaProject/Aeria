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

A run takes a sheet, a folder of sheets (such as `quest/001/`), or the whole
project, and translates every string whose `msgstr` is empty and that is not
fuzzy. On request it also translates fuzzy strings, giving the model the
previous source and translation, and clears the mark of each one it writes.
Before starting, the dialog shows how many strings and files it would
translate.

There is no job store and no state outside the files. What is left is exactly
the strings still untranslated, so a run after a stop, a usage limit, or a
failed batch continues where the last one stopped. One run goes at a time.

## Batches

Strings are grouped in file order, which keeps a scene's dialogue together and
a sheet's neighbouring rows together:

- the untranslated strings of a scene file (`quest/`, `cut_scene/`), when
  there are at most 150, are one batch;
- otherwise a file's untranslated strings are split into batches of 100.

Each batch is one request, and a batch never spans files.

## The request

The instructions, identical for every request of a run, come first so the
provider serves them from its prompt cache: the
[translation rules](./knowledge.md#translation-rules) for the target language,
the macro authoring reference of [`strings.md`](./strings.md),
`aeria-knowledge/style.md`, and the form of the answer.

The task is JSON built from the file as it is when the batch is sent:

- `names`: the game's names that occur in the batch's sources as whole words,
  with their translations from the name sheets of `po/` (`Action`,
  `BNpcName`, `ENpcResident`, `Item`, `PlaceName`, `Quest`, `Status`, and
  others; short, capitalized strings without macros), at most 80;
- `terms`: terms of `terms.csv` that occur in the sources, with their notes
  and forbidden variants, at most 60;
- `examples`: up to 40 translated strings of the same file, nearest to the
  batch first;
- `strings`: each string's short ID, source, and `#.` lines (the other client
  languages, speaker or kind, the row's other cells, macro legends), and the
  previous source and translation of a fuzzy one.

All requests of a run share one `prompt_cache_key`. A probe of this provider
measured that requests with the same key and prefix got 99 % of the prompt
from the cache once one request had stored it, eight parallel requests on a
cold prefix 25 %, and requests each with its own key 13 %; so the first
request of a run goes alone and the others start when it has answered.

The answer is one JSON object from string IDs to translations; the first
object in the answer is read. A string the model leaves out stays for a later
run.

## Checking and writing

Every translation is checked like a save in the editor
([`po-project.md`](./po-project.md#checking)). A batch's failing translations
go back once, in one request, with each problem stated; what fails again stays
untranslated and is listed in the run's status with its problems. Advice does
not reject a translation.

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
that sends nothing for 180 seconds is given up. Any other refusal leaves the
batch's strings for the next run.

A usage-limit answer of the plan stops the run: what was written stays, and
the run reports the time the limit resets when the service says. Starting it
again later continues. A lost sign-in stops the run and asks to sign in
again.

## Progress

The dialog shows, every second while it is open: strings written of the
run's strings, strings refused by the checks, tokens used, the share of
prompt tokens served from the cache, the current pace, the last failure of
the service while the run waits, and why the run stopped. The run goes on
when the dialog is hidden, and Stop drops the batches in flight.
