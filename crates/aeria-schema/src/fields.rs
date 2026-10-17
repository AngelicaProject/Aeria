//! The fields an EXDSchema sheet schema declares, and the columns they name.

use thiserror::Error;
use yaml_rust2::{Yaml, YamlLoader};

/// More columns than a sheet header can hold: a schema that declares more
/// is refused rather than expanded.
const MAX_FIELDS: usize = u16::MAX as usize + 1;

/// Why a schema file names no columns.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum FieldsError {
    #[error("the schema is not YAML: {0}")]
    Yaml(String),
    #[error("the schema declares no fields")]
    NoFields,
    #[error("the schema is invalid: {0}")]
    Invalid(String),
}

/// The name of every column a sheet schema declares, in the schema's
/// order. An array is expanded element by element: `Item[0]`, or
/// `Reward[0].Count` for an array of structures.
///
/// # Errors
///
/// Returns an error when the text is not a sheet schema.
pub fn field_names(text: &str) -> Result<Vec<String>, FieldsError> {
    let documents =
        YamlLoader::load_from_str(text).map_err(|error| FieldsError::Yaml(error.to_string()))?;
    let fields = documents
        .first()
        .and_then(|document| document["fields"].as_vec())
        .filter(|fields| !fields.is_empty())
        .ok_or(FieldsError::NoFields)?;
    let mut names = Vec::new();
    for field in fields {
        push_field(field, "", &mut names)?;
    }
    Ok(names)
}

fn push_field(field: &Yaml, prefix: &str, names: &mut Vec<String>) -> Result<(), FieldsError> {
    let path = match (prefix, field["name"].as_str()) {
        ("", Some(name)) => name.to_owned(),
        (prefix, Some(name)) => format!("{prefix}.{name}"),
        // The one field of an array element names the element itself.
        (prefix, None) => prefix.to_owned(),
    };
    if path.is_empty() {
        return Err(FieldsError::Invalid("a field has no name".to_owned()));
    }
    if field["type"].as_str() != Some("array") {
        names.push(path);
        return Ok(());
    }
    let count = field["count"]
        .as_i64()
        .and_then(|count| usize::try_from(count).ok())
        .filter(|count| (1..=MAX_FIELDS).contains(count))
        .ok_or_else(|| FieldsError::Invalid(format!("the array {path} has no count")))?;
    let inner = match &field["fields"] {
        Yaml::BadValue => None,
        Yaml::Array(inner) if !inner.is_empty() => Some(inner),
        _ => {
            return Err(FieldsError::Invalid(format!(
                "the array {path} has invalid fields"
            )));
        }
    };
    for index in 0..count {
        let element = format!("{path}[{index}]");
        match inner {
            None => names.push(element),
            Some(inner) => {
                for sub in inner {
                    push_field(sub, &element, names)?;
                }
            }
        }
        if names.len() > MAX_FIELDS {
            return Err(FieldsError::Invalid(format!(
                "the array {path} is too large"
            )));
        }
    }
    Ok(())
}

/// Names the columns of a sheet from the fields of its schema. `columns`
/// are the sheet header's columns in column order, each as its type code
/// and its byte offset in the row.
///
/// EXDSchema declares a sheet's fields in the order of the columns' offsets
/// in the row, the bits of one byte of packed booleans in bit order, not in
/// the order of column indexes: `PlaceName` declares `Name`,
/// `NameNoArticle`, `NamePrepositional`, the columns 0, 2, and 8 at offsets
/// 0, 4, and 8. The result is the name of each column by its index, or
/// `None` when the schema declares another number of columns than the
/// header has, so a schema of another game version names nothing.
#[must_use]
pub fn column_names(fields: Vec<String>, columns: &[(u16, u16)]) -> Option<Vec<String>> {
    if fields.len() != columns.len() {
        return None;
    }
    let mut order: Vec<usize> = (0..columns.len()).collect();
    order.sort_by_key(|&index| {
        let (code, offset) = columns[index];
        (offset, packed_bool_bit(code))
    });
    let mut names = vec![String::new(); columns.len()];
    for (name, index) in fields.into_iter().zip(order) {
        names[index] = name;
    }
    Some(names)
}

/// The bit of a packed boolean column (type codes `0x19` to `0x20`), 0 for
/// any other column.
const fn packed_bool_bit(code: u16) -> u16 {
    if code >= 0x19 && code <= 0x20 {
        code - 0x19
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrays_expand_element_by_element() {
        let schema = "name: Example
fields:
  - name: Name
  - name: Item
    type: array
    count: 2
  - name: Reward
    type: array
    count: 2
    fields:
      - name: Id
        type: link
        targets: [Item]
      - name: Count
  - name: Level
    type: array
    count: 2
    fields:
      - type: link
        targets: [ClassJob]
";
        assert_eq!(
            field_names(schema).expect("fields"),
            [
                "Name",
                "Item[0]",
                "Item[1]",
                "Reward[0].Id",
                "Reward[0].Count",
                "Reward[1].Id",
                "Reward[1].Count",
                "Level[0]",
                "Level[1]",
            ]
        );
    }

    #[test]
    fn nested_arrays_keep_every_index() {
        let schema = "name: Example
fields:
  - name: Grid
    type: array
    count: 2
    fields:
      - type: array
        count: 2
";
        assert_eq!(
            field_names(schema).expect("fields"),
            ["Grid[0][0]", "Grid[0][1]", "Grid[1][0]", "Grid[1][1]"]
        );
    }

    #[test]
    fn a_schema_without_fields_or_counts_names_nothing() {
        assert_eq!(field_names("name: Example"), Err(FieldsError::NoFields));
        assert!(matches!(
            field_names("fields:\n  - name: A\n    type: array\n"),
            Err(FieldsError::Invalid(_))
        ));
        assert!(matches!(
            field_names("fields:\n  - type: link\n"),
            Err(FieldsError::Invalid(_))
        ));
        assert!(matches!(
            field_names("fields: ["),
            Err(FieldsError::Yaml(_))
        ));
    }

    #[test]
    fn columns_are_named_in_the_order_of_their_offsets() {
        // PlaceName of the game: the String columns 0, 2, and 8 come first
        // in the row, then the bytes of column 1 and 3 to 7.
        let columns = [
            (0x0, 0),
            (0x2, 12),
            (0x0, 4),
            (0x2, 13),
            (0x2, 14),
            (0x2, 15),
            (0x2, 16),
            (0x2, 17),
            (0x0, 8),
            (0x3, 22),
            (0x5, 20),
            (0x3, 23),
        ];
        let fields = [
            "Name",
            "NameNoArticle",
            "NamePrepositional",
            "Adjective",
            "PossessivePronoun",
            "StartsWithVowel",
            "Countability",
            "Pronoun",
            "Article",
            "Order",
            "Unknown8",
            "MapCondition",
        ]
        .map(str::to_owned)
        .to_vec();
        let names = column_names(fields, &columns).expect("names");
        assert_eq!(names[0], "Name");
        assert_eq!(names[1], "Adjective");
        assert_eq!(names[2], "NameNoArticle");
        assert_eq!(names[8], "NamePrepositional");
        assert_eq!(names[10], "Order");
        assert_eq!(names[9], "Unknown8");
    }

    #[test]
    fn packed_booleans_of_one_byte_follow_their_bits() {
        let columns = [(0x1A, 4), (0x19, 4), (0x7, 0)];
        let fields = ["Id", "First", "Second"].map(str::to_owned).to_vec();
        assert_eq!(
            column_names(fields, &columns).expect("names"),
            ["Second", "First", "Id"]
        );
    }

    #[test]
    fn a_schema_of_another_column_count_names_nothing() {
        let fields = ["Name"].map(str::to_owned).to_vec();
        assert_eq!(column_names(fields, &[(0x0, 0), (0x0, 4)]), None);
    }
}
