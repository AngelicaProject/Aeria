//! Two checks of an answer that only a run can make: that a translation
//! belongs to its string, and that a short interface string fits the space the
//! game gives it.
//!
//! A request numbers its strings, and in a long run of short, similar strings
//! the model can slip and answer one string with the next one's translation.
//! Each answer therefore repeats the first words of its source; an answer
//! whose words are not the start of its own source is refused. The game's
//! interface is laid out for the official localizations, so a short label is
//! given the length of the longest of them (English, German, French) and
//! refused when longer.

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

/// The sheet of a PO path: `Addon/1000.po` and `BaseParam.po` give `Addon`
/// and `BaseParam`; a split sheet's folder ends with `~`.
#[must_use]
pub fn sheet_of(path: &str) -> &str {
    let first = path.split('/').next().unwrap_or(path);
    first
        .strip_suffix(".po")
        .unwrap_or(first)
        .trim_end_matches('~')
}

/// Calls `visit` with each character of macro text and whether it is part of
/// a macro tag. A tag runs from `<` to the `>` that closes it outside
/// parentheses and quotes, so `<if ($n1 > 0)>` and
/// `<sheet Addon 1 "<noun-en …>">` are one tag each.
fn scan(text: &str, mut visit: impl FnMut(char, bool)) {
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

/// The longest a translation of a short interface string may be: the length
/// of the longest of its source and the German and French lines of its
/// context (`de: …`, `fr: …`). `None` for other strings.
#[must_use]
pub fn length_budget(path: &str, source: &str, context: &[String]) -> Option<usize> {
    if !INTERFACE_SHEETS.contains(&sheet_of(path)) || source.contains("<br>") {
        return None;
    }
    let own = visible_length(source);
    if own == 0 || own > SHORT {
        return None;
    }
    let others = context.iter().filter_map(|line| {
        line.strip_prefix("de: ")
            .or_else(|| line.strip_prefix("fr: "))
            .map(visible_length)
    });
    Some(others.fold(own, usize::max))
}

/// Whether `start`, the words an answer repeats, are words of the string's
/// source, comparing letters and digits only, case ignored. The words may
/// come from inside its macros or after a conditional part the model left
/// out, so they need not begin the source; another string's words do not
/// occur in it. An empty `start` is accepted only for a source that begins
/// with a macro, whose first words the model cannot tell.
#[must_use]
pub fn matches_start(start: &str, source: &str) -> bool {
    let all = letters(source, true);
    if all.is_empty() {
        return true;
    }
    let start = letters(start, true);
    if start.is_empty() {
        return source
            .trim_start_matches(|c: char| c.is_whitespace())
            .starts_with('<');
    }
    all.contains(&start)
}

/// The lowercase letters and digits of macro text, with or without those
/// inside macro tags.
fn letters(text: &str, with_macros: bool) -> String {
    let mut out = String::new();
    scan(text, |c, in_tag| {
        if (with_macros || !in_tag) && c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheets_come_from_paths() {
        assert_eq!(sheet_of("Addon/1000.po"), "Addon");
        assert_eq!(sheet_of("BaseParam.po"), "BaseParam");
        assert_eq!(sheet_of("Leve~/0.po"), "Leve");
        assert_eq!(sheet_of("quest/000/Test.po"), "quest");
    }

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

    #[test]
    fn short_interface_strings_get_the_longest_official_length() {
        let context = vec![
            "ja: 戦利品".to_owned(),
            "de: Beutegut".to_owned(),
            "fr: Butin".to_owned(),
        ];
        assert_eq!(length_budget("Addon/1000.po", "Loot", &context), Some(8));
        assert_eq!(
            length_budget("BaseParam.po", "Direct Hit Rate", &[]),
            Some(15)
        );
        assert_eq!(length_budget("Item/0.po", "Loot", &context), None);
        assert_eq!(length_budget("Addon/1000.po", "Line<br>break", &[]), None);
        assert_eq!(
            length_budget("Addon/1000.po", &"word ".repeat(10), &[]),
            None
        );
    }

    #[test]
    fn an_answer_must_repeat_the_start_of_its_own_source() {
        assert!(matches_start("Loot", "Loot"));
        assert!(matches_start("Display Rules", "<icon2 11> Display Rules"));
        assert!(matches_start(
            "you tell thancred",
            "You tell Thancred and Alisaie of your reunion."
        ));
        assert!(matches_start("Type 1:", "Type 1: Ignore Depth"));
        assert!(!matches_start("Play Style", "Loot"));
        assert!(!matches_start("Objective", "None"));
        assert!(!matches_start("", "Loot"));
        assert!(matches_start("", "<icon 77>"));
        assert!(matches_start("!", "!"));
        assert!(matches_start(
            "Crystalline Conflict",
            "<if ($n1 > 0)>Crystalline Conflict<else>???</if>"
        ));
        assert!(matches_start(
            "HP:",
            "<sheet ActStr $n1 0><br><if $n2>Lv. <num $n2> </if>HP: <num $n3>"
        ));
        assert!(matches_start(
            "Average-sized",
            "\u{3000}<if ($n2 > 0)>Average-sized<else>Large-sized</if> = <num $n1> pts"
        ));
        assert!(matches_start(
            "<capitalize>",
            "<capitalize><sheet ActStr $n1 0></capitalize><br>MP: <num $n3>"
        ));
        assert!(matches_start(
            "",
            r#"<sheet Addon 15955 0 "<noun-en PlaceName 2 $n18 2 1>" $n14 $n12>"#
        ));
    }
}
