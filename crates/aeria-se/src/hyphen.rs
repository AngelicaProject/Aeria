//! A read-only view of macro text with the game's non-breaking hyphen
//! written as an ordinary dash. All other syntax is preserved.

use std::borrow::Cow;
use std::ops::Range;

use crate::{ExprKind, MacroSyntax, SyntaxKind, SyntaxNode};

/// Whether a macro is the game's argument-free non-breaking hyphen.
#[must_use]
pub fn is_hyphen(syntax: &MacroSyntax) -> bool {
    syntax.code == 0x1F && syntax.args.is_empty()
}

/// Macro text with non-breaking hyphens normalized for search and terms.
/// Ranges in this view can be mapped back to the original macro text for
/// highlighting and replacement. This never changes the persisted string.
pub struct HyphenText<'a> {
    text: Cow<'a, str>,
    /// The dash's offset in the view and its complete original tag.
    hyphens: Vec<(usize, Range<usize>)>,
}

impl<'a> HyphenText<'a> {
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        let mut result = Self {
            text: Cow::Borrowed(source),
            hyphens: Vec::new(),
        };
        if !source.contains("hyphen") && !source.contains("code:1") {
            return result;
        }
        let document = crate::parse(source);
        if !document.is_well_formed() {
            return result;
        }
        let mut spans = Vec::new();
        collect(document.nodes(), &mut spans);
        if spans.is_empty() {
            return result;
        }
        let mut text = String::with_capacity(source.len());
        let mut copied = 0;
        for span in spans {
            text.push_str(&source[copied..span.start]);
            result.hyphens.push((text.len(), span.clone()));
            text.push('-');
            copied = span.end;
        }
        text.push_str(&source[copied..]);
        result.text = Cow::Owned(text);
        result
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn into_text(self) -> Cow<'a, str> {
        self.text
    }

    /// Maps byte boundaries in the view back to the source. A dash maps
    /// to the whole hyphen macro, including its angle brackets.
    #[must_use]
    pub fn original_range(&self, range: Range<usize>) -> Range<usize> {
        self.original_offset(range.start)..self.original_offset(range.end)
    }

    fn original_offset(&self, offset: usize) -> usize {
        offset
            + self
                .hyphens
                .iter()
                .take_while(|(dash, _)| *dash < offset)
                .map(|(_, span)| span.len() - 1)
                .sum::<usize>()
    }
}

fn collect(nodes: &[SyntaxNode], spans: &mut Vec<Range<usize>>) {
    for node in nodes {
        let SyntaxKind::Macro(syntax) = &node.kind else {
            continue;
        };
        if is_hyphen(syntax) {
            spans.push(node.span.start()..node.span.end());
        } else if let Some(spec) = syntax.spec {
            for (index, arg) in syntax.args.iter().enumerate() {
                if spec.is_translatable_arg(index)
                    && let ExprKind::Str(inner) = &arg.kind
                {
                    collect(inner, spans);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyphens_join_words_and_ranges_cover_the_original_tags() {
        let source = "Там<hyphen>Тара<hyphen>склеп!";
        let view = HyphenText::new(source);
        assert_eq!(view.as_str(), "Там-Тара-склеп!");
        let end = view.as_str().find('!').expect("punctuation");
        assert_eq!(
            &source[view.original_range(0..end)],
            source.trim_end_matches('!')
        );
        let dash = view.as_str().find('-').expect("dash");
        assert_eq!(&source[view.original_range(dash..dash + 1)], "<hyphen>");
    }

    #[test]
    fn only_actual_hyphens_in_translatable_text_are_normalized() {
        let source =
            r#"\<hyphen> <if $gn4>Tam<hyphen>Tara<else>plain</if> <sheet "x<hyphen>y" 1 0>"#;
        assert_eq!(
            HyphenText::new(source).as_str(),
            r#"\<hyphen> <if $gn4>Tam-Tara<else>plain</if> <sheet "x<hyphen>y" 1 0>"#
        );
        assert_eq!(HyphenText::new("Tam<hyphen").as_str(), "Tam<hyphen");
        assert_eq!(
            HyphenText::new("Tam<hyphen 1>Tara").as_str(),
            "Tam<hyphen 1>Tara"
        );
        assert_eq!(HyphenText::new("Tam<hyphen >Tara").as_str(), "Tam-Tara");
    }
}
