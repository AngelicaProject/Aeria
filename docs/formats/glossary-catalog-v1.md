# Glossary catalog format v1

Status: implemented by `aeria-knowledge::catalog` and
`aeria-po::glossary_project`.

`aeria-knowledge/glossary.json` is UTF-8 JSON, at most 64 MiB, with
`formatVersion: 1`. Unknown versions, unknown fields, malformed records,
duplicate identities and source ownership conflicts are errors. There is no
fallback to CSV when a catalog exists but is invalid. Active CSV definitions
alongside a catalog are a conflict, not a second source of glossary data.

The writer uses two-space pretty JSON and LF with a final newline.

## Records

- `categories`: ordered records with a nonempty unique `id`, nonempty unique
  `name` (compared after trimming and ignoring case), and `note` string.
  IDs remain unchanged when names or guidance change.
- `sheets`: ordered declarations with `sheet`, unsigned `column`, and
  `category` referring to a category ID. Sheet and column pairs are unique.
  The category is a default for new names, not a binding on existing terms.
- `terms`: ordered records with unique nonempty `id`, `term` headword,
  category ID in `category`, `forms` array, nullable own `note`, boolean
  `matchCase`, `sources` array, nullable `translation`, and nonempty
  `versionCategory`.

A term's category is independent of its sources. A headword or alias may
belong to only one term in a category, compared after trimming and ignoring
case. A term can retain case variants as forms, because they are spellings
of that same identity. Exact repeated forms are refused. Other categories
can hold other meanings of a name.

A source binding has `sheet`, unsigned `column`, and `source` headword.
It refers to a declared sheet and column and groups source entries ignoring
headword case. Each source binding belongs to exactly one term, even when
several sheets or rows contribute to that term. Bindings survive category
renames and moves.

A standalone term has no sources and a nonempty `translation`. A
sheet-backed term has sources and `translation: null`; its translation is
read from eligible PO entries, using the same majority/tie rules as the
legacy sheet glossary. Fuzzy, untranslated, non-name and explicitly excepted
source entries do not contribute. Missing or temporarily ineligible sources
retain their catalog settings but do not create empty active terms.

`versionCategory` is a stable part of the recorded name fingerprint. For
migrated terms it is the exact former category spelling; for new terms it
is the category ID. Category display names never replace this value.
Translation and own-note fingerprints retain the existing PO contract.

## Editing and duplicate prevention

The editor submits stable term IDs and expected catalog/source versions.
When a newly eligible sheet name matches a standalone identity with exactly
the same translation, discovery attaches that source to the existing term,
preserving its properties and ID. A different translation is a visible
conflict; neither value is silently discarded. New standalone identities are checked against every active or retained term,
including sheet-backed terms and aliases. A duplicate is rejected before
writing; the desktop opens the existing term when it recognizes that input.

Editing a sheet-backed translation validates the new text for every affected
source entry and writes those entries rather than storing an override.
An explicit name edit also fills untranslated matching source rows and
resolves fuzzy matching name rows; explicit term exceptions remain untouched.
Migration alone never fills such rows. Directly edited sources record the
resulting glossary version. Other uses
keep their prior records and become outdated after a real definition change.
Changing a sheet's default category affects only newly discovered names.

Category merges explicitly identify colliding terms and the retained
definition. Sources are combined without creating another term. Different
translations or settings require a choice. A merge of equivalent meanings
rebases only exactly known current name fingerprints when the set of
translations and own notes is unchanged. It does not acknowledge records
that were already outdated or guess whether a translation follows a term.

## Legacy migration

Without a catalog, the documented CSV formats remain authoritative. Before
editing, migration resolves the complete old effective glossary and creates
one catalog record per effective term, in the same order, preserving forms,
case matching, own notes, chosen translations and version categories.
Declared categories become independent category records; their old shared
guidance is collected from declarations. Source bindings include every
contributing legacy name, including names combined across sheets.

Migration first upgrades `aeria.json` to `aeria-po/3`, preserving the original
settings in `legacy-v1/aeria.json`. Older readers refuse the new project
format. New versions continue to read legacy projects until this transition.

Project overrides are transferred into their source entries only where
needed to preserve the old effective translation. Original definitions are
archived under `aeria-knowledge/legacy-v1/terms.csv` and `term-sheets.csv`;
changed source files are archived under `legacy-v1/po/`. These archives are
historical data and never participate in loading or translation. Standalone
terms remain standalone catalog entries. Reviewed source conflicts, invalid
legacy data and unsafe transfers refuse migration before publication.

Migration preserves the recorded versions and does not reset existing
outdated records or ask for a new translation. Running it again reads the
existing catalog. New-project seed CSV files are not recreated after migration.

## Transactions and recovery

Catalog and PO mutations are prepared and checked before publication. Expected
versions cover project settings, the catalog and declared source files, including new files and
source variants that do not change the majority, and the style used by the
translation checks. Ordinary PO saves and glossary publication share the session writer lock.
Migration and glossary writes wait for synchronization/export and refuse an
active AI run, so a run cannot publish records using a retired catalog.
Each modified file also has
an exact expected original text. Concurrent changes are errors.

An application lock in `.aeria/glossary.lock` serializes glossary publication
and recovery. The empty lock file remains on disk; the operating-system lock
is held only for the operation and released when its file handle closes,
including on process exit. File existence does not indicate an active lock.
Before the first mutation, `.aeria/glossary-transaction.json`
records each relative target path, its expected original text or absence,
and its desired text or absence. Only PO paths, project settings, catalog paths and migration
archive paths are valid; absolute paths and traversal components are refused.
The catalog is published last, with individual writes using atomic replacement.

After interruption, recovery checks that each target is still either its
recorded original or desired version, then completes the remaining changes.
A third version is a visible conflict and is never overwritten. The journal
is removed only after completion. Catalog readers recover an outstanding
transaction before resolving source entries. `.aeria` is local runtime state;
the catalog, PO changes and migration archive are project-shared data.

Saving can prepare a result from an earlier immutable snapshot, then verify
every expected content version before writing. The normal writer applies its
already prepared changes directly after publishing the journal; recovery
parses that same journal only after an interruption. A successful save returns
the exact published snapshot, including canonical source entries and their
new recorded versions, without rereading the entire glossary.
