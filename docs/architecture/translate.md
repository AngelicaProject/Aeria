# Machine translation of the project

Status: **proposal, not implemented.**

`aeria translate` translates the untranslated entries of `po/` with a language
model. A program runs the loop: it splits the work, sends requests in
parallel, checks every answer, writes what passes, and continues from where
it stopped. The model gets one small, finished task per request.

## Why

Agent harnesses translated the project through their own subagents. A
subagent given a file of about 800 entries, or a range of one, stopped
partway and reported the work as done; a model decides when its turn ends,
and a long run of similar translations ends it early. Bookkeeping around that
(queues of batches, counts of what is left, re-dispatching) would be a loop
kept by the model, which is what failed.

The game is 859,426 entries, 10.4 million English words. Tools that translate
whole games with language models work the same way: a program sends batches
to a model API, caches each finished line, and resumes; glossaries are
prepared first and only the relevant entries go into a request; answers are
checked automatically and only the failing lines are translated again.
Research on model translation points the same way: retrieved example
translations help more than term lists, and repeated refinement passes change
fluency and style but rarely fix meaning.

Agents keep the work that needs judgment: the project knowledge the machine
translation follows, reading and fixing its results, `fuzzy` entries after a
game update, and text that deserves translation by hand (see
[Agents](#agents)).

## Provider

One provider: a ChatGPT subscription, through the sign-in of the public Codex
client and the Codex backend's Responses API, as open-source agents
do. OpenAI offers no official way for other applications
to use a plan; the method may stop working, and requests count against the
plan's Codex limits. `aeria login` says so.

- `aeria login` starts the device sign-in, prints the verification page and
  the code, and waits. Only the refresh token is stored, in the OS credential
  store; access tokens live in memory and are refreshed shortly before they
  expire, one refresh at a time, so a rotated refresh token is never used
  twice. `aeria logout` deletes it.
- Requests go to `POST https://chatgpt.com/backend-api/codex/responses` with
  `stream: true`, `store: false`, and `originator: aeria`.
- The model and reasoning effort come from `aeria.json` (`translate.model`,
  `translate.effort`), or `--model` and `--effort`; models are listed from
  the backend's catalog (`aeria login --models`).

This is the provider Aeria had until the built-in agent was removed
(`crates/aeria-ai/src/chatgpt.rs` and `responses.rs` before commit
`bd24282`); the sign-in and the request code come back unchanged in a new
crate, `aeria-model`.

## What is translated

`aeria translate [PATH...]` takes files and folders of `po/` (all of `po/`
without paths) and translates every entry whose `msgstr` is empty and that is
not `fuzzy`. With `--fuzzy` it also translates `fuzzy` entries, giving the
model the previous source and the previous translation, and clears the flag
of each one it writes.

There is no job store and no state outside the files. What is left is exactly
the entries still empty, so a second run after an interruption, a usage
limit, or a failed batch continues where the first stopped. `--count` prints
what a run would translate, by file, and stops.

## Batches

Entries are grouped in file order, which keeps a scene's dialogue together and
a sheet's neighbouring rows together:

- a scene file (`quest/`, `cut_scene/`) of up to 150 entries is one batch;
  a longer one is split into runs of about 100;
- any other file is split into runs of about 100 entries.

Each run is one request. A batch never spans files.

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
each entry that is still empty (or still `fuzzy` under `--fuzzy`) is set, and
the file is written to a temporary file and renamed over the original. An
entry someone translated while the batch ran is kept. Files are written with
the canonical format of `aeria-po`, so the diff shows only changed
translations.

`aeria translate` does not commit. The changes are reviewed and committed like
any other edit; `aeria check` sees them as changed files.

## Pace and limits

A run keeps up to 16 requests in flight. A rate-limit answer halves that, to
no fewer than two, and every finished batch adds one back. A network error,
timeout, or rate limit returns the batch to the queue after a wait of 20
seconds times the failures in a row; the third in a row stops the run. A
request with no data for 180 seconds is cancelled and its batch returned.

A usage-limit answer of the plan stops the run: what was written stays, and
the command prints what is left and the time the provider gave for the
limit's reset, if any. Running the command again later continues.

## Output

One line per finished batch (file, entries written, entries rejected), a
line every 30 seconds with entries written, entries left, tokens used, and
the share of prompt tokens served from the cache, and a summary: written,
rejected with reasons, left, tokens. `--json` gives the summary as one JSON
value. Exit status: 0 when nothing is left in the paths, 1 when entries were
rejected or left, 2 on an error.

## Agents

`po/README.md` tells agents that bulk translation is `aeria translate`, never
their own subagents, and what they do instead:

- before a run, make the knowledge the run follows: terms and names, the
  style of the kinds of text involved, the characters and story of the scenes;
- after a run, read the result, fix what is wrong, and record recurring
  problems in `lessons.md` and the knowledge, so the next run follows them;
- translate `fuzzy` entries after a game update, or run `aeria translate
  --fuzzy`;
- translate by hand what deserves it.

A run of a large folder takes longer than a harness lets a command run. An
agent runs it in the background or asks the person to; an interrupted run
loses only the batches in flight.

## Not included

The built-in agent's translation jobs had a planning step, critic and voice
passes, a study of terms before each unit, learning and judged lessons, and a
job database. None of it returns: each added requests per batch for small
gains, and the knowledge files kept by agents replace the learning.
