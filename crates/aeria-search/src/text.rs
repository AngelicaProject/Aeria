//! Plain text of macro strings and a language-neutral similarity measure.

use std::collections::BTreeMap;

use aeria_se::TextRangeKind;

/// The user-facing text of a macro string: its text ranges in order, with a
/// space where a macro separates two of them. Macros themselves are left out,
/// so searches match what players read.
#[must_use]
pub fn plain_text(macro_text: &str) -> String {
    let document = aeria_se::parse(macro_text);
    let analysis = document.semantic_analysis();
    let mut text = String::new();
    let mut last_end = None;
    for range in analysis.text_ranges() {
        let Some(slice) = document.slice(range.span) else {
            continue;
        };
        if last_end.is_some_and(|end| end != range.span.start())
            && !text.is_empty()
            && !text.ends_with(char::is_whitespace)
        {
            text.push(' ');
        }
        match range.kind {
            TextRangeKind::Text => text.push_str(slice),
            TextRangeKind::Escape => text.extend(slice.chars().last()),
        }
        last_end = Some(range.span.end());
    }
    text.trim().to_owned()
}

/// Whether a macro string's plain text contains `query`, ignoring case. An
/// empty query matches nothing.
#[must_use]
pub fn text_contains(macro_text: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    !query.is_empty() && plain_text(macro_text).to_lowercase().contains(&query)
}

/// Lowercased character bigrams with their counts; whitespace runs count as
/// one space so spacing differences do not matter.
fn bigrams(text: &str) -> BTreeMap<(char, char), u32> {
    let mut normalized = Vec::new();
    for character in text.chars().flat_map(char::to_lowercase) {
        if character.is_whitespace() {
            if normalized.last().is_some_and(|last: &char| *last != ' ') {
                normalized.push(' ');
            }
        } else {
            normalized.push(character);
        }
    }
    let mut counts = BTreeMap::new();
    for pair in normalized.windows(2) {
        *counts.entry((pair[0], pair[1])).or_insert(0) += 1;
    }
    counts
}

/// Similarity of two plain texts from 0 to 1: the Dice coefficient of their
/// lowercased character bigrams. It needs no word boundaries, so it works the
/// same for every language. Identical texts score 1, even single characters.
#[must_use]
pub fn similarity(left: &str, right: &str) -> f64 {
    if left.trim().to_lowercase() == right.trim().to_lowercase() {
        return 1.0;
    }
    let left = bigrams(left);
    let right = bigrams(right);
    let total: u32 = left.values().sum::<u32>() + right.values().sum::<u32>();
    if total == 0 {
        return 0.0;
    }
    let shared: u32 = left
        .iter()
        .map(|(pair, count)| right.get(pair).map_or(0, |other| (*count).min(*other)))
        .sum();
    f64::from(2 * shared) / f64::from(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_macros_and_keeps_words_apart() {
        assert_eq!(plain_text("Hello, <pcname(lnum1)>!"), "Hello, !");
        assert_eq!(plain_text("Line one<br>Line two"), "Line one Line two");
        assert_eq!(plain_text("  Plain  "), "Plain");
    }

    #[test]
    fn contains_ignores_case_and_macros() {
        assert!(text_contains("Hello, <pcname(lnum1)>!", "HELLO"));
        assert!(!text_contains("Hello, <pcname(lnum1)>!", "pcname"));
        assert!(!text_contains("Hello", "  "));
    }

    #[test]
    fn similarity_is_symmetric_and_bounded() {
        assert!((similarity("Fire Crystal", "fire crystal") - 1.0).abs() < f64::EPSILON);
        let close = similarity("Deliver the fire crystals", "Deliver the ice crystals");
        let far = similarity("Deliver the fire crystals", "Speak with the guard");
        assert!(close > 0.6, "{close}");
        assert!(far < close);
        assert!((similarity("abc", "xyz")).abs() < f64::EPSILON);
        assert!(
            (close - similarity("Deliver the ice crystals", "Deliver the fire crystals")).abs()
                < f64::EPSILON
        );
        assert!(similarity("炎のクリスタル", "氷のクリスタル") > 0.5);
    }
}
