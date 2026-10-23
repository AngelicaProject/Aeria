//! The project's skills for machine translation runs, in
//! `aeria-knowledge/skills/` (see `docs/formats/knowledge-v1.md#skills`):
//! listed, counted, run, and saved or removed only while a file still has
//! the content the editor loaded, so a change made meanwhile by hand or by
//! Git is never overwritten.

use std::path::Path;

use aeria_knowledge::{
    MAX_SKILL_BYTES, SkillScope, check_name, list_skills, parse_skill, read_skill, skill_path,
    write_skill,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// Where a skill applies (see [`SkillScope`]).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScopeDto {
    pub paths: Vec<String>,
    pub exclude: Vec<String>,
    pub source: Option<String>,
    pub translation: Option<String>,
    pub other: Option<String>,
}

impl From<SkillScope> for ScopeDto {
    fn from(scope: SkillScope) -> Self {
        Self {
            paths: scope.paths,
            exclude: scope.exclude,
            source: scope.source,
            translation: scope.translation,
            other: scope.other,
        }
    }
}

impl ScopeDto {
    /// The scope with each list trimmed and each empty expression left out.
    fn scope(self) -> SkillScope {
        let list = |items: Vec<String>| -> Vec<String> {
            items
                .into_iter()
                .map(|item| item.trim().to_owned())
                .filter(|item| !item.is_empty())
                .collect()
        };
        let pattern = |pattern: Option<String>| pattern.filter(|pattern| !pattern.is_empty());
        SkillScope {
            paths: list(self.paths),
            exclude: list(self.exclude),
            source: pattern(self.source),
            translation: pattern(self.translation),
            other: pattern(self.other),
        }
    }
}

/// One skill: its file's text, and its scope and body, or why its front
/// matter cannot be read.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDto {
    pub name: String,
    pub text: String,
    pub scope: ScopeDto,
    pub body: String,
    pub problem: Option<String>,
}

/// The skills of the project, by name, and the files that cannot be read.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsDto {
    pub skills: Vec<SkillDto>,
    pub problems: Vec<String>,
}

fn root(app: &tauri::AppHandle) -> CommandResult<std::path::PathBuf> {
    Ok(app.state::<DesktopState>().session()?.root().to_owned())
}

fn listed(root: &Path) -> SkillsDto {
    let (files, problems) = list_skills(root);
    SkillsDto {
        skills: files
            .into_iter()
            .map(|file| match file.skill {
                Ok(skill) => SkillDto {
                    name: file.name,
                    text: file.text,
                    scope: skill.scope.into(),
                    body: skill.body,
                    problem: None,
                },
                Err(problem) => SkillDto {
                    name: file.name,
                    body: file.text.clone(),
                    text: file.text,
                    scope: ScopeDto::default(),
                    problem: Some(problem),
                },
            })
            .collect(),
        problems,
    }
}

fn read(root: &Path, name: &str) -> CommandResult<Option<String>> {
    read_skill(root, name).map_err(|message| CommandError::new("projectKnowledgeRead", message))
}

fn conflict(name: &str) -> CommandError {
    CommandError::new(
        "projectKnowledgeConflict",
        format!("the skill {name:?} changed after it was loaded"),
    )
}

fn write_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::new("projectKnowledgeWrite", error.to_string())
}

/// Writes `text` to the file of `name` through a temporary file and a
/// rename, so a reader never sees half of it.
fn write(root: &Path, name: &str, text: &str) -> CommandResult<()> {
    let path = skill_path(root, name);
    let directory = path.parent().unwrap_or(root).to_owned();
    let partial = directory.join(format!(".{name}.md.partial"));
    let written = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&directory)?;
        let mut file = std::fs::File::create(&partial)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&partial, &path)
    })();
    written.map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        write_error(format!("{name}: {error}"))
    })
}

/// Saves the skill `name` with `text`, its whole file. `previous` is the
/// name it was loaded under, `None` for a new one, and `expected` its file
/// as loaded: the save is refused when the file changed meanwhile, when
/// another skill already has the name, or when its front matter cannot be
/// read. A new name renames the file.
fn save(
    root: &Path,
    name: &str,
    previous: Option<&str>,
    expected: Option<&str>,
    text: &str,
) -> CommandResult<SkillsDto> {
    let invalid = |message: String| CommandError::new("projectKnowledgeInvalid", message);
    check_name(name).map_err(invalid)?;
    if text.len() as u64 > MAX_SKILL_BYTES {
        return Err(invalid(format!(
            "a skill is at most {MAX_SKILL_BYTES} bytes"
        )));
    }
    parse_skill(text).map_err(invalid)?;
    let renamed = previous.is_some_and(|previous| previous != name);
    // A name that differs only by case is the same file on Windows.
    let same_file = previous.is_some_and(|previous| previous.to_lowercase() == name.to_lowercase());
    if (previous.is_none() || renamed) && !same_file && read(root, name)?.is_some() {
        return Err(CommandError::new(
            "skillExists",
            format!("a skill named {name:?} already exists"),
        ));
    }
    let loaded_as = previous.unwrap_or(name);
    if read(root, loaded_as)?.as_deref() != expected {
        return Err(conflict(loaded_as));
    }
    match previous {
        Some(previous) if renamed && same_file => {
            // The file takes the new case of its name.
            let old = skill_path(root, previous);
            let step = old.with_extension("md.renaming");
            std::fs::rename(&old, &step)
                .and_then(|()| std::fs::rename(&step, skill_path(root, name)))
                .map_err(write_error)?;
            write(root, name, text)?;
        }
        Some(previous) if renamed => {
            write(root, name, text)?;
            std::fs::remove_file(skill_path(root, previous)).map_err(write_error)?;
        }
        _ => write(root, name, text)?,
    }
    Ok(listed(root))
}

/// What a run of `scope` with `body` asks: its paths, and the correction.
fn fix_of(
    scope: SkillScope,
    body: String,
    name: Option<String>,
) -> (Vec<String>, aeria_model::Fix) {
    (
        scope.paths,
        aeria_model::Fix {
            request: Some(body),
            skill: name,
            exclude: scope.exclude,
            filter: aeria_model::run::Filter {
                source: scope.source,
                translation: scope.translation,
                other: scope.other,
            },
            ..aeria_model::Fix::default()
        },
    )
}

#[tauri::command(rename_all = "camelCase")]
/// The skills of the open project.
///
/// # Errors
///
/// Returns `noProjectOpen`.
pub async fn skills(app: tauri::AppHandle) -> CommandResult<SkillsDto> {
    run_blocking(move || Ok(listed(&root(&app)?))).await
}

#[tauri::command(rename_all = "camelCase")]
/// Saves a skill from its scope and body, in canonical form, as [`save`]
/// does.
///
/// # Errors
///
/// Returns `projectKnowledgeInvalid` for a name, scope, or text that cannot
/// be saved, `projectKnowledgeConflict` when the file changed, `skillExists`
/// for a name another skill has, or `projectKnowledgeWrite`.
pub async fn save_skill(
    app: tauri::AppHandle,
    name: String,
    previous: Option<String>,
    expected: Option<String>,
    scope: ScopeDto,
    body: String,
) -> CommandResult<SkillsDto> {
    run_blocking(move || {
        let text = write_skill(&scope.scope(), &body);
        save(
            &root(&app)?,
            name.trim(),
            previous.as_deref(),
            expected.as_deref(),
            &text,
        )
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Removes the skill `name` if its file still has the `expected` text.
///
/// # Errors
///
/// Returns `projectKnowledgeConflict` when the file changed, or
/// `projectKnowledgeWrite`.
pub async fn delete_skill(
    app: tauri::AppHandle,
    name: String,
    expected: String,
) -> CommandResult<SkillsDto> {
    run_blocking(move || {
        let root = root(&app)?;
        if read(&root, &name)?.as_deref() != Some(expected.as_str()) {
            return Err(conflict(&name));
        }
        std::fs::remove_file(skill_path(&root, &name)).map_err(write_error)?;
        Ok(listed(&root))
    })
    .await
}

/// How many strings a run of a scope takes, by each of its paths.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeCountDto {
    pub total: usize,
    /// One count per path of the scope, by the first path that chooses each
    /// string's file; empty when the scope has no paths.
    pub paths: Vec<usize>,
}

#[tauri::command(rename_all = "camelCase")]
/// How many translations a run of `scope` takes: translated, not reviewed,
/// in its paths and not excluded, matching its expressions.
///
/// # Errors
///
/// Returns `skillScopeInvalid` for an expression that does not compile,
/// `noProjectOpen`, or `projectKnowledgeRead` when `po/` cannot be read.
pub async fn skill_count(app: tauri::AppHandle, scope: ScopeDto) -> CommandResult<ScopeCountDto> {
    run_blocking(move || {
        let session = app.state::<DesktopState>().session()?;
        let (paths, fix) = fix_of(scope.scope(), String::new(), None);
        aeria_model::run::Matcher::new(&fix.filter)
            .map_err(|message| CommandError::new("skillScopeInvalid", message))?;
        let counts = aeria_model::run::count_fix(&session, &fix, &paths)
            .map_err(|message| CommandError::new("projectKnowledgeRead", message))?;
        Ok(ScopeCountDto {
            total: counts.iter().sum(),
            paths: if paths.is_empty() { Vec::new() } else { counts },
        })
    })
    .await
}

#[tauri::command(rename_all = "camelCase")]
/// Runs the skill `name` as saved over the translations of its scope.
///
/// # Errors
///
/// Returns `skillNotFound`, `projectKnowledgeInvalid` for a skill whose
/// front matter cannot be read, `translationRunning` while a run goes,
/// `noProjectOpen`, or an error of the credential store.
pub async fn run_skill(
    app: tauri::AppHandle,
    name: String,
    model: String,
    effort: Option<String>,
) -> CommandResult<()> {
    let root = root(&app)?;
    let text = read(&root, &name)?.ok_or_else(|| {
        CommandError::new(
            "skillNotFound",
            format!("the project has no skill {name:?}"),
        )
    })?;
    let (scope, body) = parse_skill(&text)
        .map_err(|message| CommandError::new("projectKnowledgeInvalid", message))?;
    let (paths, fix) = fix_of(scope, body, Some(name));
    crate::translate::start_run(
        &app,
        aeria_model::Options {
            paths,
            fuzzy: false,
            contexts: Vec::new(),
            model,
            effort: effort.filter(|effort| !effort.is_empty()),
            fix: Some(fix),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skill_is_saved_renamed_and_kept_from_changes_made_meanwhile() {
        let root = tempfile::tempdir().expect("root");
        let root = root.path();
        let saved = save(root, "Род игрока", None, None, "Исправь род.\n").expect("save");
        assert_eq!(saved.skills.len(), 1);
        let again = save(root, "Род игрока", None, None, "Другое").expect_err("exists");
        assert_eq!(again.code, "skillExists");
        let changed = save(
            root,
            "Род игрока",
            Some("Род игрока"),
            Some("Старое"),
            "Новое",
        )
        .expect_err("conflict");
        assert_eq!(changed.code, "projectKnowledgeConflict");
        let scoped = write_skill(
            &ScopeDto {
                paths: vec![" quest/ ".to_owned(), String::new()],
                other: Some("\\$gn4".to_owned()),
                ..ScopeDto::default()
            }
            .scope(),
            "Исправь пол.",
        );
        let renamed = save(
            root,
            "Пол игрока",
            Some("Род игрока"),
            Some("Исправь род.\n"),
            &scoped,
        )
        .expect("rename");
        assert_eq!(renamed.skills[0].name, "Пол игрока");
        assert_eq!(renamed.skills[0].scope.paths, ["quest/"]);
        assert_eq!(renamed.skills[0].scope.other.as_deref(), Some("\\$gn4"));
        assert_eq!(renamed.skills[0].body, "Исправь пол.\n");
        let recased = save(
            root,
            "пол игрока",
            Some("Пол игрока"),
            Some(&scoped),
            &scoped,
        )
        .expect("case");
        assert_eq!(recased.skills[0].name, "пол игрока");
        let broken = save(root, "x", None, None, "---\nwhere: []\n---\n").expect_err("front");
        assert_eq!(broken.code, "projectKnowledgeInvalid");
        assert!(save(root, "a/b", None, None, "x").is_err());
    }
}
