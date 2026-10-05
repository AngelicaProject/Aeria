//! Carrying translations to the files made again for the game, by
//! `msgctxt`: the one rule of a game update.

use std::collections::{BTreeMap, BTreeSet};

use crate::identity::Identity;
use crate::po::{Entry, Header, PoFile};

/// Carries the translation of `before`, the entry with the same `msgctxt`
/// in the previous files, to `entry`, made from the game with no
/// translation:
///
/// | `before` | result |
/// | --- | --- |
/// | same `msgid` | translation, notes, and marks kept |
/// | other `msgid` | translation, notes, and review mark kept; fuzzy, with the source the translation was written for as `#\| msgid`; term exceptions dropped |
///
/// A reviewed translation stays reviewed when its source changes: it is a
/// person's, so machine translation leaves it, and the fuzzy mark asks the
/// person to look at it again.
fn carry(before: &Entry, entry: &mut Entry) {
    entry.translation.clone_from(&before.translation);
    entry.notes.clone_from(&before.notes);
    entry.reviewed.clone_from(&before.reviewed);
    if before.source == entry.source {
        entry.fuzzy = before.fuzzy;
        entry.previous.clone_from(&before.previous);
        entry.term_exceptions.clone_from(&before.term_exceptions);
    } else if !before.translation.is_empty() {
        entry.fuzzy = true;
        // The source the translation was written for: the previous source,
        // unless that entry was already waiting for a change of its own.
        entry.previous = Some(if before.fuzzy {
            before
                .previous
                .clone()
                .unwrap_or_else(|| before.source.clone())
        } else {
            before.source.clone()
        });
    }
}

/// An entry the game no longer has, kept for its translation or notes.
fn obsolete(entry: &Entry) -> Option<Entry> {
    (!entry.translation.is_empty() || !entry.notes.is_empty()).then(|| Entry {
        extracted: Vec::new(),
        line: 0,
        ..entry.clone()
    })
}

/// One file carried to its file made again: see [`merge_files`].
#[must_use]
pub fn merge(previous: Option<&PoFile>, fresh: PoFile) -> PoFile {
    let mut files = merge_files(
        &previous
            .map(|file| BTreeMap::from([(String::new(), file.clone())]))
            .unwrap_or_default(),
        BTreeMap::from([(String::new(), fresh)]),
        "",
    );
    files.remove("").unwrap_or_default()
}

/// Every file of a project, by its path relative to `po/`, carried to the
/// files `fresh` made from the game for `version`, entry by entry across all
/// files, by `msgctxt`: a sheet whose rows move to other files, or whose
/// file is split or joined, keeps every translation.
///
/// An entry of `previous` whose `msgctxt` the game no longer has becomes
/// obsolete (`#~`) when it has a translation or notes: in the file of its
/// previous path when `fresh` has that path, else in the first file of its
/// sheet, else in a file of its previous path of its own. Obsolete entries
/// of `previous` take part like the others, so a string the game brings back
/// gets its translation back. Carrying the result to the same `fresh` again
/// changes nothing.
#[must_use]
pub fn merge_files(
    previous: &BTreeMap<String, PoFile>,
    mut fresh: BTreeMap<String, PoFile>,
    version: &str,
) -> BTreeMap<String, PoFile> {
    let old: BTreeMap<&str, &Entry> = previous
        .values()
        .flat_map(|file| file.obsolete.iter().chain(&file.entries))
        .map(|entry| (entry.context.as_str(), entry))
        .collect();
    let mut joined: BTreeSet<&str> = BTreeSet::new();
    let mut first_file: BTreeMap<String, String> = BTreeMap::new();
    for (path, file) in &mut fresh {
        for entry in &mut file.entries {
            if let Ok(identity) = Identity::parse(&entry.context) {
                first_file
                    .entry(identity.sheet)
                    .or_insert_with(|| path.clone());
            }
            if let Some((context, before)) = old.get_key_value(entry.context.as_str()) {
                joined.insert(context);
                carry(before, entry);
            }
        }
        file.obsolete.clear();
    }
    for (path, file) in previous {
        for entry in file.entries.iter().chain(&file.obsolete) {
            if joined.contains(entry.context.as_str()) {
                continue;
            }
            let Some(kept) = obsolete(entry) else {
                continue;
            };
            let home = if fresh.contains_key(path) {
                path.clone()
            } else {
                Identity::parse(&entry.context)
                    .ok()
                    .and_then(|identity| first_file.get(&identity.sheet).cloned())
                    .unwrap_or_else(|| path.clone())
            };
            fresh
                .entry(home)
                .or_insert_with(|| PoFile {
                    header: moved_header(&file.header, version),
                    ..PoFile::default()
                })
                .obsolete
                .push(kept);
        }
    }
    for file in fresh.values_mut() {
        file.obsolete.sort_by(|a, b| a.context.cmp(&b.context));
        file.obsolete.dedup_by(|a, b| a.context == b.context);
    }
    fresh
}

/// A previous file's header for `version`.
fn moved_header(header: &Header, version: &str) -> Header {
    Header {
        comments: header.comments.clone(),
        fields: header
            .fields
            .iter()
            .map(|(name, value)| {
                let value = if name == "X-Game-Version" {
                    version.to_owned()
                } else {
                    value.clone()
                };
                (name.clone(), value)
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(context: &str, source: &str, translation: &str) -> Entry {
        Entry {
            context: context.to_owned(),
            source: source.to_owned(),
            translation: translation.to_owned(),
            ..Entry::default()
        }
    }

    fn file(entries: Vec<Entry>) -> PoFile {
        PoFile {
            entries,
            ..PoFile::default()
        }
    }

    #[test]
    fn translations_follow_their_identity() {
        let previous = file(vec![
            {
                let mut same = entry("S:1:0:0", "Hello", "Привет");
                same.notes = vec!["note".to_owned()];
                same
            },
            entry("S:2:0:0", "Old text", "Старый"),
            entry("S:3:0:0", "Gone", "Ушло"),
            entry("S:4:0:0", "Untranslated gone", ""),
        ]);
        let fresh = file(vec![
            entry("S:1:0:0", "Hello", ""),
            entry("S:2:0:0", "New text", ""),
            entry("S:5:0:0", "Added", ""),
        ]);
        let merged = merge(Some(&previous), fresh.clone());
        assert_eq!(merged.entries[0].translation, "Привет");
        assert_eq!(merged.entries[0].notes, ["note"]);
        assert!(!merged.entries[0].fuzzy);
        assert_eq!(merged.entries[1].translation, "Старый");
        assert!(merged.entries[1].fuzzy);
        assert_eq!(merged.entries[1].previous.as_deref(), Some("Old text"));
        assert_eq!(merged.entries[2].translation, "");
        assert_eq!(
            merged
                .obsolete
                .iter()
                .map(|entry| entry.context.as_str())
                .collect::<Vec<_>>(),
            ["S:3:0:0"]
        );

        // Merging again changes nothing.
        assert_eq!(merge(Some(&merged), fresh.clone()), merged);

        // The string comes back in a later version with its translation.
        let back = merge(
            Some(&merged),
            file(vec![
                entry("S:3:0:0", "Gone", ""),
                entry("S:1:0:0", "Hello", ""),
            ]),
        );
        assert_eq!(back.entries[0].translation, "Ушло");
        assert!(!back.entries[0].fuzzy);
    }

    #[test]
    fn a_fuzzy_translation_keeps_the_source_it_was_written_for() {
        let first = merge(
            Some(&file(vec![entry("S:1:0:0", "One", "Один")])),
            file(vec![entry("S:1:0:0", "Two", "")]),
        );
        let second = merge(Some(&first), file(vec![entry("S:1:0:0", "Three", "")]));
        assert!(second.entries[0].fuzzy);
        assert_eq!(second.entries[0].previous.as_deref(), Some("One"));
        // An untranslated string whose source changed is not fuzzy.
        let empty = merge(
            Some(&file(vec![entry("S:1:0:0", "One", "")])),
            file(vec![entry("S:1:0:0", "Two", "")]),
        );
        assert!(!empty.entries[0].fuzzy);
    }

    #[test]
    fn term_exceptions_last_while_the_source_does() {
        let mut excepted = entry("S:1:0:0", "Maelstrom of Despair", "Вихрь отчаяния");
        excepted.term_exceptions = vec!["Maelstrom".to_owned()];
        let previous = file(vec![excepted]);
        let same = merge(
            Some(&previous),
            file(vec![entry("S:1:0:0", "Maelstrom of Despair", "")]),
        );
        assert_eq!(same.entries[0].term_exceptions, ["Maelstrom"]);
        let changed = merge(
            Some(&previous),
            file(vec![entry("S:1:0:0", "Maelstrom of Hope", "")]),
        );
        assert!(changed.entries[0].fuzzy);
        assert!(changed.entries[0].term_exceptions.is_empty());
    }

    #[test]
    fn a_review_stays_with_its_translation() {
        let mut reviewed = entry("S:1:0:0", "Walk", "Прогулка");
        reviewed.set_reviewed(true);
        let previous = file(vec![reviewed]);
        let changed = merge(Some(&previous), file(vec![entry("S:1:0:0", "A Walk", "")]));
        assert!(changed.entries[0].fuzzy);
        assert!(
            changed.entries[0].is_reviewed(),
            "a person's translation stays theirs while they look at it again"
        );
    }
}
