//! Section files: Markdown where each `## key` heading starts one entry,
//! optionally followed by a metadata line
//! `<!-- aeria: name=value; … -->`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

pub(crate) const META_START: &str = "<!-- aeria:";

/// One `## key` section.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub key: String,
    /// Values of the metadata line under the heading.
    pub meta: BTreeMap<String, String>,
    pub text: String,
    /// One-based line of the heading.
    pub line: usize,
}

impl Section {
    #[must_use]
    pub fn new(key: &str, text: &str) -> Self {
        Self {
            key: key.trim().to_owned(),
            meta: BTreeMap::new(),
            text: text.trim().to_owned(),
            line: 0,
        }
    }

    /// A person decided the entry; agents do not change it without asking.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.meta.get("settled").is_some_and(|value| {
            matches!(value.to_ascii_lowercase().as_str(), "yes" | "true" | "1")
        })
    }
}

/// Reads a metadata line.
pub(crate) fn parse_meta(line: &str) -> Option<BTreeMap<String, String>> {
    let inner = line.trim().strip_prefix(META_START)?.strip_suffix("-->")?;
    Some(
        inner
            .split(';')
            .filter_map(|pair| {
                let (name, value) = pair.split_once('=')?;
                Some((name.trim().to_owned(), value.trim().to_owned()))
            })
            .collect(),
    )
}

/// Splits a section body into its metadata, from a metadata line before any
/// text, and its text.
pub(crate) fn split_meta<'a>(lines: &[&'a str]) -> (BTreeMap<String, String>, Vec<&'a str>) {
    let mut meta = BTreeMap::new();
    let mut body = Vec::new();
    for line in lines {
        match (
            body.iter().all(|line: &&str| line.trim().is_empty()),
            parse_meta(line),
        ) {
            (true, Some(values)) if meta.is_empty() => meta = values,
            _ => body.push(*line),
        }
    }
    (meta, body)
}

/// Reads the `## key` sections of a Markdown file. Text before the first
/// heading is for people and is ignored, as are sections without a key or
/// text; a later section with the same key replaces an earlier one.
#[must_use]
pub fn parse_sections(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut current: Option<(usize, String, Vec<&str>)> = None;
    let finish = |sections: &mut Vec<Section>, (line, key, lines): (usize, String, Vec<&str>)| {
        let (meta, body) = split_meta(&lines);
        let text = body.join("\n").trim().to_owned();
        if key.is_empty() || text.is_empty() {
            return;
        }
        sections.retain(|section| !section.key.eq_ignore_ascii_case(&key));
        sections.push(Section {
            key,
            meta,
            text,
            line,
        });
    };
    for (index, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(done) = current.take() {
                finish(&mut sections, done);
            }
            current = Some((index + 1, heading.trim().to_owned(), Vec::new()));
        } else if let Some((_, _, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    if let Some(done) = current.take() {
        finish(&mut sections, done);
    }
    sections
}

/// Writes sections in canonical form: a title, a heading per section, its
/// metadata line when it has metadata, its text, and a blank line between
/// sections.
#[must_use]
pub fn write_sections(title: &str, sections: &[Section]) -> String {
    let mut text = format!("# {title}\n");
    for section in sections {
        let _ = write!(text, "\n## {}\n", section.key);
        if !section.meta.is_empty() {
            let values: Vec<String> = section
                .meta
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            let _ = writeln!(text, "{META_START} {} -->", values.join("; "));
        }
        let _ = writeln!(text, "{}", section.text.trim());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_keep_metadata_and_the_last_duplicate() {
        let text = "# Style\nFor people.\n\n## journal\n<!-- aeria: settled=yes; note=x -->\nUse вы.\n\n## Journal\nUse ты.\n\n## empty\n\n## dialogue\nLive speech.\n";
        let sections = parse_sections(text);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].key, "Journal");
        assert_eq!(sections[0].text, "Use ты.");
        assert!(!sections[0].settled());
        assert_eq!(sections[1].key, "dialogue");
        let settled = parse_sections("## a\n<!-- aeria: settled=yes -->\nText\n");
        assert!(settled[0].settled());
        assert_eq!(settled[0].line, 1);
        let written = write_sections("Style", &settled);
        let reread = parse_sections(&written);
        assert_eq!(reread.len(), 1);
        assert_eq!(
            (&reread[0].key, &reread[0].meta, &reread[0].text),
            (&settled[0].key, &settled[0].meta, &settled[0].text)
        );
    }
}
