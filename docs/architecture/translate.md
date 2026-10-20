# Machine translation of the project

Status: **implemented** in `aeria-model` and the desktop (Settings → Machine
translation, Translation → Machine translation).

Machine translation fills the untranslated strings of `po/` with a language
model, and corrects translations a person chooses (see
[Correcting translations](#correcting-translations)). It is a tool a person
runs: a program splits the work, sends requests
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
[`knowledge.md`](./knowledge.md)), reading and fixing the result (a fixed translation is an example for the next
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
writes. A translation a person reviewed is never translated, fuzzy or not
(see [`po-project.md`](./po-project.md#reviews)). Before starting, the dialog shows how many strings the chosen sheets
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

The order of sheets does not matter: what ties a name to its mentions is the
terms, which a run reads once, not what the run wrote before.

## The request

The instructions, identical for every request of a run, come first so the
provider serves them from its prompt cache: the
[translation rules](./knowledge.md#translation-rules) for the target language,
the macro authoring reference of [`strings.md`](./strings.md),
`aeria-knowledge/style.md`, and the form of the answer.

The task is JSON built from the files as they are when the request is sent:

- `terms`: the terms that occur in the sources, of `terms.csv` and of the
  declared sheets, each with its `category`, other wordings, and note (a
  term of `terms.csv` with its category's note before its own; see
  [`../formats/knowledge-v1.md`](../formats/knowledge-v1.md#the-terms-of-the-sheets)), at
  most 200; a name with several meanings is one item
  per meaning, and the model uses the meaning the string means, and none
  where the word of the source is not the thing the category names. A term
  goes only when it applies to a string of the request, not counting the
  strings with an exception for it. A
  term's translation is the name as it stands alone: a sentence writes it
  in the number, case, and capitals it needs, declined like any word where
  the language declines names (Поговорите с Ко Рабнтой), with a capital
  where a sentence or a title begins and as the language writes the name
  elsewhere; only a name the game fills in through a macro keeps its stored
  form. The translations of the
  game's name sheets are not sent as such: a name the project agreed on is a
  term, of `terms.csv` or of a sheet declared part of the glossary
  (see [`knowledge.md`](./knowledge.md#the-terms-are-the-one-glossary));
- `files`: each file of the request on its own, with
  - `about`: the file's header comment: its sheet, a quest's title, and
    whether its strings are in play order;
  - `examples`: translated strings of the same file, the ones a person
    reviewed first, then nearest to the batch
    first: up to 40 shared among the files, at least 5 each;
  - `strings`: each string's ID (unique in the request), source, and `#.`
    lines (the other client languages, speaker or kind, the row's other
    cells, macro legends; the legends are made again from the current macro
    catalog, so what the catalog learned since the file was made reaches the
    model before a game update rewrites the comments), `gendered`: the texts (`source`, `fr`, `de`)
    whose line has a condition on the player character's gender, so the
    translation most likely needs one too, `maxLength` for an interface
    label and `maxBytes` for a name in the world (below), the previous
    source and translation of a fuzzy one, and
    `termExceptions`: the terms of `terms` that do not apply to it, by a
    person's exception (see
    [`po-project.md`](./po-project.md#term-exceptions)), which the
    instructions say to translate by their meaning.

The editor's string guide shows a person the same terms, speaker label,
length, and gender marks of one string; `aeria_model::hints` reads them for
both (see
[`desktop-editor-ui.md`](./desktop-editor-ui.md#string-guide)). This is a
contract ([principle 11](../product/principles.md)): what a request adds
about one string is read through `aeria_model::hints` or the functions it
shares with the request, and shown in the guide in the same change. What
belongs to a whole batch, such as `examples`, or to every request, such as
the instructions, is not part of it.

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
none (`aeria_po::length`; the sheet comes from the `msgctxt`). The request
asks to keep the translation near that length, with the language's usual
abbreviations when needed. It is guidance: the official lengths only
approximate what the layout fits, so a longer label is neither refused nor
advice. Names of items and places get no length: their tooltips show them
whole.

## Names in the world

The game copies the name of a character or object of the world into a
64-byte field ending with NUL (`GameObject_SetName`), and shows it from
there over the object and as a target. A name of 64 bytes or more keeps 63,
and then loses its last character that is not ASCII even when it fits, so a
Cyrillic name is cut to 62 bytes or less. The names are column 0 of the
sheets the game names objects by through `ObjStr`: `Aetheryte`, `BNpcName`,
`Companion`, `ENpcResident`, `EObjName`, `GatheringPointName`, and
`Treasure` (`Mount` is in the `ObjStr` table, but no object kind reaches
it). Such a string gets `maxBytes`, 63: the bytes of its encoded
`SeString`, which for text without macros are its UTF-8 bytes (a Cyrillic
letter takes two). The request asks for a name within it, and a longer one
is refused; for a person it is advice of the checks. `Item`
names are copied too, for housing furniture, but the editor and requests do
not hold them to it: most items are never placed, and their other views show
the name whole.

## Checking and writing

Every translation is checked like a save in the editor
([`po-project.md`](./po-project.md#checking)), after the answer's first
words, and refused for a name longer than the game shows. An answer that is
not valid JSON,
most often for a quote the model did not escape inside macro text, is read
entry by entry (`"id": ["first words", "translation"]`, a bare quote read as
part of its string, the elements separated with or without spaces); what is
read this way is checked like any answer, so a wrong reading is refused,
never written: a translation that holds the answer's separators (`","`,
`",[`) its source does not have is what a reading left of the elements after
it, and is refused. A string the answer has no
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

## Correcting translations

A run can correct translations instead of translating (`Options::fix`): it
takes the chosen strings (`Options::contexts`) that have a translation and
are not reviewed, and sends each translation with what is wrong with it, so
the model changes what is wrong and keeps what is right. A person asks for
any of:

- **proofreading** (`Fix::proofread`): every translation is read as the
  localization's editor reads it, against its source, the other client
  languages of its context, and the strings around it in the scene; it is
  fixed where it loses or adds meaning, misses the form of address the
  style asks for in this line (read from the line of this string in the
  language the style follows, not from its neighbours), the agreement with
  the player character's gender, a name or a term, or reads as a
  translation (constructions copied from the source, officialese, filler).
  The source's meaning comes before smoothness, and what reads well stays
  unchanged. The name of a thing of the game (an action, a status, a menu)
  keeps the form the translation gives it unless the request's names or
  terms give another: many such names, such as those of the interface, are
  not among the request's names, and the game shows them under the name the
  project gave them. Research on refining translations with a model found that one
  general request of this kind improves fluency and style more reliably
  than asking for each kind of error or having the model find errors
  first, and that it can move a translation's meaning, which the reasons
  below and the review in Git are for;
- **the issues of the checks**: what the checks find in the translation,
  problems and advice, every kind or only the kinds chosen (`Fix::groups`,
  by `Issue::group`, as project search groups them). A problem always goes,
  since a corrected translation must pass the checks; a term exception
  that names no term of the source does not, since a person removes it;
- **following changed terms** (`Fix::terms`): the translations written with
  an older version of the chosen terms are brought to the terms as they
  are (see [Following changed terms](#following-changed-terms));
- **adapting** (`Fix::adapt`): a fuzzy translation, whose source changed
  since it was written, is adapted to the new source. The request gives the
  previous source and the translation of it, and the model changes what the
  change of the source changes, keeping every other word, or answers the
  translation unchanged when it still fits. What passes is written with the
  fuzzy mark and the previous source cleared (`EditKind::Adapt`), even when
  it is unchanged, since it is now a translation of the source. A string
  that is not fuzzy is not adapted;
- **a request** in the translator's words, the same for every translation
  of the run, such as a form of address the style changed.

With proofreading or a request every chosen translation is taken; with
adapting, every fuzzy one; with the issues or the terms alone, a string
without any is not. This is the one place advice reaches a model, and only
the advice a person chose.

The request is built as for translating, with the same terms, examples, and
hints of each string, and two more fields:
`translation`, the current translation, and `fix`, the messages of its
issues and the terms it is to follow. The instructions are those of translating with what a correction
may change in place of "translate every string", and the request after the
style; they are the same for every request of the run, so the prompt cache
holds them. The answer has the same form, first words included, and a third
string: why the translation changed, in a few words of the target language
(`обращение по французской строке`, `калька`), empty when it did not. A
reason is not checked and never written to the project.

### Following changed terms

Each translation records the version of each term it was written with (see
[`po-project.md`](./po-project.md#terms-written-with)). When a term
changes, only the translations written with its old version, or before it
was a term, need another look; a run with `Fix::terms` takes exactly those
of its terms (every term when the list is empty) that are not reviewed. Each
string's `fix` names its terms that changed and asks the model to make the
translation follow them as `terms` give them now and change nothing else:
the term is used where the word of the source is the thing it names, as its
translation and note describe it, declined as the sentence needs, and a word
that means something else there stays. Whether the term applies, and how
the sentence takes it, is the model's judgment; no check compares the
translation with the term.

Every answer that passes the checks is written, also one that leaves the
translation as it was: it records that the translation follows the terms as
they are, so the string is not taken again until a term of it changes. A
refused answer leaves the old record, and the next run takes the string
again. The glossary tab starts such a run for one term, the terms chosen, or
every term with strings on an old version (see
[`desktop-editor-ui.md`](./desktop-editor-ui.md#glossary-tab)).

A correction is refused only for what refuses a machine translation: a
problem of the checks, or a name longer than the game shows. Refused
corrections go back with their problems like refused translations, and what
still fails is listed and not written.

A correction is written as a bulk edit of the session
(`EditKind::Correct`): only while the entry still has the translation the
request was built from, and never over a review, so work saved meanwhile and
a review made meanwhile are kept. The fuzzy mark stays. Every string of a
run is counted once: as written when its text changed or an adaptation
cleared its fuzzy mark, as left as it was otherwise, or as refused. The
run's status lists the first 1,000 corrections it wrote, each before and
after with its reason, so a person reads them as an editor's notes before
reading the diff. Like any run, a correction does not commit, and its
changes are reviewed and reverted through Git.

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

A usage-limit answer of the plan does not stop the run: its batch goes back
to the queue, and the run waits until the time the service says the limit
resets, and a minute more, or 15 minutes when it says none, then goes on at
the pace of one request; the other requests in flight that meet the same
limit go back without waiting again. The progress says when the run asks
again, and **Stop** ends the wait. A lost sign-in stops the run and asks to
sign in again.

A run that stopped before it settled every string it planned, by **Stop**,
a failure, or Aeria closed, or one the service refused requests of, can be
continued from its report ([Continuing](#continuing)).

## Progress

The dialog chooses what to translate in a tree of the project's sheets,
with a search and each sheet's and folder's untranslated strings, and with
quick choices: the quests and every sheet with untranslated strings. It shows, every second while it is open: the
strings settled of the run's strings, as the meter shows them, then those
written, those a correction left as they were, and those refused by the
checks; tokens used, the share of
prompt tokens served from the cache, the current pace, the requests the
service is answering with their file, strings, retry, and how long each has
waited, a hint about the reasoning depth when an answer takes over five
minutes, the last failure of the service while the run waits, and why the
run stopped. Tokens count as each response answers, so a batch's retries
show as they happen. **Open report** opens the run's [report](#report).

Each run writes a journal to `logs/translation-<start>.log` in the data
folder (`%APPDATA%\Aeria` on Windows), and the last 20 are kept: what the
run set out to translate, each request with its file, strings, retry, time,
and tokens, how many strings of each answer fail the checks with their most
frequent problems, each failure and wait, and why the run stopped. The texts
sent and received are not written: a problem is named only up to its first
quote or colon. The run goes on
when the dialog is hidden, and Stop drops the batches in flight at once,
without waiting for the service to answer them.

## Report

Each run also writes a report to `logs/translation-<start>.jsonl` beside its
journal, kept and deleted with it: what became of every string the run
asked, so a person finds why a string was refused and an error of the model
or of the checks can be told apart. The report holds the project's texts,
so it stays in the data folder of the computer that ran it, never in the
project or its repository. It is JSON Lines, one object per line, each named
by `record`, written as each string is settled, so a run that stops keeps
what it settled and a report can be read while its run goes:

- `run`, first: `version` (1), the run's `kind` (`translating` or
  `fixing`), `model`, `effort`, the `terms` a correction follows, by
  headword (empty for every term or none), and `startedAt` (Unix
  milliseconds).
- `options`, second: what the run was asked, as `aeria_model::Options`
  (`paths`, `contexts`, `fuzzy`, `model`, `effort`, and `fix` with its
  issues, groups, proofreading, adaptation, request, and terms), and
  `continues`, the id of the report of the run it continues, or null.
- `string`: its `path` and `context`, its `source`, its translation
  `before` the run, each `attempt` with the model's `translation` and the
  `problems` it was refused for, as the issues of the editor, and the
  `outcome`: `written` (with `after` and the model's `reason`), `unchanged`
  (the answer kept the translation), `stale` (the translation changed or was
  reviewed while the run asked, so the answer was not written), or
  `refused` (the last attempt still failed the checks).
- `left`: a request the service refused, with its `paths`, its `strings`,
  and the `error`; what it asked is left for the next run.
- `stop`, last: why the run stopped, as its status says.

A reader skips a record or a field it does not know. A report that cannot be
written does not stop its run. The desktop lists and reads only the reports
of the kinds and version it writes.

### Continuing

Continuing a run (`translation_continue`) starts a run with the options of
its report, leaving out every string its report, and the reports of the runs
it continues, have a `string` record of (`Run::continuing`): what was
written, left unchanged, refused, or changed meanwhile is not asked again,
and a request the service refused is. The new run writes its own report,
whose `continues` names the one it continues, so a run can be continued
again and after Aeria restarts. A report deleted with the oldest journals
ends the chain: the strings only it settled are asked again.
