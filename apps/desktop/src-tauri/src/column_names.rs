//! Column names of the open project's sheets from EXDSchema, for display.
//!
//! Schema files are downloaded from GitHub into the cache directory
//! (`exdschema/`) and named against the installed game's sheet headers by
//! `aeria-schema`. The names are presentation only.

use std::sync::OnceLock;

use aeria_schema::{SchemaStore, column_names};
use serde::Serialize;
use tauri::Manager;

use crate::commands::run_blocking;
use crate::error::CommandError;
use crate::paths::AeriaPaths;
use crate::state::DesktopState;

type CommandResult<T> = Result<T, CommandError>;

const CACHE_DIRECTORY: &str = "exdschema";

/// The schema store, made on first use.
#[derive(Default)]
pub struct ColumnNames {
    store: OnceLock<Result<SchemaStore, String>>,
}

impl ColumnNames {
    fn store(&self, app: &tauri::AppHandle) -> CommandResult<&SchemaStore> {
        self.store
            .get_or_init(|| {
                let cache = app
                    .aeria_cache_dir()
                    .map_err(|error| error.to_string())?
                    .join(CACHE_DIRECTORY);
                SchemaStore::new(cache).map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|message| CommandError::new("columnNames", message.clone()))
    }
}

/// The names of a sheet's columns by column index, and the game version of
/// the EXDSchema branch that named them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetColumnNamesDto {
    pub schema_version: String,
    pub names: Vec<String>,
}

#[tauri::command(rename_all = "camelCase")]
/// Names the columns of a sheet of the open project from EXDSchema. `None`
/// when EXDSchema has no schema for the sheet at the game's version, or its
/// schema declares another number of columns than the game's sheet header.
///
/// # Errors
///
/// Returns a typed command error when no project is open, a game file cannot
/// be read, or EXDSchema cannot be reached for a schema not in the cache.
pub async fn sheet_column_names(
    app: tauri::AppHandle,
    sheet_name: String,
) -> CommandResult<Option<SheetColumnNamesDto>> {
    let source = app.state::<DesktopState>().session()?.source_handle();
    let version = source.version().clone();
    let sheet = sheet_name.clone();
    let Some(columns) = run_blocking(move || Ok(source.sheet_columns(&sheet)?)).await? else {
        return Ok(None);
    };
    let store = app.state::<ColumnNames>();
    let fields = store
        .store(&app)?
        .sheet_fields(&version, &sheet_name)
        .await
        .map_err(|error| CommandError::new("columnNames", error.to_string()))?;
    Ok(fields.and_then(|fields| {
        column_names(fields.fields, &columns).map(|names| SheetColumnNamesDto {
            schema_version: fields.version,
            names,
        })
    }))
}
