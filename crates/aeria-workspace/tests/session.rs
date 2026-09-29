//! Project sessions over synthetic games: opening, editing, reading pages,
//! and source updates.

mod support;

use std::fs;

use aeria_core::{DetachReason, ReviewState, SourceBinding, SourceStatus};
use aeria_rebase::UnitUpdateOutcome;
use aeria_source::SourceLanguage;
use aeria_sqpack::testing::TextSheet;
use aeria_workspace::{
    AssistedExpectation, AssistedWrite, AssistedWriteError, ProjectSession, ProjectSessionError,
    SourceUpdateRequirement, TranslationMutationError, TranslationReadError, TranslationRowCursor,
    WorkspaceError,
};
use support::{V1, V2, V3, copy_project, dialogue, game, game_in, managed_files, texts};

const SHEET: &str = "Addon";

fn binding(row: u32, column: u32) -> SourceBinding {
    SourceBinding::new(SHEET, row, 0, column)
}

#[test]
fn a_new_project_records_the_game_version_and_reopens() {
    let game = game(V1, &[(SHEET, &texts(&[(1, "Hello")]))]);
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), game.handle(), "ru").expect("initialize");
    session
        .set_target(&binding(1, 0), "Привет")
        .expect("target");
    drop(session);

    let manifest =
        fs::read_to_string(repository.path().join(".aeria/manifest.json")).expect("manifest");
    assert_eq!(
        manifest,
        format!(
            "{{\n  \"formatVersion\": 3,\n  \"sourceLanguage\": \"en\",\n  \"targetLanguage\": \"ru\",\n  \"gameVersion\": \"{V1}\"\n}}\n"
        )
    );
    let files = managed_files(repository.path());
    assert!(
        files.values().all(|bytes| !String::from_utf8_lossy(bytes)
            .contains(&*game.source.game_path().to_string_lossy())),
        "no game path is persisted"
    );
    let session = ProjectSession::open(repository.path(), game.handle()).expect("reopen");
    let unit = session
        .workspace()
        .unit_by_source_binding(&binding(1, 0))
        .expect("unit");
    assert_eq!(unit.source().text(), "Hello");
    assert_eq!(unit.target_macro(), "Привет");
}

#[test]
fn initialization_never_replaces_an_existing_project() {
    let game = game(V1, &[(SHEET, &texts(&[(1, "Hello")]))]);
    let repository = tempfile::tempdir().expect("repository");
    ProjectSession::initialize(repository.path(), game.handle(), "ru").expect("initialize");
    let before = managed_files(repository.path());
    assert!(ProjectSession::initialize(repository.path(), game.handle(), "fr").is_err());
    assert!(
        ProjectSession::initialize(
            tempfile::tempdir().expect("repository").path(),
            game.handle(),
            " "
        )
        .is_err()
    );
    assert_eq!(managed_files(repository.path()), before);
}

#[test]
fn another_source_language_or_an_older_game_cannot_open_the_project() {
    let sheet = texts(&[(1, "Hello")]);
    let new = game(V2, &[(SHEET, &sheet)]);
    let repository = tempfile::tempdir().expect("repository");
    ProjectSession::initialize(repository.path(), new.handle(), "ru").expect("initialize");
    let before = managed_files(repository.path());

    let german = game_in(V2, &[(SHEET, &sheet)], SourceLanguage::German);
    assert!(matches!(
        ProjectSession::open(repository.path(), german.handle()),
        Err(ProjectSessionError::Compatibility {
            source: WorkspaceError::SourceLanguageMismatch { .. },
            ..
        })
    ));
    let old = game(V1, &[(SHEET, &sheet)]);
    for result in [
        ProjectSession::open(repository.path(), old.handle()).map(|_| ()),
        ProjectSession::open_with_source_update(repository.path(), old.handle()).map(|_| ()),
    ] {
        assert!(matches!(
            result,
            Err(ProjectSessionError::GameOutdated { .. })
        ));
    }
    assert_eq!(managed_files(repository.path()), before);
}

#[test]
fn a_game_update_is_required_previewed_applied_and_repeatable_after_interruption() {
    let old = game(
        V1,
        &[(
            SHEET,
            &texts(&[(1, "Kept"), (2, "Changed"), (3, "Removed")]),
        )],
    );
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), old.handle(), "fr").expect("initialize");
    let ids: Vec<_> = (1..=3)
        .map(|row| {
            let id = session.set_target(&binding(row, 0), "t").expect("target");
            session
                .set_review_state(id, ReviewState::Reviewed)
                .expect("review");
            id
        })
        .collect();
    drop(session);
    let before = managed_files(repository.path());

    let new = game(V2, &[(SHEET, &texts(&[(1, "Kept"), (2, "Changed!")]))]);
    assert!(matches!(
        ProjectSession::open(repository.path(), new.handle()),
        Err(ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::GameUpdated { .. },
            ..
        })
    ));
    let preview =
        ProjectSession::preview_source_update(repository.path(), &new.source).expect("preview");
    assert_eq!(
        managed_files(repository.path()),
        before,
        "a preview writes nothing"
    );
    let outcomes: Vec<_> = ids
        .iter()
        .map(|id| {
            preview
                .plan
                .entries()
                .iter()
                .find(|entry| entry.translation_unit_id == *id)
                .expect("entry")
                .outcome
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            UnitUpdateOutcome::Unchanged,
            UnitUpdateOutcome::SourceChanged,
            UnitUpdateOutcome::Detached(DetachReason::RowRemoved),
        ]
    );

    // An update interrupted after the shards but before the manifest.
    let interrupted = copy_project(repository.path());
    let (session, report) =
        ProjectSession::open_with_source_update(repository.path(), new.handle()).expect("update");
    assert_eq!(report.expect("updated").plan, preview.plan);
    let unit = |index: usize| session.workspace().unit(ids[index]).expect("unit");
    assert_eq!(unit(0).review_state(), ReviewState::Reviewed);
    assert_eq!(unit(1).review_state(), ReviewState::NeedsReview);
    assert_eq!(unit(1).source().text(), "Changed!");
    assert_eq!(
        unit(2).source_status(),
        SourceStatus::Detached(DetachReason::RowRemoved)
    );
    assert_eq!(
        unit(2).source().text(),
        "Removed",
        "a detached unit keeps its facts"
    );
    assert_eq!(session.detached_units().count(), 1);
    let updated = managed_files(repository.path());
    drop(session);

    let manifest = std::path::PathBuf::from(".aeria/manifest.json");
    for (path, bytes) in &updated {
        if *path != manifest {
            fs::write(interrupted.path().join(path), bytes).expect("rewritten shard");
        }
    }
    assert!(matches!(
        ProjectSession::open(interrupted.path(), new.handle()),
        Err(ProjectSessionError::SourceUpdateRequired { .. })
    ));
    let (_, report) =
        ProjectSession::open_with_source_update(interrupted.path(), new.handle()).expect("repeat");
    assert_eq!(
        report.expect("manifest update").plan.summary.changed_units,
        0
    );
    assert_eq!(managed_files(interrupted.path()), updated);

    let (_, report) =
        ProjectSession::open_with_source_update(repository.path(), new.handle()).expect("again");
    assert!(report.is_none(), "an updated project opens directly");
}

#[test]
fn keyed_dialogue_translations_follow_their_lines_through_updates() {
    let old = game(
        V1,
        &[(SHEET, &dialogue(&[(1, "K1", "Alpha"), (2, "K2", "Beta")]))],
    );
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), old.handle(), "fr").expect("initialize");
    let alpha = session
        .set_target(&binding(1, 1), "Alpha fr")
        .expect("target");
    let beta = session.set_target(&binding(2, 1), "Bêta").expect("target");
    assert!(matches!(
        session.set_target(&binding(1, 0), "clé"),
        Err(TranslationMutationError::SourceNotTranslatable { .. })
    ));
    drop(session);

    let new = game(
        V2,
        &[(
            SHEET,
            &dialogue(&[(1, "K0", "Inserted"), (2, "K1", "Alpha"), (3, "K2", "Beta")]),
        )],
    );
    let (session, _) =
        ProjectSession::open_with_source_update(repository.path(), new.handle()).expect("update");
    let row_of = |id| {
        session
            .workspace()
            .unit(id)
            .expect("unit")
            .source_binding()
            .row_id()
    };
    assert_eq!((row_of(alpha), row_of(beta)), (2, 3));
    let page = session
        .page_translation_rows(SHEET, None, 10)
        .expect("page");
    let targets: Vec<_> = page
        .rows
        .iter()
        .map(|row| {
            row.cells[0]
                .translation
                .as_ref()
                .map(|overlay| overlay.target_macro.clone())
        })
        .collect();
    assert_eq!(
        targets,
        [None, Some("Alpha fr".to_owned()), Some("Bêta".to_owned())]
    );
}

#[test]
fn pages_compose_game_rows_with_the_sparse_overlay() {
    let sheet = TextSheet::new(3, &[0, 1, 2])
        .keyed(0)
        .row(1, &[(0, "ID_1"), (1, "Name"), (2, "")])
        .row(2, &[(0, "ID_2")])
        .row(3, &[(0, "ID_3"), (1, "..."), (2, "Description")]);
    let game = game(V1, &[(SHEET, &sheet)]);
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), game.handle(), "fr").expect("initialize");
    session
        .set_target(&binding(3, 2), "Description fr")
        .expect("target");
    let before = managed_files(repository.path());

    let first = session.page_translation_rows(SHEET, None, 2).expect("page");
    assert_eq!(first.rows.len(), 1, "row 2 has only context");
    let row = &first.rows[0];
    assert_eq!(row.context[0].source_macro, "ID_1");
    assert_eq!(
        row.cells.len(),
        1,
        "empty and context cells are not editable"
    );
    assert_eq!(row.cells[0].source_macro, "Name");
    assert!(row.cells[0].translation.is_none());
    let cursor = first.next_after.expect("more rows");
    assert_eq!((cursor.row_id(), cursor.subrow_id()), (2, 0));

    let second = session
        .page_translation_rows(SHEET, Some(&cursor), 2)
        .expect("page");
    assert!(second.next_after.is_none());
    let cells = &second.rows[0].cells;
    assert!(cells[0].formatting_only, "punctuation only");
    assert_eq!(
        cells[1].translation.as_ref().expect("overlay").target_macro,
        "Description fr"
    );
    assert_eq!(
        managed_files(repository.path()),
        before,
        "reads write nothing"
    );

    assert!(
        session
            .page_translation_rows("Missing", None, 2)
            .expect("page")
            .rows
            .is_empty()
    );
    assert!(matches!(
        session.page_translation_rows(SHEET, None, 0),
        Err(TranslationReadError::InvalidPageLimit { .. })
    ));
    assert!(matches!(
        session.page_translation_rows(SHEET, Some(&TranslationRowCursor::new("Other", 1, 0)), 2),
        Err(TranslationReadError::CursorSheetMismatch { .. })
    ));
    let progress = session.translation_progress();
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].translated, 1);
}

#[test]
fn the_target_language_is_set_in_the_manifest_and_keeps_every_unit() {
    let game = game(V1, &[(SHEET, &texts(&[(1, "Hello")]))]);
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), game.handle(), "und").expect("initialize");
    let id = session
        .set_target(&binding(1, 0), "Привет")
        .expect("target");
    let shards_before: Vec<_> = managed_files(repository.path())
        .into_iter()
        .filter(|(path, _)| !path.ends_with("manifest.json"))
        .collect();
    for invalid in ["", "und", "russian", "ru_RU"] {
        assert!(matches!(
            session.set_target_language(invalid),
            Err(ProjectSessionError::InvalidTargetLanguage { .. })
        ));
    }
    session.set_target_language("ru").expect("language");
    assert_eq!(session.workspace().metadata().target_language(), "ru");
    let manifest =
        fs::read_to_string(repository.path().join(".aeria/manifest.json")).expect("manifest");
    assert!(manifest.contains("\"targetLanguage\": \"ru\""));
    let shards_after: Vec<_> = managed_files(repository.path())
        .into_iter()
        .filter(|(path, _)| !path.ends_with("manifest.json"))
        .collect();
    assert_eq!(shards_after, shards_before, "units are not rewritten");
    // Edits continue against the new manifest, and a reopen sees the language.
    session
        .set_target(&binding(1, 0), "Здравствуй")
        .expect("edit after the change");
    let reopened = ProjectSession::open(repository.path(), game.handle()).expect("reopen");
    assert_eq!(reopened.workspace().metadata().target_language(), "ru");
    assert_eq!(
        reopened.workspace().unit(id).expect("unit").target_macro(),
        "Здравствуй"
    );
}

#[test]
fn mutations_validate_persist_once_and_skip_identical_writes() {
    let game = game(V1, &[(SHEET, &texts(&[(1, "Hello"), (2, "World")]))]);
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), game.handle(), "fr").expect("initialize");
    for target in ["", "   "] {
        assert!(matches!(
            session.set_target(&binding(1, 0), target),
            Err(TranslationMutationError::EmptyTarget)
        ));
    }
    assert!(matches!(
        session.set_target(&binding(1, 0), "<if $n1>"),
        Err(TranslationMutationError::Workspace(
            WorkspaceError::InvalidTarget { .. }
        ))
    ));
    assert!(
        managed_files(repository.path()).len() == 1,
        "only the manifest"
    );

    let id = session
        .set_target(&binding(1, 0), "Bonjour")
        .expect("create");
    let written = managed_files(repository.path());
    assert_eq!(written.len(), 2, "one shard");
    session
        .set_target(&binding(1, 0), "Bonjour")
        .expect("identical");
    session.set_note(id, None).expect("identical note");
    session
        .set_review_state(id, ReviewState::Draft)
        .expect("identical review");
    assert_eq!(managed_files(repository.path()), written);

    session
        .set_review_state(id, ReviewState::Reviewed)
        .expect("review");
    session.set_note(id, Some("note".to_owned())).expect("note");
    assert_eq!(
        session.workspace().unit(id).expect("unit").review_state(),
        ReviewState::Reviewed,
        "a note keeps the review"
    );
    session.set_target(&binding(1, 0), "Salut").expect("edit");
    let unit = session.workspace().unit(id).expect("unit");
    assert_eq!(unit.review_state(), ReviewState::Draft);
    assert_eq!(unit.translator_note(), Some("note"));

    let other = session
        .set_target(&binding(2, 0), "Monde")
        .expect("second unit");
    let reopened = ProjectSession::open(repository.path(), game.handle()).expect("reopen");
    assert_eq!(reopened.workspace().units().count(), 2);
    assert!(reopened.workspace().unit(other).is_some());
}

#[test]
fn assisted_targets_follow_the_structure_policy_and_compare_and_set() {
    let bytes = aeria_se::codec::encode("Hi <player-name $n1>!").expect("encode");
    let game = game(
        V1,
        &[(SHEET, &TextSheet::new(1, &[0]).row_bytes(1, 0, &bytes))],
    );
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), game.handle(), "ru").expect("initialize");
    let target = binding(1, 0);
    assert_eq!(
        session.source_macro(&target).expect("source"),
        "Hi <player-name $n1>!"
    );
    let untranslated = session.assisted_state(&target);
    assert_eq!(
        untranslated,
        AssistedExpectation {
            target: None,
            review_state: None
        }
    );
    assert!(matches!(
        session.set_assisted_target(&target, "Привет!", &untranslated, false),
        Err(AssistedWriteError::Structure { .. })
    ));
    let id = session
        .set_assisted_target(&target, "Привет, <player-name $n1>!", &untranslated, false)
        .expect("assisted");
    let written = session.assisted_state(&target);
    assert!(matches!(
        session.set_assisted_target(&target, "Здравствуй, <player-name $n1>!", &untranslated, false),
        Err(AssistedWriteError::Conflict { ref current }) if current == &written
    ));
    session
        .set_review_state(id, ReviewState::Reviewed)
        .expect("review");
    let reviewed = session.assisted_state(&target);
    assert!(matches!(
        session.set_assisted_target(&target, "Здравствуй, <player-name $n1>!", &reviewed, false),
        Err(AssistedWriteError::Reviewed)
    ));
    session
        .set_assisted_target(&target, "Здравствуй, <player-name $n1>!", &reviewed, true)
        .expect("approved replacement");
    assert!(matches!(
        session.source_macro(&binding(1, 9)),
        Err(TranslationMutationError::SourceNotTranslatable { .. })
    ));
}

#[test]
fn a_batch_of_assisted_targets_writes_what_passes_its_checks_at_once() {
    let game = game(
        V1,
        &[(SHEET, &texts(&[(1, "Hello"), (2, "Bye"), (3, "Yes")]))],
    );
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), game.handle(), "ru").expect("initialize");
    let untranslated = AssistedExpectation {
        target: None,
        review_state: None,
    };
    let write = |row: u32, target: &str, review: Option<ReviewState>| AssistedWrite {
        source_binding: binding(row, 0),
        target_macro: target.to_owned(),
        expected: untranslated.clone(),
        review_state: review,
    };
    let results = session.set_assisted_targets(
        &[
            write(1, "Привет", None),
            write(2, "<i>Пока", None),
            write(3, "Да", Some(ReviewState::NeedsReview)),
        ],
        false,
    );
    assert!(results[0].is_ok());
    assert!(matches!(
        results[1],
        Err(AssistedWriteError::Structure { .. })
    ));
    assert!(results[2].is_ok());

    let reopened = ProjectSession::open(repository.path(), game.handle()).expect("reopen");
    assert_eq!(
        reopened.assisted_state(&binding(1, 0)).target.as_deref(),
        Some("Привет")
    );
    assert_eq!(reopened.assisted_state(&binding(2, 0)).target, None);
    assert_eq!(
        reopened.assisted_state(&binding(3, 0)).review_state,
        Some(ReviewState::NeedsReview)
    );
}

#[test]
fn files_changed_behind_a_session_block_mutations_until_reconciled() {
    let game = game(V1, &[(SHEET, &texts(&[(1, "Hello")]))]);
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), game.handle(), "fr").expect("initialize");
    let id = session
        .set_target(&binding(1, 0), "Bonjour")
        .expect("target");

    // A merge replaces the shard with a unit translated against older text.
    let shard = fs::read_dir(repository.path().join(".aeria/units"))
        .expect("units")
        .map(|entry| entry.expect("entry").path())
        .next()
        .expect("shard");
    let stale = fs::read_to_string(&shard)
        .expect("shard")
        .replace(r#""source":"Hello""#, r#""source":"Hello there""#)
        .replace("Bonjour", "Salut");
    fs::write(&shard, stale).expect("stale shard");
    let before = managed_files(repository.path());
    for result in [
        session.set_target(&binding(1, 0), "Coucou").map(|_| ()),
        session.set_note(id, Some("note".to_owned())),
        session.set_review_state(id, ReviewState::Reviewed),
    ] {
        assert!(result.is_err(), "a mutation never overwrites unseen files");
    }
    assert_eq!(managed_files(repository.path()), before);

    assert!(matches!(
        session.reload_workspace(),
        Err(ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::SourceFactsMismatch { units: 1 },
            ..
        })
    ));
    session
        .reload_and_reconcile_workspace()
        .expect("reconcile")
        .expect("reconciled");
    let unit = session.workspace().unit(id).expect("unit");
    assert_eq!(unit.target_macro(), "Salut", "the merged edit is kept");
    assert_eq!(unit.review_state(), ReviewState::NeedsReview);
    assert_eq!(unit.source().text(), "Hello");
    session
        .set_target(&binding(1, 0), "Coucou")
        .expect("editable again");
}

#[test]
fn a_detached_unit_is_not_edited_and_a_chain_of_patches_reattaches_it() {
    let v1 = game(V1, &[(SHEET, &texts(&[(1, "Line"), (2, "Other")]))]);
    let repository = tempfile::tempdir().expect("repository");
    let mut session =
        ProjectSession::initialize(repository.path(), v1.handle(), "fr").expect("initialize");
    let id = session.set_target(&binding(1, 0), "Ligne").expect("target");
    drop(session);

    let v2 = game(V2, &[(SHEET, &texts(&[(2, "Other")]))]);
    let (mut session, _) =
        ProjectSession::open_with_source_update(repository.path(), v2.handle()).expect("v2");
    assert!(matches!(
        session.set_note(id, Some("n".to_owned())),
        Err(TranslationMutationError::Workspace(
            WorkspaceError::DetachedUnit { .. }
        ))
    ));
    drop(session);

    let v3 = game(V3, &[(SHEET, &texts(&[(1, "Line"), (2, "Other")]))]);
    let (session, report) =
        ProjectSession::open_with_source_update(repository.path(), v3.handle()).expect("v3");
    assert_eq!(report.expect("updated").plan.summary.reattached, 1);
    let unit = session.workspace().unit(id).expect("unit");
    assert!(unit.is_bound());
    assert_eq!(unit.target_macro(), "Ligne");
}

#[test]
fn units_merged_from_an_older_game_version_are_reviewed_never_misattributed() {
    // Two String columns; the patch inserts a column before them.
    let old_sheet = TextSheet::new(2, &[0, 1])
        .row(1, &[(0, "Name"), (1, "Description")])
        .row(2, &[(0, "Other"), (1, "Text")]);
    let new_sheet = TextSheet::new(3, &[0, 1, 2])
        .row(1, &[(0, "x"), (1, "Name"), (2, "Description")])
        .row(2, &[(0, "x"), (1, "Other"), (2, "Text, revised")]);
    let old = game(V1, &[(SHEET, &old_sheet)]);
    let new = game(V2, &[(SHEET, &new_sheet)]);

    // The main branch is updated to V2; a contributor branch translated on
    // V1 and is merged afterwards.
    let main = tempfile::tempdir().expect("main");
    let mut session =
        ProjectSession::initialize(main.path(), old.handle(), "fr").expect("initialize");
    let main_id = session.set_target(&binding(2, 0), "Autre").expect("target");
    let branch = copy_project(main.path());
    drop(session);
    ProjectSession::open_with_source_update(main.path(), new.handle()).expect("main update");

    let mut contributor = ProjectSession::open(branch.path(), old.handle()).expect("branch");
    let description = contributor
        .set_target(&binding(1, 1), "Déscription")
        .expect("target");
    let text = contributor
        .set_target(&binding(2, 1), "Texte")
        .expect("target");
    drop(contributor);
    let merged_ids = [description.to_string(), text.to_string()];
    for (path, bytes) in managed_files(branch.path()) {
        if path.ends_with("manifest.json") {
            continue;
        }
        let target = main.path().join(&path);
        let mut lines: Vec<String> = fs::read_to_string(&target)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        for line in String::from_utf8(bytes).expect("UTF-8").lines() {
            if merged_ids.iter().any(|id| line.contains(id.as_str())) {
                lines.push(line.to_owned());
            }
        }
        lines.sort();
        fs::create_dir_all(target.parent().expect("parent")).expect("directory");
        fs::write(target, lines.join("\n") + "\n").expect("merged shard");
    }

    assert!(matches!(
        ProjectSession::open(main.path(), new.handle()),
        Err(ProjectSessionError::SourceUpdateRequired {
            requirement: SourceUpdateRequirement::SourceFactsMismatch { units: 2 },
            ..
        })
    ));
    let (session, _) =
        ProjectSession::open_with_source_update(main.path(), new.handle()).expect("reconcile");
    let unit = |id| session.workspace().unit(id).expect("unit");
    assert_eq!(
        unit(description).source_binding(),
        &binding(1, 2),
        "follows its column"
    );
    assert_eq!(unit(description).review_state(), ReviewState::Draft);
    assert_eq!(unit(text).source_binding(), &binding(2, 2));
    assert_eq!(unit(text).review_state(), ReviewState::NeedsReview);
    assert_eq!(unit(text).source().text(), "Text, revised");
    assert_eq!(
        unit(main_id).source_binding(),
        &binding(2, 1),
        "the main branch unit keeps the cell the old column index now names"
    );
}
