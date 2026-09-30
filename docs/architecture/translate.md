# Machine translation of the project

Status: **proposal, not implemented.**

Machine translation fills the untranslated entries of `po/` with a language
model. It is a tool a person runs, like the translation tools other projects
use: a program splits the work, sends requests in parallel, checks every
answer, writes what passes, and continues from where it stopped. The model
gets one small, finished task per request. Agents do not run it.

## Why

Agent harnesses translated the project through their own subagents. A
subagent given a file of about 800 entries, or a range of one, stopped
partway and reported the work as done; a model decides when its turn ends,
and a long run of similar translations ends it early. Any bookkeeping around
that (queues of batches, counts of what is left, re-dispatching, a command
agents start and poll) keeps the model in charge of a loop, which is what
failed.

The game is 859,426 entries, 10.4 million English words. Tools that translate
whole games with language models are run by a person: they sign in to a
provider, choose what to translate, start the run, and review the result; the
program sends batches to the model API, caches each finished line, and
resumes. Glossaries are prepared first and only the relevant entries go into a
request; answers are checked automatically and only the failing lines are
translated again. Localization platforms work the same way: machine
pre-translation is started by a person or by automation, and the people after
it review. Research on model translation agrees on the parts that matter:
retrieved example translations help more than term lists, and repeated
refinement passes change fluency and style but rarely fix meaning.

Agents do the work that needs judgment (see [Agents](#agents)).

## Where it runs

The desktop is its home: signing in under Settings, choosing files or folders
of `po/`, starting and stopping a run, and its progress. The desktop does not
open projects of this format yet (see
[`po-project.md`](./po-project.md)); until it does, a person runs the same
code from a terminal with `aeria translate`, which runs in the foreground and
shows its progress on one line. Stopping it with Ctrl+C loses only the batches
in flight.

The translation itself lives in a crate the desktop and the command share,
`aeria-model`.

## Provider

One provider: a ChatGPT subscription, through the sign-in of the public Codex
client and the Codex backend's Responses API, as open-source agents do. OpenAI
offers no official way for other applications to use a plan; the method may
stop working, and requests count against the plan's Codex limits. Signing in
says so.

- Signing in (`aeria login`) starts the device sign-in, shows the verification
  page and the code, and waits. Only the refresh token is stored, in the OS
  credential store; access tokens live in memory and are refreshed shortly
  before they expire, one refresh at a time, so a rotated refresh token is
  never used twice. Signing out (`aeria logout`) deletes it.
- Requests go to `POST https://chatgpt.com/backend-api/codex/responses` with
  `stream: true`, `store: false`, and `originator: aeria`.
- The model and reasoning effort are chosen when a run starts, from the
  backend's catalog, and remembered in `aeria.json` (`translate.model`,
  `translate.effort`).

This is the provider Aeria had until the built-in agent was removed
(`crates/aeria-ai/src/chatgpt.rs` and `responses.rs` before commit
`bd24282`); the sign-in and the request code come back unchanged.

## What is translated

A run takes files and folders of `po/` and translates every entry whose
`msgstr` is empty and that is not `fuzzy`. On request it also translates
`fuzzy` entries, giving the model the previous source and the previous
translation, and clears the flag of each one it writes. Before starting, it
shows how many entries it would translate.

There is no job store and no state outside the files. What is left is exactly
the entries still empty, so a second run after a stop, a usage limit, or a
failed batch continues where the first stopped.

## Batches

Entries are grouped in file order, which keeps a scene's dialogue together and
a sheet's neighbouring rows together:

- a scene file (`quest/`, `cut_scene/`) of up to 150 entries is one batch;
  a longer one is split into runs of about 100;
- any other file is split into runs of about 100 entries.

Each batch is one request. A batch never spans files.

## The request

The instructions, identical for every request of a run, come first so the
provider can serve them from its prompt cache:

1. the rules of `po/README.md` for the target language (macros, line breaks,
   forms);
2. `aeria-knowledge/style.md`;
3. the output contract.

Then the batch's own material:

4. terms of `terms.csv` that occur in the batch's source text;
5. for dialogue, the sections of `characters.md` for its speakers and the
   section of `story.md` for its sheet;
6. up to 40 entries of the same file that are already translated, nearest to
   the batch first, as examples of the file's wording;
7. the batch: for each entry its identifier, source text, and `#.` comments
   (the other client languages, speaker, kind, macro legends).

All requests of a run share one `prompt_cache_key`. A probe of this provider
measured that requests with the same key and prefix got 99 % of the prompt
from the cache once one request had stored it, eight parallel requests on a
cold prefix 25 %, and requests each with its own key 13 %; so the first
request of a run goes alone and the others start when it has answered.

The answer is one JSON object from identifiers to translations. An entry the
model leaves out is left for a later run.

## Checking and writing

Every translation is checked with the rules of `aeria check`
([`po-project.md`](./po-project.md#checking)): the structure of macros,
line breaks, forbidden terms, and the target language's rules. A batch's
failing entries go back once, in one request, with each problem stated; what
fails again stays empty and is listed at the end of the run with the reason.
Advice (a term that may be missing, machine phrasing) does not reject a
translation.

A batch is written as soon as it is checked: the file is read, the `msgstr` of
each entry that is still empty (or still `fuzzy`, when those are translated)
is set, and the file is written to a temporary file and renamed over the
original. An entry someone translated while the batch ran is kept. Files are
written with the canonical format of `aeria-po`, so the diff shows only
changed translations.

A run does not commit. The person reviews the changes, or has agents review
them, and commits them like any other edit.

## Pace and limits

The run sets its own pace; nobody chooses it. It keeps up to 16 requests in
flight. A rate-limit answer halves that, to no fewer than two, and every
finished batch adds one back. A network error, timeout, or rate limit returns
the batch to the queue after a wait of 20 seconds times the failures in a
row; the third in a row stops the run. A request with no data for 180 seconds
is cancelled and its batch returned.

A usage-limit answer of the plan stops the run: what was written stays, and
the run reports what is left and the time the provider gave for the limit's
reset, if any. Starting it again later continues.

## Progress

While a run goes, it shows entries written, left, and rejected, tokens used,
and the share of prompt tokens served from the cache. At the end it lists the
rejected entries with their reasons and how many are left.

## Agents

Agents are not told to run machine translation, and `po/README.md` tells
them not to translate large amounts with their own subagents: bulk
translation is the person's tool. Their work is what needs judgment:

- the knowledge a run follows: terms and names, the style of each kind of
  text, the characters and story of the scenes;
- reading the result of a run, fixing what is wrong, and recording recurring
  problems in `lessons.md` and the knowledge, so the next run follows them;
- `fuzzy` entries after a game update;
- text that deserves translation by hand.

## Not included

The built-in agent's translation jobs had a planning step, critic and voice
passes, a study of terms before each unit, learning and judged lessons, and a
job database. None of it returns: each added requests per batch for small
gains, and the knowledge files kept by agents replace the learning.
