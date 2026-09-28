//! The speaker name a line of dialogue can start with.
//!
//! A quest or cutscene line that starts with `(-name-)` is shown with that
//! name in place of the speaking character's own, such as `(-???-)` for a
//! character not yet introduced or `(-Exuberant Newcomer-)`. The markers are
//! plain text in the string's bytes, not a macro; the game reads them only at
//! the very start of a string. The name between them is text the player
//! reads and may hold macros, such as `(-<i>Title</i>-)`.

use crate::syntax::{MacroString, Span, SyntaxKind};

/// Opens a speaker name at the start of a string.
pub const SPEAKER_OPEN: &str = "(-";
/// Closes a speaker name.
pub const SPEAKER_CLOSE: &str = "-)";

/// Where a string's speaker name is, as byte spans of its macro text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeakerName {
    /// The opening `(-`, at the start of the string.
    pub open: Span,
    /// The name between the markers.
    pub name: Span,
    /// The first `-)` after the opening marker in text outside macros.
    pub close: Span,
}

/// The speaker name `document` starts with, if any.
#[must_use]
pub fn speaker_name(document: &MacroString) -> Option<SpeakerName> {
    let nodes = document.nodes();
    let first = nodes.first()?;
    if !matches!(first.kind, SyntaxKind::Text(_))
        || first.span.start() != 0
        || !document.slice(first.span)?.starts_with(SPEAKER_OPEN)
    {
        return None;
    }
    let open = Span::new(0, SPEAKER_OPEN.len());
    for (index, node) in nodes.iter().enumerate() {
        if !matches!(node.kind, SyntaxKind::Text(_)) {
            continue;
        }
        let text = document.slice(node.span)?;
        let from = if index == 0 { SPEAKER_OPEN.len() } else { 0 };
        if let Some(at) = text.get(from..).and_then(|rest| rest.find(SPEAKER_CLOSE)) {
            let start = node.span.start() + from + at;
            return Some(SpeakerName {
                open,
                name: Span::new(open.end(), start),
                close: Span::new(start, start + SPEAKER_CLOSE.len()),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parse;

    fn spans(text: &str) -> Option<(&str, &str, usize)> {
        let document = parse(text);
        let speaker = speaker_name(&document)?;
        Some((
            &text[speaker.name.start()..speaker.name.end()],
            &text[speaker.close.end()..],
            speaker.open.start(),
        ))
    }

    #[test]
    fn a_line_starting_with_a_name_in_markers_has_a_speaker_name() {
        assert_eq!(
            spans("(-???-)Well, this is a surprise."),
            Some(("???", "Well, this is a surprise.", 0))
        );
        assert_eq!(
            spans("(-<i>An Introduction to the Heavens</i>-)Chapter one."),
            Some(("<i>An Introduction to the Heavens</i>", "Chapter one.", 0)),
            "the name may hold macros"
        );
        assert_eq!(spans("(--)Empty."), Some(("", "Empty.", 0)));
    }

    #[test]
    fn markers_elsewhere_are_text() {
        assert_eq!(spans("Damage (-<num $n2>%)"), None, "only at the start");
        assert_eq!(spans(" (-???-)Hello"), None);
        assert_eq!(spans("(-no end"), None);
        assert_eq!(spans("(-)"), None, "the markers do not overlap");
        assert_eq!(spans("<i>(-???-)</i>"), None);
    }
}
