//! Column names of the game's sheets from
//! [EXDSchema](https://github.com/xivdev/EXDSchema), for display.
//!
//! The game keeps no column names; EXDSchema is a community schema of every
//! sheet's fields. A schema names a sheet's columns only when it declares
//! as many fields as the sheet header has columns (see [`column_names`]).
//! The names are presentation: nothing about a string's identity, its
//! translation, or export depends on them.

#![forbid(unsafe_code)]

mod fields;
mod store;

pub use fields::{FieldsError, column_names, field_names};
pub use store::{Resolved, SchemaStore, SheetFields, StoreError, pick_version};
