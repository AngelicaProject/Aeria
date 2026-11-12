use aeria_po::glossary_project::{self as catalog, EditedTerm, TermMerge};
use aeria_po::{Entry, PoFile, project_knowledge};
use std::{collections::BTreeSet, fs, path::Path};

fn settings(root: &Path) {
    aeria_po::write_settings(
        root,
        &aeria_po::Settings {
            format: "aeria-po/2".to_owned(),
            source_language: "en".to_owned(),
            target_language: "ru".to_owned(),
            game_version: String::new(),
        },
    )
    .unwrap();
}

fn write(root: &Path, path: &str, text: &str) {
    let full = root.join(path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, text).unwrap();
}

fn entry(context: &str, source: &str, translation: &str) -> Entry {
    Entry {
        context: context.to_owned(),
        source: source.to_owned(),
        translation: translation.to_owned(),
        ..Entry::default()
    }
}

fn file(root: &Path, path: &str, entries: Vec<Entry>) {
    write(
        root,
        path,
        &PoFile {
            entries,
            ..PoFile::default()
        }
        .write(),
    );
}

fn recorded(root: &Path) {
    let knowledge = project_knowledge(root).knowledge;
    for path in aeria_po::list(root).unwrap() {
        let full = root.join("po").join(path);
        let mut file = PoFile::parse(&fs::read_to_string(&full).unwrap()).0;
        for entry in &mut file.entries {
            entry.record_terms(&knowledge.terms);
        }
        fs::write(full, file.write()).unwrap();
    }
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    settings(root);
    write(
        root,
        "aeria-knowledge/terms.csv",
        "term,translation,forms,note,category\nThe Studium,Академия,,,место\nAether,эфир,,,понятие\n",
    );
    write(
        root,
        "aeria-knowledge/term-sheets.csv",
        "sheet,column,category,note\nPlaceName,0,место,Названия мест.\n",
    );
    file(
        root,
        "po/PlaceName/0.po",
        vec![
            entry("PlaceName:1:0:0", "The Studium", "Академия"),
            entry("PlaceName:2:0:0", "The Studium", "Академия"),
        ],
    );
    file(
        root,
        "po/Dialogue.po",
        vec![entry(
            "Dialogue:1:0:0",
            "Welcome to the Studium",
            "Добро пожаловать в Академию",
        )],
    );
    recorded(root);
    dir
}

fn edits(loaded: &catalog::Resolved) -> Vec<EditedTerm> {
    loaded
        .catalog
        .terms
        .iter()
        .zip(&loaded.knowledge.terms.entries)
        .map(|(t, e)| EditedTerm {
            id: Some(t.id.clone()),
            term: e.term.clone(),
            translation: e.translation.clone(),
            forms: e.forms.clone(),
            note: e.note.clone(),
            match_case: e.match_case,
            category: e.category.clone(),
        })
        .collect()
}

fn outdated(root: &Path) -> BTreeSet<(String, String)> {
    let knowledge = project_knowledge(root).knowledge;
    assert!(knowledge.problems.is_empty(), "{:?}", knowledge.problems);
    aeria_po::Corpus::default()
        .outdated_strings(root, &knowledge.terms, &[])
        .unwrap()
        .into_iter()
        .map(|s| (s.path, s.context))
        .collect()
}

#[test]
fn migration_preserves_matching_versions_and_archives_original_decisions() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let before = outdated(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    assert_eq!(before, outdated(root));
    let index = loaded
        .catalog
        .terms
        .iter()
        .position(|t| t.term == "The Studium")
        .unwrap();
    assert!(!loaded.catalog.terms[index].match_case);
    assert!(loaded.catalog.terms[index].translation.is_none());
    assert_eq!(loaded.catalog.terms[index].sources.len(), 1);
    assert_eq!(
        loaded.knowledge.terms.matches("the Studium")[0].term,
        "The Studium"
    );
    assert!(!root.join("aeria-knowledge/terms.csv").exists());
    assert!(root.join("aeria-knowledge/legacy-v1/terms.csv").exists());
    assert_eq!(aeria_po::read_settings(root).unwrap().format, "aeria-po/3");
    assert!(root.join("aeria-knowledge/legacy-v1/aeria.json").exists());
    assert_eq!(
        catalog::migrate(root, "ru").unwrap().catalog,
        loaded.catalog
    );
}

#[test]
fn editing_a_sheet_term_writes_every_source_and_only_other_strings_become_outdated() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    let expected = catalog::token(&loaded);
    let mut changes = edits(&loaded);
    changes
        .iter_mut()
        .find(|t| t.term == "The Studium")
        .unwrap()
        .translation = "Училище".to_owned();
    let after = catalog::save(root, "ru", &expected, changes, None, None, &[]).unwrap();
    let names = PoFile::parse(&fs::read_to_string(root.join("po/PlaceName/0.po")).unwrap()).0;
    assert!(
        names
            .entries
            .iter()
            .all(|e| e.translation == "Училище"
                && e.outdated_terms(&after.knowledge.terms).is_empty())
    );
    assert_eq!(
        outdated(root),
        BTreeSet::from([("Dialogue.po".to_owned(), "Dialogue:1:0:0".to_owned())])
    );
    assert!(
        after
            .catalog
            .terms
            .iter()
            .filter(|t| !t.sources.is_empty())
            .all(|t| t.translation.is_none())
    );
    assert!(!root.join("aeria-knowledge/terms.csv").exists());
}

#[test]
fn duplicate_creation_and_stale_source_versions_are_refused_without_writes() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    let expected = catalog::token(&loaded);
    let mut changes = edits(&loaded);
    let mut duplicate = changes[0].clone();
    duplicate.id = None;
    duplicate.term = "  THE STUDIUM  ".to_owned();
    changes.push(duplicate);
    assert!(catalog::save(root, "ru", &expected, changes, None, None, &[]).is_err());
    assert_eq!(expected, catalog::token(&catalog::load(root).unwrap()));
    let source = root.join("po/PlaceName/0.po");
    let text = fs::read_to_string(&source).unwrap();
    fs::write(&source, text.replace("Академия", "Коллегия")).unwrap();
    assert!(catalog::save(root, "ru", &expected, edits(&loaded), None, None, &[]).is_err());
}

#[test]
fn category_rename_notes_and_defaults_do_not_reclassify_existing_terms_or_stale_strings() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    let expected = catalog::token(&loaded);
    let mut categories = loaded.catalog.categories.clone();
    categories
        .iter_mut()
        .find(|c| c.name == "место")
        .unwrap()
        .name = "локация".to_owned();
    categories
        .iter_mut()
        .find(|c| c.name == "локация")
        .unwrap()
        .note = "Новая общая заметка".to_owned();
    let mut changes = edits(&loaded);
    changes
        .iter_mut()
        .filter(|e| e.category == "место")
        .for_each(|e| e.category = "локация".to_owned());
    let renamed =
        catalog::save(root, "ru", &expected, changes, Some(categories), None, &[]).unwrap();
    assert!(outdated(root).is_empty());
    let mut sheets = renamed.knowledge.sheets.clone();
    sheets[0].category = "понятие".to_owned();
    let updated = catalog::save(
        root,
        "ru",
        &catalog::token(&renamed),
        edits(&renamed),
        None,
        Some(sheets),
        &[],
    )
    .unwrap();
    assert_eq!(
        updated
            .knowledge
            .terms
            .find("The Studium")
            .unwrap()
            .category,
        "локация"
    );
    file(
        root,
        "po/PlaceName/1000.po",
        vec![entry("PlaceName:1001:0:0", "New Place", "Новое место")],
    );
    let discovered = catalog::load(root).unwrap();
    assert_eq!(
        discovered
            .knowledge
            .terms
            .find("New Place")
            .unwrap()
            .category,
        "понятие"
    );
    assert_eq!(
        discovered
            .knowledge
            .terms
            .find("The Studium")
            .unwrap()
            .category,
        "локация"
    );
}

#[test]
fn equivalent_category_merge_rebases_only_known_current_records() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    settings(root);
    write(
        root,
        "aeria-knowledge/terms.csv",
        "term,translation,category\nDrake,дрейк,персонаж\nDrake,дрейк,существо\n",
    );
    write(
        root,
        "aeria-knowledge/term-sheets.csv",
        "sheet,column,category\nENpcResident,0,персонаж\nBNpcName,0,существо\n",
    );
    file(
        root,
        "po/ENpcResident/0.po",
        vec![entry("ENpcResident:1:0:0", "Drake", "дрейк")],
    );
    file(
        root,
        "po/BNpcName/0.po",
        vec![entry("BNpcName:1:0:0", "Drake", "дрейк")],
    );
    file(
        root,
        "po/Dialogue.po",
        vec![
            entry("Dialogue:1:0:0", "A Drake", "Дрейк"),
            entry("Dialogue:2:0:0", "Old Drake", "Старый дрейк"),
        ],
    );
    recorded(root);
    let full = root.join("po/Dialogue.po");
    let mut dialogue = PoFile::parse(&fs::read_to_string(&full).unwrap()).0;
    dialogue.entries[1].written_with.clear();
    fs::write(full, dialogue.write()).unwrap();
    let before = outdated(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    let from = &loaded.catalog.terms[0];
    let into = &loaded.catalog.terms[1];
    let target = loaded
        .catalog
        .categories
        .iter()
        .find(|c| c.id == into.category)
        .unwrap();
    let categories = loaded
        .catalog
        .categories
        .iter()
        .filter(|c| c.id != from.category)
        .cloned()
        .collect();
    let changes = edits(&loaded)
        .into_iter()
        .filter(|e| e.id.as_deref() != Some(&from.id))
        .collect();
    let mut sheets = loaded.knowledge.sheets.clone();
    for sheet in &mut sheets {
        sheet.category = target.name.clone();
    }
    let after = catalog::save(
        root,
        "ru",
        &catalog::token(&loaded),
        changes,
        Some(categories),
        Some(sheets),
        &[TermMerge {
            from: from.id.clone(),
            into: into.id.clone(),
        }],
    )
    .unwrap();
    assert_eq!(after.catalog.terms.len(), 1);
    assert_eq!(after.catalog.terms[0].sources.len(), 2);
    assert_eq!(before, outdated(root));
}

#[test]
fn migration_refuses_conflicting_reviewed_strings_before_any_write() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let full = root.join("po/PlaceName/0.po");
    let mut names = PoFile::parse(&fs::read_to_string(&full).unwrap()).0;
    for entry in &mut names.entries {
        entry.translation = "Коллегия".to_owned();
        entry.set_reviewed(true);
    }
    fs::write(&full, names.write()).unwrap();
    let before = fs::read(&full).unwrap();
    assert!(
        catalog::migrate(root, "ru")
            .unwrap_err()
            .contains("reviewed")
    );
    assert_eq!(before, fs::read(full).unwrap());
    assert!(!root.join(aeria_knowledge::catalog::CATALOG_PATH).exists());
    assert!(root.join("aeria-knowledge/terms.csv").exists());
}

#[test]
fn interrupted_transactions_finish_without_overwriting_a_third_version() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    let catalog_path = aeria_knowledge::catalog::CATALOG_PATH;
    let before = fs::read_to_string(root.join(catalog_path)).unwrap();
    let mut changed = loaded.catalog.clone();
    changed.categories[0].note = "Recovered".to_owned();
    let after = changed.text().unwrap();
    let source_path = "po/PlaceName/0.po";
    let source = fs::read_to_string(root.join(source_path)).unwrap();
    let journal = serde_json::json!([
        { "path":source_path,"before":source,"after":source.clone()+"\n" },
        { "path":catalog_path,"before":before,"after":after }
    ]);
    write(
        root,
        ".aeria/glossary-transaction.json",
        &journal.to_string(),
    );
    // Simulate a process interruption after the first file was published.
    write(root, source_path, &(source.clone() + "\n"));
    catalog::recover(root).unwrap();
    assert_eq!(fs::read_to_string(root.join(catalog_path)).unwrap(), after);
    assert!(!root.join(".aeria/glossary-transaction.json").exists());
    write(
        root,
        ".aeria/glossary-transaction.json",
        &journal.to_string(),
    );
    write(root, source_path, "external edit");
    assert!(catalog::recover(root).is_err());
    assert_eq!(
        fs::read_to_string(root.join(source_path)).unwrap(),
        "external edit"
    );
    assert!(root.join(".aeria/glossary-transaction.json").exists());
}

#[test]
fn unknown_catalog_versions_and_journal_traversal_never_fall_back_or_write() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    let mut invalid = serde_json::to_value(&loaded.catalog).unwrap();
    invalid["formatVersion"] = 99.into();
    write(
        root,
        aeria_knowledge::catalog::CATALOG_PATH,
        &invalid.to_string(),
    );
    assert!(catalog::load(root).is_err());
    assert!(!project_knowledge(root).knowledge.problems.is_empty());
    let journal = serde_json::json!([{ "path":"po/../outside.txt","before":null,"after":"bad" }]);
    write(
        root,
        ".aeria/glossary-transaction.json",
        &journal.to_string(),
    );
    assert!(catalog::recover(root).is_err());
    assert!(!root.join("outside.txt").exists());
}

#[test]
fn temporarily_missing_source_retains_its_independent_settings() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    let full = root.join("po/PlaceName/0.po");
    let source = fs::read_to_string(&full).unwrap();
    fs::write(&full, source.replace("msgstr \"Академия\"", "msgstr \"\"")).unwrap();
    let inactive = catalog::load(root).unwrap();
    assert!(inactive.knowledge.terms.find("The Studium").is_none());
    let changes: Vec<_> = inactive
        .term_ids
        .iter()
        .zip(&inactive.knowledge.terms.entries)
        .map(|(id, e)| EditedTerm {
            id: Some(id.clone()),
            term: e.term.clone(),
            translation: e.translation.clone(),
            forms: e.forms.clone(),
            note: e.note.clone(),
            match_case: e.match_case,
            category: e.category.clone(),
        })
        .collect();
    catalog::save(
        root,
        "ru",
        &catalog::token(&inactive),
        changes,
        None,
        None,
        &[],
    )
    .unwrap();
    fs::write(full, source).unwrap();
    let restored = catalog::load(root).unwrap();
    assert_eq!(loaded.catalog.terms, restored.catalog.terms);
    assert!(
        !restored
            .knowledge
            .terms
            .find("The Studium")
            .unwrap()
            .match_case
    );
}

#[test]
fn unsafe_source_translation_and_cyclic_merges_are_refused_without_writes() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let loaded = catalog::migrate(root, "ru").unwrap();
    let expected = catalog::token(&loaded);
    let mut changes = edits(&loaded);
    changes[0].translation = "<if (>".to_owned();
    assert!(catalog::save(root, "ru", &expected, changes, None, None, &[]).is_err());
    let a = loaded.catalog.terms[0].id.clone();
    let b = loaded.catalog.terms[1].id.clone();
    assert!(
        catalog::save(
            root,
            "ru",
            &expected,
            edits(&loaded),
            None,
            None,
            &[
                TermMerge {
                    from: a.clone(),
                    into: b.clone()
                },
                TermMerge { from: b, into: a }
            ]
        )
        .is_err()
    );
    assert_eq!(expected, catalog::token(&catalog::load(root).unwrap()));
}

#[test]
fn an_explicit_name_edit_fills_untranslated_rows_and_resolves_fuzzy_rows_but_keeps_exceptions() {
    let dir = fixture();
    let root = dir.path();
    settings(root);
    let full = root.join("po/PlaceName/0.po");
    let mut names = PoFile::parse(&fs::read_to_string(&full).unwrap()).0;
    names
        .entries
        .push(entry("PlaceName:3:0:0", "The Studium", ""));
    let mut fuzzy = entry("PlaceName:4:0:0", "The Studium", "Старое название");
    fuzzy.fuzzy = true;
    fuzzy.previous = Some("Old Studium".to_owned());
    names.entries.push(fuzzy);
    let mut excepted = entry("PlaceName:5:0:0", "The Studium", "Другое значение");
    excepted.term_exceptions.push("The Studium".to_owned());
    names.entries.push(excepted);
    fs::write(&full, names.write()).unwrap();
    recorded(root);
    let original = fs::read(&full).unwrap();
    let loaded = catalog::migrate(root, "ru").unwrap();
    assert_eq!(
        original,
        fs::read(&full).unwrap(),
        "migration must not fill missing translations"
    );
    let mut changes = edits(&loaded);
    changes
        .iter_mut()
        .find(|e| e.term == "The Studium")
        .unwrap()
        .translation = "Училище".to_owned();
    let after = catalog::save(
        root,
        "ru",
        &catalog::token(&loaded),
        changes,
        None,
        None,
        &[],
    )
    .unwrap();
    let names = PoFile::parse(&fs::read_to_string(full).unwrap()).0;
    for entry in &names.entries[..4] {
        assert_eq!(entry.translation, "Училище");
        assert!(!entry.fuzzy);
        assert!(entry.previous.is_none());
        assert!(entry.outdated_terms(&after.knowledge.terms).is_empty());
    }
    assert_eq!(names.entries[4].translation, "Другое значение");
}
