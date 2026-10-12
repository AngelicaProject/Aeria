//! The terms of the sheets a project declares part of its glossary
//! (`aeria-knowledge/term-sheets.csv`): each name of a declared sheet is a
//! term with the translation its strings have most often, after the terms
//! of `terms.csv` (see `docs/formats/knowledge-v1.md`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aeria_knowledge::{GlossaryEntry, Knowledge, TermSheet};

use crate::identity::Identity;
use crate::po::PoFile;
use crate::project::{PO_DIR, list};

/// A term of the declared sheets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SheetTerm {
    /// The sheet of its first string.
    pub sheet: String,
    /// The `msgctxt` of its first string.
    pub context: String,
    pub term: String,
    pub translation: String,
    pub category: String,
    /// The other spellings of the name in the sheets.
    pub forms: Vec<String>,
    /// How many strings of the sheets have the name and count for it.
    pub strings: usize,
    /// Each translation the strings that count have, with how many have
    /// it: the term's own first, then the others, the most frequent first.
    pub variants: Vec<Variant>,
}

/// A translation of a name among the strings of the sheets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Variant {
    pub translation: String,
    pub strings: usize,
}

/// A term of the declared sheets that a term of `terms.csv` takes the
/// place of.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplacedTerm {
    /// The term of `terms.csv`, by its place among the file's entries.
    pub entry: usize,
    /// The term the sheets would give.
    pub term: SheetTerm,
}

/// A project's knowledge with the terms of its declared sheets.
#[derive(Clone, Debug, Default)]
pub struct ProjectKnowledge {
    pub knowledge: Knowledge,
    /// The terms of the declared sheets, in the order the glossary has them.
    pub sheet_terms: Vec<SheetTerm>,
    /// The headwords of terms of `terms.csv` that say no more than the
    /// sheets would: the same translation, no note, and no other forms than
    /// the sheets' spellings of the name. Taking them out of the file
    /// changes nothing.
    pub redundant: Vec<String>,
    /// The terms of the sheets the terms of `terms.csv` take the place of,
    /// in the order the sheets give them.
    pub replaced: Vec<ReplacedTerm>,
    /// The files and folders of `po/` the terms were read from: the terms
    /// are read again when one of them changes.
    pub read: Vec<PathBuf>,
}

/// Whether `path`, relative to `po/`, is a file of `sheet`: the sheet's file,
/// or a file of its folder when it is split by row. A `~` a path adds to a
/// name (see [`crate::identity::SheetPaths`]) is not part of the name.
fn is_file_of(path: &str, sheet: &str) -> bool {
    let Some(stem) = path.strip_suffix(".po") else {
        return false;
    };
    let parts: Vec<&str> = stem
        .split('/')
        .map(|part| part.split('~').next().unwrap_or(part))
        .collect();
    let wanted: Vec<&str> = sheet.split('/').collect();
    parts == wanted
        || parts.split_last().is_some_and(|(file, folder)| {
            folder == wanted && !file.is_empty() && file.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// A string of a declared sheet that names a term.
struct Named {
    sheet: usize,
    context: String,
    source: String,
    translation: String,
    notes: Vec<String>,
    exceptions: Vec<String>,
}

/// The strings of `sheet`'s column that can name a term, in file and entry
/// order: translated, not fuzzy, and plain text, since a term is found in
/// the text of a string.
fn named_strings(
    root: &Path,
    files: &[String],
    index: usize,
    sheet: &TermSheet,
    read: &mut Vec<PathBuf>,
) -> Vec<Named> {
    let mut found = Vec::new();
    for path in files.iter().filter(|path| is_file_of(path, &sheet.sheet)) {
        let full = root.join(PO_DIR).join(path);
        if let Some(folder) = full.parent()
            && !read.iter().any(|seen| seen == folder)
        {
            read.push(folder.to_owned());
        }
        read.push(full.clone());
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        for entry in PoFile::parse(&text).0.entries {
            let named = Identity::parse(&entry.context).is_ok_and(|identity| {
                identity.sheet == sheet.sheet && identity.column == sheet.column
            });
            let source = entry.source.trim();
            let translation = entry.translation.trim();
            // A name has a letter: `1` or `???` names nothing.
            if !named
                || entry.fuzzy
                || translation.is_empty()
                || !source.chars().any(char::is_alphabetic)
                || source.contains(['<', '\n'])
            {
                continue;
            }
            found.push(Named {
                sheet: index,
                context: entry.context.clone(),
                source: source.to_owned(),
                translation: translation.to_owned(),
                notes: entry.notes.clone(),
                exceptions: entry.term_exceptions.clone(),
            });
        }
    }
    found
}

/// The term of one name of the sheets of one category, from its strings in
/// order: the translation most of them have, the first of those as frequent.
/// A string with an exception for the name means another thing and does not
/// count. `None` when no string counts.
fn term_of(strings: &[&Named], sheets: &[TermSheet]) -> Option<(GlossaryEntry, SheetTerm)> {
    let name = strings.first()?.source.to_lowercase();
    let counted: Vec<&&Named> = strings
        .iter()
        .filter(|string| {
            !string
                .exceptions
                .iter()
                .any(|exception| exception.to_lowercase() == name)
        })
        .collect();
    let first = counted.first()?;
    let mut tally: Vec<(&str, usize)> = Vec::new();
    for string in &counted {
        match tally
            .iter_mut()
            .find(|(known, _)| *known == string.translation)
        {
            Some((_, count)) => *count += 1,
            None => tally.push((&string.translation, 1)),
        }
    }
    // The most frequent first; equals keep the order they appear in, so the
    // first of them is the term's.
    tally.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let translation = tally.first()?.0;
    let mut forms: Vec<String> = Vec::new();
    for string in &counted {
        if string.source != first.source && !forms.contains(&string.source) {
            forms.push(string.source.clone());
        }
    }
    // The sheets' notes say what their names are; the translator notes of
    // the strings say more of this one.
    let mut notes: Vec<&str> = Vec::new();
    for string in &counted {
        let sheet_note = sheets[string.sheet].note.as_deref();
        for note in sheet_note
            .into_iter()
            .chain(string.notes.iter().map(String::as_str))
        {
            let note = note.trim();
            if !note.is_empty() && !notes.contains(&note) {
                notes.push(note);
            }
        }
    }
    let sheet = &sheets[first.sheet];
    let entry = GlossaryEntry {
        term: first.source.clone(),
        translation: translation.to_owned(),
        forms: forms.clone(),
        note: (!notes.is_empty()).then(|| notes.join("\n")),
        // A name is found as the game writes it: `In the Mist` is not the
        // words of a sentence.
        match_case: true,
        category: sheet.category.clone(),
    };
    let term = SheetTerm {
        sheet: sheet.sheet.clone(),
        context: first.context.clone(),
        term: entry.term.clone(),
        translation: entry.translation.clone(),
        category: entry.category.clone(),
        forms,
        strings: counted.len(),
        variants: tally
            .iter()
            .map(|(translation, strings)| Variant {
                translation: (*translation).to_owned(),
                strings: *strings,
            })
            .collect(),
    };
    Some((entry, term))
}

/// Whether `entry` of `terms.csv` takes the place of the sheets' term of
/// `name` (lowercase) in `category`: it has the name and the category,
/// ignoring case.
fn replaces(entry: &GlossaryEntry, name: &str, category: &str) -> bool {
    entry.category.to_lowercase() == category.to_lowercase()
        && entry.all_forms().any(|form| form.to_lowercase() == name)
}

/// Whether `entry` of `terms.csv` says no more than the sheets' `term` of
/// its name: the same translation, no note, and no other forms than the
/// sheets' spellings of the name.
fn says_no_more(entry: &GlossaryEntry, term: &GlossaryEntry) -> bool {
    let spellings: Vec<String> = term.all_forms().map(str::to_lowercase).collect();
    entry.translation == term.translation
        && entry.note.is_none()
        && entry
            .all_forms()
            .all(|form| spellings.contains(&form.to_lowercase()))
}

/// Reads the knowledge of the project at `root` and adds the terms of the
/// sheets it declares: one per name and category, after the terms of
/// `terms.csv`, in the order their names first appear, but for the names a
/// term of `terms.csv` already gives.
#[must_use]
pub fn project_knowledge(root: &Path) -> ProjectKnowledge {
    let mut knowledge = Knowledge::load(root);
    let mut loaded = ProjectKnowledge::default();
    if knowledge.sheets.is_empty() {
        loaded.knowledge = knowledge;
        return loaded;
    }
    let files = match list(root) {
        Ok(files) => files,
        Err(error) => {
            knowledge.problems.push(error.to_string());
            Vec::new()
        }
    };
    let sheets = knowledge.sheets.clone();
    let mut strings: Vec<Named> = Vec::new();
    for (index, sheet) in sheets.iter().enumerate() {
        strings.extend(named_strings(root, &files, index, sheet, &mut loaded.read));
    }
    // The strings of each name and category, in the order names appear.
    let key = |string: &Named| {
        (
            string.source.to_lowercase(),
            sheets[string.sheet].category.to_lowercase(),
        )
    };
    // Each key's group, by its place in `names`: a sheet has tens of
    // thousands of names.
    let mut names: Vec<((String, String), Vec<&Named>)> = Vec::new();
    let mut places: HashMap<(String, String), usize> = HashMap::new();
    for string in &strings {
        let wanted = key(string);
        if let Some(place) = places.get(&wanted) {
            names[*place].1.push(string);
        } else {
            places.insert(wanted.clone(), names.len());
            names.push((wanted, vec![string]));
        }
    }
    // The terms of the file by each lowercase form.
    let mut owned: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, own) in knowledge.terms.entries.iter().enumerate() {
        for form in own.all_forms() {
            owned.entry(form.to_lowercase()).or_default().push(index);
        }
    }
    let mut added = Vec::new();
    // The lowercase translations each name has among the terms added.
    let mut translated: HashMap<String, Vec<String>> = HashMap::new();
    for ((name, _), group) in &names {
        let Some((entry, term)) = term_of(group, &sheets) else {
            continue;
        };
        let own = owned
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .find(|index| replaces(&knowledge.terms.entries[*index], name, &entry.category));
        if let Some(index) = own {
            let own = &knowledge.terms.entries[index];
            if says_no_more(own, &entry) && !loaded.redundant.contains(&own.term) {
                loaded.redundant.push(own.term.clone());
            }
            loaded.replaced.push(ReplacedTerm { entry: index, term });
            continue;
        }
        // A name translated alike in another category is the same meaning:
        // a character and the same character as a foe.
        let translation = entry.translation.to_lowercase();
        let known = translated.entry(name.clone()).or_default();
        if known.contains(&translation) {
            continue;
        }
        known.push(translation);
        added.push(entry);
        loaded.sheet_terms.push(term);
    }
    knowledge.terms.append(added);
    loaded.knowledge = knowledge;
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_is_found_by_its_files() {
        assert!(is_file_of("Quest~/65000.po", "Quest"));
        assert!(is_file_of("PlaceName.po", "PlaceName"));
        assert!(is_file_of("PlaceName/0.po", "PlaceName"));
        assert!(is_file_of("custom/000/Names~2.po", "custom/000/Names"));
        assert!(!is_file_of("quest/000/A.po", "Quest"));
        assert!(!is_file_of("QuestRedo.po", "Quest"));
        assert!(!is_file_of("Quest~/65000.txt", "Quest"));
    }

    fn write(root: &Path, path: &str, text: &str) {
        let full = root.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("folder");
        std::fs::write(full, text).expect("write");
    }

    fn entry(context: &str, source: &str, translation: &str) -> String {
        format!("msgctxt \"{context}\"\nmsgid \"{source}\"\nmsgstr \"{translation}\"\n\n")
    }

    #[test]
    fn a_name_of_a_sheet_is_a_term_with_its_most_frequent_translation() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        write(
            root,
            "aeria-knowledge/term-sheets.csv",
            "sheet,column,category,note\nENpcResident,0,персонаж,Имена.\n",
        );
        write(
            root,
            "po/ENpcResident/0.po",
            &[
                entry(
                    "ENpcResident:1:0:0",
                    "Faire Crier",
                    "Зазывала Праздника огня",
                ),
                "# Зазывала у ворот.\n".to_owned()
                    + &entry("ENpcResident:2:0:0", "faire crier", "зазывала праздника"),
                entry("ENpcResident:3:0:0", "faire crier", "зазывала праздника"),
                "#, aeria-term-exception: faire crier\n".to_owned()
                    + &entry(
                        "ENpcResident:4:0:0",
                        "faire crier",
                        "зазывала Летнего фестиваля",
                    ),
                "#, aeria-term-exception: faire crier\n".to_owned()
                    + &entry(
                        "ENpcResident:5:0:0",
                        "faire crier",
                        "зазывала Летнего фестиваля",
                    ),
                entry("ENpcResident:6:0:0", "Thancred", "Танкред"),
                entry("ENpcResident:7:0:0", "Thancred", "Тэнкред"),
                entry("ENpcResident:8:0:0", "In the Mist", ""),
                entry("ENpcResident:9:0:0", "The <i>Scions</i>", "Потомки"),
                "#, fuzzy\n".to_owned() + &entry("ENpcResident:10:0:0", "Old", "Старое"),
            ]
            .concat(),
        );
        let loaded = project_knowledge(root);
        assert!(
            loaded.knowledge.problems.is_empty(),
            "{:?}",
            loaded.knowledge.problems
        );
        let terms: Vec<(&str, &str, &[String], usize)> = loaded
            .sheet_terms
            .iter()
            .map(|term| {
                (
                    term.term.as_str(),
                    term.translation.as_str(),
                    term.forms.as_slice(),
                    term.strings,
                )
            })
            .collect();
        // The most frequent translation wins, the first of equals; the
        // strings excepted from the name do not count; another spelling is
        // a form; untranslated, macros, and fuzzy name nothing.
        assert_eq!(
            terms,
            [
                (
                    "Faire Crier",
                    "зазывала праздника",
                    &["faire crier".to_owned()][..],
                    3
                ),
                ("Thancred", "Танкред", &[][..], 2),
            ]
        );
        // Every translation of a name is a variant, the term's first.
        let variants: Vec<(&str, usize)> = loaded.sheet_terms[1]
            .variants
            .iter()
            .map(|variant| (variant.translation.as_str(), variant.strings))
            .collect();
        assert_eq!(variants, [("Танкред", 1), ("Тэнкред", 1)]);
        assert_eq!(loaded.sheet_terms[0].variants.len(), 2);
        assert_eq!(loaded.sheet_terms[0].variants[0].strings, 2);
        let crier = &loaded.knowledge.terms.entries[0];
        assert_eq!(crier.category, "персонаж");
        assert_eq!(crier.note.as_deref(), Some("Имена.\nЗазывала у ворот."));
        assert!(crier.match_case);
        // The file and its folder are what the terms were read from.
        assert_eq!(loaded.read.len(), 2);
    }

    #[test]
    fn a_term_of_the_file_takes_the_place_of_the_sheets_term() {
        let directory = tempfile::tempdir().expect("directory");
        let root = directory.path();
        write(
            root,
            "aeria-knowledge/terms.csv",
            "term,translation,forms,note,category\nThancred,Танкред,,,Персонаж\nfaire crier,зазывала,,,персонаж\nMist,Туман,,,место\nSylph,сильф,sylphs,,персонаж\n",
        );
        write(
            root,
            "aeria-knowledge/term-sheets.csv",
            "sheet,column,category\nENpcResident,0,персонаж\nBNpcName,0,существо\n",
        );
        write(
            root,
            "po/ENpcResident/0.po",
            &[
                entry("ENpcResident:1:0:0", "Thancred", "Танкред"),
                entry("ENpcResident:2:0:0", "faire crier", "зазывала праздника"),
                entry("ENpcResident:3:0:0", "sylphs", "Сильфы"),
                entry("ENpcResident:4:0:0", "Alphinaud", "Альфино"),
            ]
            .concat(),
        );
        write(
            root,
            "po/BNpcName/0.po",
            &[
                entry("BNpcName:1:0:0", "mist", "туман"),
                entry("BNpcName:2:0:0", "Alphinaud", "Альфино"),
                entry("BNpcName:3:0:0", "2", "2"),
            ]
            .concat(),
        );
        let loaded = project_knowledge(root);
        let terms: Vec<(&str, &str)> = loaded
            .sheet_terms
            .iter()
            .map(|term| (term.term.as_str(), term.translation.as_str()))
            .collect();
        // A term of the file takes the name in its own category, ignoring
        // case, by its headword or a form: the creature is a meaning of
        // `Mist` of its own.
        // A name translated alike in another category is one term; a source
        // without a letter names nothing.
        assert_eq!(terms, [("Alphinaud", "Альфино"), ("mist", "туман")]);
        // The file's term the sheet gives alike can be taken out; one that
        // says more cannot.
        assert_eq!(loaded.redundant, ["Thancred"]);
        // What the file's terms take the place of is kept with them.
        let replaced: Vec<(usize, &str)> = loaded
            .replaced
            .iter()
            .map(|replaced| (replaced.entry, replaced.term.translation.as_str()))
            .collect();
        assert_eq!(
            replaced,
            [(0, "Танкред"), (1, "зазывала праздника"), (3, "Сильфы")]
        );
        assert_eq!(
            loaded
                .knowledge
                .terms
                .missing_in("A mist rises", "Встаёт туман"),
            Vec::<String>::new()
        );
    }
}
