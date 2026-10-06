//! How long a translation may be: a short interface string fits the space the
//! game lays out for the official localizations, and a name the game copies
//! into a character or object of the world is cut at a number of bytes.
//!
//! The game's interface is laid out for the official localizations, so a
//! short label is given the length of the longest of them (English, German,
//! French). A translation longer than its budget is advice for a person and
//! refused from machine translation.

use crate::identity::Identity;
use crate::po::Entry;

/// Sheets of interface labels, whose short strings must fit their space.
const INTERFACE_SHEETS: &[&str] = &[
    "Addon",
    "AddonTransient",
    "BaseParam",
    "ClassJob",
    "ClassJobCategory",
    "Completion",
    "ConfigKey",
    "ContentType",
    "ItemSearchCategory",
    "ItemUICategory",
    "MainCommand",
    "MainCommandCategory",
];

/// Strings longer than this, or with a line break, wrap or scroll; they get
/// no length.
const SHORT: usize = 40;

/// Sheets whose name (column 0) the game copies into a character or object
/// of the world, which shows it over its head and as a target: the sheets
/// the game's `ObjStr` names objects by (`GameObject_GetObjStrId`), except
/// `Mount`, which no object kind reaches.
const OBJECT_NAME_SHEETS: &[&str] = &[
    "Aetheryte",
    "BNpcName",
    "Companion",
    "ENpcResident",
    "EObjName",
    "GatheringPointName",
    "Treasure",
];

/// The longest name a world object shows whole. The game copies a name
/// into a 64-byte field ending with NUL (`GameObject_SetName`); a name of
/// 64 bytes or more keeps 63 and then loses its last character that is
/// not ASCII even when it fits, so a Cyrillic name is cut to 62 or less.
const OBJECT_NAME_BYTES: usize = 63;

/// What the length of a translation is counted in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unit {
    /// The characters it shows, macros not counted: an interface label.
    Characters,
    /// The bytes of its `SeString`: a name of a world object.
    Bytes,
}

/// The longest a translation may be.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budget {
    pub max: usize,
    pub unit: Unit,
}

impl Budget {
    /// The length of `text` counted in this budget's unit.
    #[must_use]
    pub fn length_of(self, text: &str) -> usize {
        length_in(self.unit, text)
    }
}

/// The length of `text` counted in `unit`. Text that does not encode is
/// counted by its UTF-8 bytes; its other checks fail.
#[must_use]
pub fn length_in(unit: Unit, text: &str) -> usize {
    match unit {
        Unit::Characters => visible_length(text),
        Unit::Bytes => aeria_se::codec::encode(text).map_or(text.len(), |bytes| bytes.len()),
    }
}

/// The longest the translation of `entry` may be: for the name of a world
/// object, the bytes the game shows whole; for a short interface string,
/// the characters of the longest of its source and the German and French
/// lines of its context (`de: …`, `fr: …`). `None` for other strings. The
/// sheet and column come from the entry's identity.
#[must_use]
pub fn length_budget(entry: &Entry) -> Option<Budget> {
    let identity = Identity::parse(&entry.context).ok()?;
    let sheet = identity.sheet.as_str();
    if identity.column == 0 && OBJECT_NAME_SHEETS.contains(&sheet) {
        return Some(Budget {
            max: OBJECT_NAME_BYTES,
            unit: Unit::Bytes,
        });
    }
    let source = &entry.source;
    if !INTERFACE_SHEETS.contains(&sheet) || source.contains("<br>") {
        return None;
    }
    let own = visible_length(source);
    if own == 0 || own > SHORT {
        return None;
    }
    let others = entry.extracted.iter().filter_map(|line| {
        line.strip_prefix("de: ")
            .or_else(|| line.strip_prefix("fr: "))
            .map(visible_length)
    });
    Some(Budget {
        max: others.fold(own, usize::max),
        unit: Unit::Characters,
    })
}

/// Calls `visit` with each character of macro text and whether it is part of
/// a macro tag. A tag runs from `<` to the `>` that closes it outside
/// parentheses and quotes, so `<if ($n1 > 0)>` and
/// `<sheet Addon 1 "<noun-en …>">` are one tag each.
pub fn scan(text: &str, mut visit: impl FnMut(char, bool)) {
    let mut in_tag = false;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for c in text.chars() {
        if !in_tag {
            if c == '<' {
                in_tag = true;
                depth = 0;
                quoted = false;
                visit(c, true);
            } else {
                visit(c, false);
            }
            continue;
        }
        visit(c, true);
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if !quoted {
            match c {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                '>' if depth == 0 => in_tag = false,
                _ => {}
            }
        }
    }
}

/// The characters a string shows: macros left out, `<nbsp>` one character,
/// `<shy>` none.
#[must_use]
pub fn visible_length(text: &str) -> usize {
    let mut count = 0;
    let mut tag = String::new();
    scan(text, |c, in_tag| {
        if in_tag {
            tag.push(c);
            if c == '>' && tag.starts_with('<') && is_closed(&tag) {
                if tag == "<nbsp>" {
                    count += 1;
                }
                tag.clear();
            }
        } else {
            tag.clear();
            count += 1;
        }
    });
    count
}

/// Whether a collected tag is complete: its parentheses and quotes closed.
fn is_closed(tag: &str) -> bool {
    let mut depth = 0i32;
    let mut quoted = false;
    for c in tag.chars() {
        match c {
            '"' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            _ => {}
        }
    }
    depth <= 0 && !quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_length_leaves_out_macros() {
        assert_eq!(visible_length("<icon2 11> Display Rules"), 14);
        assert_eq!(visible_length("Butin<nbsp>!"), 7);
        assert_eq!(visible_length("in<shy>struc<shy>tions"), 12);
        assert_eq!(visible_length("Шанс прям. удара"), 16);
        assert_eq!(
            visible_length("<if ($n1 > 0)>Crystalline Conflict<else>???</if>"),
            23
        );
        assert_eq!(
            visible_length(r#"<sheet Addon 15955 0 "<noun-en PlaceName 2 $n18 2 1>" $n14>"#),
            0
        );
    }

    fn entry(context: &str, source: &str, extracted: &[&str]) -> Entry {
        Entry {
            context: context.to_owned(),
            source: source.to_owned(),
            extracted: extracted.iter().map(|line| (*line).to_owned()).collect(),
            ..Entry::default()
        }
    }

    fn characters(max: usize) -> Budget {
        Budget {
            max,
            unit: Unit::Characters,
        }
    }

    #[test]
    fn short_interface_strings_get_the_longest_official_length() {
        let context = ["ja: 戦利品", "de: Beutegut", "fr: Butin"];
        let loot = entry("Addon:1:0:0", "Loot", &context);
        assert_eq!(length_budget(&loot), Some(characters(8)));
        assert_eq!(
            length_budget(&entry("BaseParam:1:0:0", "Direct Hit Rate", &[])),
            Some(characters(15))
        );
        assert_eq!(length_budget(&entry("Item:1:0:9", "Loot", &context)), None);
        assert_eq!(
            length_budget(&entry("Addon:1:0:0", "Line<br>break", &[])),
            None
        );
        assert_eq!(
            length_budget(&entry("Addon:1:0:0", &"word ".repeat(10), &[])),
            None
        );
        assert_eq!(length_budget(&entry("", "Loot", &context)), None);
    }

    #[test]
    fn names_of_world_objects_get_the_bytes_the_game_shows() {
        let budget = Budget {
            max: 63,
            unit: Unit::Bytes,
        };
        let bytes = Some(budget);
        let aide = entry(
            "ENpcResident:1019070:0:0",
            "East Aldenard Trading Company aide",
            &[],
        );
        assert_eq!(length_budget(&aide), bytes);
        assert_eq!(
            length_budget(&entry("BNpcName:2:0:0", "ruins runner", &[])),
            bytes
        );
        assert_eq!(
            length_budget(&entry(
                "ENpcResident:1019070:0:2",
                "East Aldenard Trading Company aides",
                &[]
            )),
            None,
            "the plural is not the object's name"
        );
        assert_eq!(
            length_budget(&entry("Mount:1:0:0", "company chocobo", &[])),
            None
        );
        let cut = "служащий торговой компании «Восточный Альденард»";
        assert_eq!(budget.length_of(cut), 92);
        assert_eq!(budget.length_of("East Aldenard Trading Company aide"), 34);
        assert_eq!(budget.length_of("Восточный<nbsp>Альденард"), 40);
    }
}
