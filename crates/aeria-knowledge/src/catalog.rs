//! The independent glossary catalog. Sheet translations remain in PO files.

use crate::GlossaryEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const CATALOG_PATH: &str = "aeria-knowledge/glossary.json";
pub const CATALOG_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sheet {
    pub sheet: String,
    pub column: u32,
    pub category: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TermSource {
    pub sheet: String,
    pub column: u32,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Term {
    pub id: String,
    pub term: String,
    pub category: String,
    pub forms: Vec<String>,
    pub note: Option<String>,
    pub match_case: bool,
    pub sources: Vec<TermSource>,
    /// Present only for standalone terms. A sheet-backed term reads PO.
    pub translation: Option<String>,
    /// Preserved during migration and category renames.
    pub version_category: String,
}

impl Term {
    #[must_use]
    pub fn entry(&self, category: &Category, translation: String) -> GlossaryEntry {
        GlossaryEntry {
            version_category: Some(self.version_category.clone()),
            term: self.term.clone(),
            translation,
            forms: self.forms.clone(),
            note: self.note.clone(),
            category_note: (!category.note.is_empty()).then(|| category.note.clone()),
            match_case: self.match_case,
            category: category.name.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Catalog {
    pub format_version: u32,
    pub categories: Vec<Category>,
    pub sheets: Vec<Sheet>,
    pub terms: Vec<Term>,
}

impl Catalog {
    /// Reject unknown versions and ambiguous identity before any write.
    ///
    /// # Errors
    /// Returns the violated format or identity rule.
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != CATALOG_VERSION {
            return Err(format!(
                "unsupported glossary catalog version {}",
                self.format_version
            ));
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for category in &self.categories {
            if category.id.is_empty()
                || category.name.trim().is_empty()
                || !ids.insert(category.id.as_str())
                || !names.insert(category.name.trim().to_lowercase())
            {
                return Err("categories must have unique IDs and nonempty unique names".to_owned());
            }
        }
        let mut declarations = HashSet::new();
        for sheet in &self.sheets {
            if sheet.sheet.is_empty()
                || !ids.contains(sheet.category.as_str())
                || !declarations.insert((&sheet.sheet, sheet.column))
            {
                return Err("invalid or repeated glossary sheet".to_owned());
            }
        }
        let mut terms = HashSet::new();
        let mut forms = HashSet::new();
        let mut sources = HashSet::new();
        for term in &self.terms {
            if term.id.is_empty()
                || !terms.insert(&term.id)
                || term.term.trim().is_empty()
                || !ids.contains(term.category.as_str())
                || term.version_category.is_empty()
            {
                return Err(format!("invalid term identity: {:?}", term.term));
            }
            if term.sources.is_empty() != term.translation.is_some()
                || term
                    .translation
                    .as_ref()
                    .is_some_and(|t| t.trim().is_empty())
            {
                return Err(format!(
                    "term {:?}: only standalone terms store a translation",
                    term.term
                ));
            }
            let mut own = HashSet::new();
            let mut exact_forms = HashSet::new();
            for form in std::iter::once(&term.term).chain(&term.forms) {
                if !exact_forms.insert(form.trim()) {
                    return Err(format!("repeated form of {:?}", term.term));
                }
                let name = form.trim().to_lowercase();
                if !own.insert(name.clone()) {
                    continue;
                }
                if name.is_empty() || !forms.insert((&term.category, name)) {
                    return Err(format!("term {form:?} is already defined in this category"));
                }
            }
            for source in &term.sources {
                if source.source.trim().is_empty()
                    || !declarations.contains(&(&source.sheet, source.column))
                    || !sources.insert((
                        source.sheet.as_str(),
                        source.column,
                        source.source.to_lowercase(),
                    ))
                {
                    return Err(format!("invalid or repeated source of {:?}", term.term));
                }
            }
        }
        Ok(())
    }

    /// Parse a catalog without accepting unknown fields or versions.
    ///
    /// # Errors
    /// Returns a JSON syntax error or a catalog validation error.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() as u64 > crate::glossary::MAX_GLOSSARY_BYTES {
            return Err("glossary catalog exceeds 64 MiB".to_owned());
        }
        let catalog: Self = serde_json::from_str(text).map_err(|e| e.to_string())?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Serialize the validated catalog in canonical form.
    ///
    /// # Errors
    /// Returns a validation or serialization error.
    pub fn text(&self) -> Result<String, String> {
        self.validate()?;
        let text = serde_json::to_string_pretty(self)
            .map(|text| text + "\n")
            .map_err(|e| e.to_string())?;
        if text.len() as u64 > crate::glossary::MAX_GLOSSARY_BYTES {
            return Err("glossary catalog exceeds 64 MiB".to_owned());
        }
        Ok(text)
    }
}
