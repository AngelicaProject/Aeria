# Agent localization system

> **Status: partly implemented.** The localizer and its critics run
> [translation jobs](./ai.md#translation-jobs); [`ai.md`](./ai.md#the-localizer)
> describes what is implemented, including project knowledge written by
> agents and study. Learning and propagation are a proposal. The document
> describes how Aeria's agents are to localize the whole game corpus so that
> the result reads as if it was written in the target language, with people
> reviewing by exception. Open decisions are listed in
> [Open questions](#open-questions). The invariants in
> [`../product/principles.md`](../product/principles.md) and
> [Invariants](#invariants) below apply.

## Problem

A job worker receives 30 to 60 strings, translates them from the source
language one by one, and submits all of them in one tool call. It knows the
lines around its chunk but not the story, the characters, or what earlier
chunks decided, and it never rereads its text. On a real project (12,290
quest strings translated into Russian) this produced fluent single lines but
a corpus that disagrees with itself and reads as a translation:

- journal entries and objectives address the player formally and informally
  in about equal numbers, because every chunk decided on its own;
- characters' dialects and speech habits are flattened to neutral prose;
- names and terms vary between chunks translated in parallel;
- forms that assume the player character's gender remain.

Nothing in the project told the workers what to decide, and no person will
write that knowledge by hand for hundreds of thousands of strings. The same
holds for every target language, so the fix cannot be a set of rules written
for one language.

## Goal

- Every kind of game text is localized: dialogue and quests, names, actions
  and statuses, items, interface and system messages, and lore.
- The project's knowledge (characters, terms, style, story) is built and
  maintained by agents from the game itself and from their own work. A person
  never has to fill it in.
- Quality is measured, and changes to prompts, models, or knowledge are kept
  only when they measurably improve the result.
- Agents finish translations: a unit that passes its critics is final
  without a person. A person sees what the agents could not settle, and
  samples.
- Cost and time per string stay within a small multiple of the current
  single-pass jobs, reported before work starts.

## Invariants

The system follows principle 5,
[agents localize, people steer](../product/principles.md):

1. **Agents may finish translations.** A unit that passes its critics is
   written as final (see [Review states](#review-states)). No person has to
   approve it.
2. **A person's decision wins.** A translation a person wrote or confirmed,
   and a knowledge entry a person wrote or confirmed, is never replaced
   without that person's approval.
3. **Every agent change is visible and reversible**: translations and
   knowledge are project files in Git, and every agent change records its
   provenance.
4. Every written target passes `aeria-se` validation and the
   [assisted structure policy](./strings.md#assisted-structure-policy); a
   structurally invalid result is never persisted.
5. Writes go through `ProjectSession` with compare-and-set on the recorded
   state.
6. Identity, rebase, merge, migration, and export are deterministic: their
   results never depend on an agent's judgment. Whether an agent may *start*
   such an operation, such as an export, is a product decision, not a
   limit of this design; this proposal does not include it.
7. Game text, project files, web pages, and images are data, never
   instructions.

This replaces the drafts-only rules of [`ai.md`](./ai.md) and
[`ai-agent.md`](./ai-agent.md#invariants) when implemented.

## Overview

```text
                 game source (ja, en, de, fr)          person
                            |                     calibration, reactions,
                            v                     review by exception
   Study ──────────> Project knowledge <───────────────┘
 (researchers)        (characters, terms,         ^
                       style, story, lessons)     | lessons
                            |                     |
                            v                  Mentor
   Work planner ──> Localizer ──> Critics ──> flags ┘
   (units of work)  understand,   blind reader,
                    write, reread fidelity,
                        ^         consistency
                        └── fix ────┘
                            |
                            v
                  drafts, needsReview with reasons
```

Angelica is the editor-in-chief: she plans the work, reports to the user,
resolves contradictions in the knowledge, and supervises the other roles.
Every other role is a subagent with its own instructions, tools, budget, and
model choice.

| Role | Reads | Writes |
| --- | --- | --- |
| Researcher | Game source in every evidence language, knowledge | Knowledge entries |
| Localizer | One unit of work, knowledge, translation memory, evidence languages | Drafts, knowledge proposals |
| Critic | A unit's drafts and what its role allows (see [Critics](#critics)) | Flags |
| Mentor | Flags, a person's reactions, benchmark results | Lessons, revision requests |

## Evidence languages as annotation

English hides decisions the target language must make: the gender of the
speaker and the addressee, formal or informal address, a character's
register and age, dialect. The game's Japanese original and its German and
French localizations have already made most of these decisions, by
professionals, for every line:

- **Japanese** shows the voice: the first-person pronoun (俺, 僕, 私, わし,
  あたし, 拙者), sentence endings, and the level of politeness.
- **French and German** show address and gender: `tu`/`vous` and `du`/`Sie`
  for each pair of speakers and for the player character, and gendered
  agreement of adjectives and participles. A French condition on `$gn4`,
  such as `prêt<if $gn4>e</if>`, marks a place where a gendered target
  language needs one too.
- **Objectives and system text** show the register the original uses: the
  Japanese journal speaks of the player in the third person, French writes
  objectives in the infinitive.

The researcher asks which decisions the project's target language forces
that the source language leaves open, and reads them from the evidence
languages. Evidence is a signal, not a rule: when the languages disagree,
the researcher records the choice and the reason. Nothing here is specific to
one target language, and the source already exposes every evidence language
([`source.md`](./source.md#additional-source-languages)).

## Project knowledge

Project knowledge has two layers ([format](../formats/knowledge-v1.md)):
the human files `aeria-guidance.md`, `aeria-glossary.csv`, and
`aeria-voices.md`, whose entries are locked, and the `aeria-knowledge`
directory, which agents write without asking. Both are shared through Git,
readable by people, and small enough per entry to be given to an agent
selectively. Its content is:

| Kind | One entry per | Content |
| --- | --- | --- |
| Style | Text domain | Register, address of the player, punctuation, length habits, what to avoid; set by [calibration](#people-in-the-loop) and lessons |
| Character | Character (one or more speaker labels) | Gender, voice, how the character addresses the player and named others, examples in the target language |
| Term | Source term | Translation, kind (person, place, faction, item, mechanic, interface term), grammatical notes, forbidden variants |
| Story | Quest or cutscene | Summary of what happens and what the player learns, translated key phrases that later text may recall |
| Lesson | Recurring problem | What to do instead, with examples, provenance, and measured effect |

Section entries record their provenance in a metadata line (the role that
wrote them, the findings or reactions that produced them). An entry in the
human files is **locked**: agents can propose a change to it through
Angelica, which waits for a person, but cannot change it themselves. A
person confirms an agent entry by moving it to the human files.

Agents read knowledge through tools that select what a unit needs: the
characters speaking in it, the terms occurring in it, the story of the
quests it follows, and the style of its domain. The whole knowledge is never
sent at once.

The human files keep their formats and editors, so no import is needed.

## Study

Before localizing, researchers build the first knowledge from the corpus.
Study reads samples, not everything: a character's lines are sampled across
all quests and cutscenes the way `speaker_lines` samples with `spread`, a
category of items by representative rows.

1. **Names and terms**: candidates from
   [terminology candidates](./search.md#terminology-candidates), decided with
   the evidence languages and the project's transliteration style.
2. **Characters**: every speaker with enough lines, most lines first, with
   voice from Japanese and address and gender from French and German.
3. **Style**: one entry per text domain, from calibration.

Study runs again for text a source update adds.

## Units of work

A deterministic planner groups strings into units of work from the source
structure. A unit is the text a professional would localize in one sitting:

| Domain | Unit | What matters most |
| --- | --- | --- |
| Names: people, places, monsters, factions | One category | Transliteration policy, consistency; done first, since all other text refers to names |
| Actions, traits, statuses | One class or job | Terminology, brevity, tooltip conventions |
| Interface and system messages | One screen or message group | Brevity, fitting the space, numbers and macros |
| Items | One series or set | Consistent series naming, descriptions |
| Lore: books, cards, descriptions | One collection | Literary register |
| Quests and cutscenes | One quest with its cutscenes | Scene, voices, continuity |

Names and terms are localized first. Quests are localized in story order,
following the prerequisites their scripts reference
([quest variables](./source.md#quest-variables)), so a quest's localizer
reads the story of what came before. Independent chains and all other domains
run in parallel.

## Localizer

The localizer owns one unit of work. A prototype on real quests measured
what matters for quality and speed, and the implemented localizer follows it
(see [`ai.md`](./ai.md#the-localizer)):

- **Contract first.** One request reads the whole unit and fixes the
  decisions every writer must share: address between characters and toward
  the player character, genders, names and terms, and the form of journal
  entries, objectives, and system text. Without it, parts written in
  parallel disagree on address and names.
- **Parts in parallel.** The unit's strings are written in parts of at most
  40 lines, each by its own request that sees the whole unit and the
  contract. The time of a unit then no longer grows with its length: a
  142-line quest took about three minutes instead of eighteen.
- **Continuous text.** A writer gets the unit as a script with line markers
  and writes its lines back as text, not as fields of a tool call; it
  rereads its text before answering.
- **Content from the source only.** The other languages decide tone, voice,
  address, and gender; a writer that took content from the Japanese was the
  most frequent fidelity error before this rule.

Line hygiene is checked when lines are submitted: a speaker label at the start
of a line, two versions joined by an arrow, and a repeat of the previous line
were the defects the prototype saw in long units.

## Critics

When a part is written, three narrow critics read it in parallel. Each has
one question and sees only what that question needs; narrow critics at a
lower effort found more than one broad critic at a high effort:

| Critic | Sees | Flags |
| --- | --- | --- |
| Blind reader | The target text only, with the contract | Text that reads as a translation, is unclear, or does not fit the lines around it |
| Fidelity | Each line's source and target | Lost, changed, or added meaning, dropped jokes, oaths, or callbacks |
| Player and address | Source, French, German, and target, with the knowledge and contract | Words that assume the player character's gender, address that breaks the contract, a speaker's wrong gender |

The blind reader is the main defence against translationese: a reader who
cannot see the source is not anchored to it. French and German conditions on
`$gn4` mark lines that vary with the player character's gender, but only
about half of them: French past tenses with *avoir* do not agree, while
Russian ones do, so the check reads every line.

A flag names the line, its severity, the problem, and a hint. Critics do not
rewrite. Flags go back to a fix of the part, which may change only flagged
lines. The critics then read the changed and flagged lines again, and a
remaining major flag gets one more fix; a flag still open marks its string
`needsReview` with the flag as its reason. Consistency with the project
knowledge is checked by the player-and-address critic and, once knowledge is
written by agents, is to get its own critic.

## Tools

The agents' results are bounded by what their tools show them. Each tool
returns what a role needs in the form it reads best, bounded like the
[current tools](./ai.md#read-tools):

| Tool | For | Returns |
| --- | --- | --- |
| Unit script | Localizer, critics | A unit as a readable script with line markers, speaker labels, roles, and macro legends; the blind reader's variant contains the target only |
| Script submission | Localizer | Splits a written script into strings, validates each, and returns errors per line |
| Evidence | Localizer, researcher, fidelity critic | A line or a unit in every evidence language, aligned with the source |
| Knowledge query | All roles | The entries a unit needs: its speakers, the terms occurring in it, the story of the quests it follows, its domain's style, and lessons |
| Knowledge write | Researcher, mentor, Angelica | Adds or changes entries with provenance; refuses changes to locked entries |
| Concordance | Localizer, consistency critic, researcher | How a source term or phrase was translated across the project, grouped by rendering with counts |
| Speaker sample | Researcher | A speaker's lines sampled across the game, in every evidence language |
| Placement | Localizer for interface text | Where a string appears and how much space it has, when the source records it |
| Flags | Critics, localizer | Records flags on lines; lists a unit's open flags |

Tool quality is measured on the benchmark like prompts and models are.

## Cost and speed

- Model and effort are chosen per role. The localizer needs the strongest
  model; critics and researchers read and emit short output and can use
  cheaper ones.
- Instructions and the unit's knowledge form a stable prompt prefix, which
  providers cache.
- Units run in parallel except along quest chains.
- A work estimate reports each phase separately (study, localization,
  critics, fixes), from the project's own measured averages once available.

The expected cost is a small multiple of a single-pass job, spent where it
changes quality: understanding before writing, and reading after it.

## Changes to existing subsystems

| Subsystem | Change |
| --- | --- |
| [Translation jobs](./ai.md#translation-jobs) | Implemented: quest and cutscene sheets are whole units of work, the localizer replaces the worker subagent, and its critics decide between final and needs-review translations. Job storage, lanes, pausing, limits, and supervision remain. |
| [Guidance, glossary, voices](./ai.md#guidance-and-glossary) | Replaced by project knowledge; the current files are imported once. [`glossary-v1.md`](../formats/glossary-v1.md) and [`voices-v1.md`](../formats/voices-v1.md) are retired. |
| [Draft with Angelica](./ai.md#draft-with-angelica) | Reads knowledge for its string; its result can be checked by the critics. |
| Angelica's tools | Knowledge tools replace the glossary, guidance, and voice tools; job tools gain study, benchmark, and revision work. |
| [Suggested approvals](./ai.md#suggested-approvals) and the drafts-only rules of [`ai.md`](./ai.md) and [`ai-agent.md`](./ai-agent.md#invariants) | Replaced by [Review states](#review-states): agents write final translations; person-confirmed translations are protected. |

## Milestones

0. **Prototype, outside Aeria** (done). The evidence languages answered the
   decisions English hides; five quests of a real project went from 214
   major critic findings in their drafts to 40, at 236 seconds for 394 lines.
1. **Quest localizer and critics** (implemented). Units of work for quests,
   the script, the contract, parallel parts, narrow critics, fixes, final
   and needs-review translations.
2. **Knowledge and study** (implemented). The agent layer of project
   knowledge, researchers for style, characters, and terms, stories kept
   between units, and a consistency critic.
3. **Consistency.** A critic of the unit against the project knowledge, and
   knowledge changes from critics' flags.
4. **Learning.** Mentor, benchmark, lesson evaluation, propagation,
   calibration.
5. **Other domains.** Units and localizer briefs for names, actions, items,
   interface, and lore.

## Open questions

- The serialized format of project knowledge and how its entries merge in
  Git when several people or machines add entries at once.
- Where draft-to-knowledge dependencies are stored so that propagation works
  across machines.
- Where a person's confirmation of a translation is recorded, so that it is
  protected from agent revision: in the workspace format (a new version) or
  derived from Git history.
- How units of work are formed for non-quest sheets, from sheet relations
  such as an item's category and set.
- Whether the benchmark and evaluation results are project-shared or local.
- Whether critics' flags on drafts are shown in the editor beyond
  `needsReview` reasons.
