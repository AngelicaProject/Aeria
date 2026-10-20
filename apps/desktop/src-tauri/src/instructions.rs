//! The project's instructions for machine translation runs, in
//! `aeria-knowledge/instructions/` (see `docs/formats/knowledge-v1.md#instructions`):
//! listed, and saved or removed only while a file still has the content the
//! editor loaded, so a change made meanwhile by hand or by Git is never
//! overwritten.

use std::path::Path;

use aeria_knowledge::{
    MAX_INSTRUCTION_BYTES, check_name, instruction_path, list_instructions, read_instruction,
};
use serde::Serialize;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

/// One instruction.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionDto {
    pub name: String,
    pub text: String,
}

/// The instructions of the project, by name, and the files that cannot be
/// read.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionsDto {
    pub instructions: Vec<InstructionDto>,
    pub problems: Vec<String>,
}

fn root(app: &tauri::AppHandle) -> CommandResult<std::path::PathBuf> {
    Ok(app.state::<DesktopState>().session()?.root().to_owned())
}

fn listed(root: &Path) -> InstructionsDto {
    let (instructions, problems) = list_instructions(root);
    InstructionsDto {
        instructions: instructions
            .into_iter()
            .map(|instruction| InstructionDto {
                name: instruction.name,
                text: instruction.text,
            })
            .collect(),
        problems,
    }
}

fn read(root: &Path, name: &str) -> CommandResult<Option<String>> {
    read_instruction(root, name)
        .map_err(|message| CommandError::new("projectKnowledgeRead", message))
}

fn conflict(name: &str) -> CommandError {
    CommandError::new(
        "projectKnowledgeConflict",
        format!("the instruction {name:?} changed after it was loaded"),
    )
}

/// Writes `text` to the file of `name` through a temporary file and a
/// rename, so a reader never sees half of it.
fn write(root: &Path, name: &str, text: &str) -> CommandResult<()> {
    let path = instruction_path(root, name);
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
        CommandError::new("projectKnowledgeWrite", format!("{name}: {error}"))
    })
}

/// Saves the instruction `name` with `text`. `previous` is the name it was
/// loaded under, `None` for a new one, and `expected` its text as loaded:
/// the save is refused when the file changed meanwhile, or when another
/// instruction already has the name. A new name renames the file.
fn save(
    root: &Path,
    name: &str,
    previous: Option<&str>,
    expected: Option<&str>,
    text: &str,
) -> CommandResult<InstructionsDto> {
    check_name(name).map_err(|message| CommandError::new("projectKnowledgeInvalid", message))?;
    if text.len() as u64 > MAX_INSTRUCTION_BYTES {
        return Err(CommandError::new(
            "projectKnowledgeInvalid",
            format!("an instruction is at most {MAX_INSTRUCTION_BYTES} bytes"),
        ));
    }
    let renamed = previous.is_some_and(|previous| previous != name);
    // A name that differs only by case is the same file on Windows.
    let same_file = previous.is_some_and(|previous| previous.to_lowercase() == name.to_lowercase());
    if (previous.is_none() || renamed) && !same_file && read(root, name)?.is_some() {
        return Err(CommandError::new(
            "instructionExists",
            format!("an instruction named {name:?} already exists"),
        ));
    }
    let loaded_as = previous.unwrap_or(name);
    if read(root, loaded_as)?.as_deref() != expected {
        return Err(conflict(loaded_as));
    }
    if let (true, Some(previous)) = (renamed, previous) {
        if same_file {
            // The file takes the new case of its name.
            let old = instruction_path(root, previous);
            let step = old.with_extension("md.renaming");
            std::fs::rename(&old, &step)
                .and_then(|()| std::fs::rename(&step, instruction_path(root, name)))
                .map_err(|error| CommandError::new("projectKnowledgeWrite", error.to_string()))?;
            write(root, name, text)?;
        } else {
            write(root, name, text)?;
            std::fs::remove_file(instruction_path(root, previous))
                .map_err(|error| CommandError::new("projectKnowledgeWrite", error.to_string()))?;
        }
    } else {
        write(root, name, text)?;
    }
    Ok(listed(root))
}

#[tauri::command(rename_all = "camelCase")]
/// The instructions of the open project.
///
/// # Errors
///
/// Returns `noProjectOpen`.
pub async fn instructions(app: tauri::AppHandle) -> CommandResult<InstructionsDto> {
    run_blocking(move || Ok(listed(&root(&app)?))).await
}

#[tauri::command(rename_all = "camelCase")]
/// Saves an instruction as [`save`] does.
///
/// # Errors
///
/// Returns `projectKnowledgeInvalid` for a name or text that cannot be
/// saved, `projectKnowledgeConflict` when the file changed, `instructionExists`
/// for a name another instruction has, or `projectKnowledgeWrite`.
pub async fn save_instruction(
    app: tauri::AppHandle,
    name: String,
    previous: Option<String>,
    expected: Option<String>,
    text: String,
) -> CommandResult<InstructionsDto> {
    run_blocking(move || {
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
/// Removes the instruction `name` if it still has the `expected` text.
///
/// # Errors
///
/// Returns `projectKnowledgeConflict` when the file changed, or
/// `projectKnowledgeWrite`.
pub async fn delete_instruction(
    app: tauri::AppHandle,
    name: String,
    expected: String,
) -> CommandResult<InstructionsDto> {
    run_blocking(move || {
        let root = root(&app)?;
        if read(&root, &name)?.as_deref() != Some(expected.as_str()) {
            return Err(conflict(&name));
        }
        std::fs::remove_file(instruction_path(&root, &name))
            .map_err(|error| CommandError::new("projectKnowledgeWrite", error.to_string()))?;
        Ok(listed(&root))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_instruction_is_saved_renamed_and_kept_from_changes_made_meanwhile() {
        let root = tempfile::tempdir().expect("root");
        let root = root.path();
        let saved = save(root, "Род игрока", None, None, "Исправь род.").expect("save");
        assert_eq!(saved.instructions.len(), 1);
        // A second new one with the name is refused.
        let again = save(root, "Род игрока", None, None, "Другое").expect_err("exists");
        assert_eq!(again.code, "instructionExists");
        // A change made meanwhile is never overwritten.
        let changed = save(
            root,
            "Род игрока",
            Some("Род игрока"),
            Some("Старое"),
            "Новое",
        )
        .expect_err("conflict");
        assert_eq!(changed.code, "projectKnowledgeConflict");
        // A new name renames the file, and the case of a name can change.
        let renamed = save(
            root,
            "Пол игрока",
            Some("Род игрока"),
            Some("Исправь род."),
            "Исправь пол.",
        )
        .expect("rename");
        let names: Vec<&str> = renamed
            .instructions
            .iter()
            .map(|instruction| instruction.name.as_str())
            .collect();
        assert_eq!(names, ["Пол игрока"]);
        let recased = save(
            root,
            "пол игрока",
            Some("Пол игрока"),
            Some("Исправь пол."),
            "Исправь пол.",
        )
        .expect("case");
        assert_eq!(recased.instructions[0].name, "пол игрока");
        assert!(save(root, "a/b", None, None, "x").is_err());
    }
}
