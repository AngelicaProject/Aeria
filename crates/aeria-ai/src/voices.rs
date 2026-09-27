//! Character voice profiles: `aeria-voices.md` at the repository root.
//!
//! Voice Profiles Format v1 (`docs/formats/voices-v1.md`): Markdown where
//! each `## ` heading names one character by the speaker labels of the
//! game's dialogue keys, such as `## URIANGER`, and the text below it
//! describes how the character speaks in the target language. The file is
//! optional, human-edited, and shared through Git like the glossary.

use std::fmt::Write as _;

use serde::Serialize;

/// Voice profile file name at the repository root.
pub const VOICES_FILE: &str = "aeria-voices.md";
/// Largest voice profile file read.
pub const MAX_VOICES_BYTES: u64 = 256 * 1024;
/// Longest profile text placed in a request.
pub const MAX_VOICE_PROMPT_CHARS: usize = 2000;
/// Most speaker labels one profile heading may name.
pub const MAX_PROFILE_SPEAKERS: usize = 20;

/// One character's profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProfile {
    /// Speaker labels in upper case, in heading order.
    pub speakers: Vec<String>,
    pub text: String,
}

/// Why part of the file is ignored.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDiagnostic {
    /// 1-based line of the heading.
    pub line: usize,
    pub message: String,
}

/// The profiles of a voice file and the parts that were ignored.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VoiceProfiles {
    pub profiles: Vec<VoiceProfile>,
    pub diagnostics: Vec<VoiceDiagnostic>,
}

/// A heading and its body as written in the file.
struct Section<'a> {
    /// 0-based line of the heading.
    line: usize,
    heading: &'a str,
    body: Vec<&'a str>,
}

fn sections(text: &str) -> (Vec<&str>, Vec<Section<'_>>) {
    let mut preamble = Vec::new();
    let mut sections: Vec<Section<'_>> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(heading) = line.strip_prefix("## ") {
            sections.push(Section {
                line: index,
                heading,
                body: Vec::new(),
            });
        } else if let Some(section) = sections.last_mut() {
            section.body.push(line);
        } else {
            preamble.push(line);
        }
    }
    (preamble, sections)
}

/// A speaker label: ASCII letters, digits, and `_`, with at least one
/// letter. Returned in upper case.
#[must_use]
pub fn speaker_label(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && text.bytes().any(|byte| byte.is_ascii_alphabetic()))
    .then(|| text.to_ascii_uppercase())
}

fn heading_labels(heading: &str) -> Result<Vec<String>, String> {
    let mut labels = Vec::new();
    for part in heading.split(',') {
        let label = speaker_label(part).ok_or_else(|| {
            format!(
                "{:?} is not a speaker label; name the character by the label in its string keys, such as URIANGER",
                part.trim()
            )
        })?;
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    if labels.len() > MAX_PROFILE_SPEAKERS {
        return Err(format!(
            "a profile names at most {MAX_PROFILE_SPEAKERS} speaker labels"
        ));
    }
    Ok(labels)
}

fn body_text(body: &[&str]) -> String {
    body.join("\n").trim().to_owned()
}

/// Reads a voice file. Text before the first `## ` heading is for people
/// and is not used. A heading that is not a list of speaker labels, a
/// profile without text, and a label named by an earlier profile are
/// reported and ignored.
#[must_use]
pub fn parse_voices(text: &str) -> VoiceProfiles {
    let text = text.trim_start_matches('\u{feff}');
    let mut result = VoiceProfiles::default();
    for section in sections(text).1 {
        let line = section.line + 1;
        let labels = match heading_labels(section.heading) {
            Ok(labels) => labels,
            Err(message) => {
                result.diagnostics.push(VoiceDiagnostic { line, message });
                continue;
            }
        };
        let body = body_text(&section.body);
        if body.is_empty() {
            result.diagnostics.push(VoiceDiagnostic {
                line,
                message: "the profile has no text".to_owned(),
            });
            continue;
        }
        let mut speakers = Vec::new();
        for label in labels {
            if result.find(&label).is_some() {
                result.diagnostics.push(VoiceDiagnostic {
                    line,
                    message: format!("{label} already has a profile above; ignored here"),
                });
            } else {
                speakers.push(label);
            }
        }
        if !speakers.is_empty() {
            result.profiles.push(VoiceProfile {
                speakers,
                text: body,
            });
        }
    }
    result
}

impl VoiceProfiles {
    /// The profile that names a speaker label, ignoring case.
    #[must_use]
    pub fn find(&self, speaker: &str) -> Option<&VoiceProfile> {
        let speaker = speaker.to_ascii_uppercase();
        self.profiles
            .iter()
            .find(|profile| profile.speakers.contains(&speaker))
    }

    /// The profiles of some speakers, each once, in the order first named.
    #[must_use]
    pub fn for_speakers<'a>(
        &self,
        speakers: impl IntoIterator<Item = &'a str>,
    ) -> Vec<&VoiceProfile> {
        let mut found: Vec<&VoiceProfile> = Vec::new();
        for speaker in speakers {
            if let Some(profile) = self.find(speaker)
                && !found.iter().any(|known| std::ptr::eq(*known, profile))
            {
                found.push(profile);
            }
        }
        found
    }

    /// Every speaker label with a profile, in file order.
    #[must_use]
    pub fn speakers(&self) -> Vec<&str> {
        self.profiles
            .iter()
            .flat_map(|profile| profile.speakers.iter().map(String::as_str))
            .collect()
    }
}

impl VoiceProfile {
    /// The profile's text bounded for a request.
    #[must_use]
    pub fn text_for_prompt(&self) -> String {
        if self.text.chars().count() <= MAX_VOICE_PROMPT_CHARS {
            return self.text.clone();
        }
        let cut: String = self.text.chars().take(MAX_VOICE_PROMPT_CHARS).collect();
        format!("{cut}\n[profile truncated; read the rest with get_voices]")
    }

    /// The profile as a prompt block.
    #[must_use]
    pub fn prompt_block(&self) -> String {
        let mut block = String::new();
        let _ = write!(
            block,
            "<voice speakers=\"{}\">\n{}\n</voice>",
            self.speakers.join(", "),
            self.text_for_prompt()
        );
        block
    }
}

/// Sets one character's profile, or removes speaker labels, and returns
/// the new file. Setting replaces the heading and text of the one profile
/// that names any of the labels, in place, or appends a new profile. The
/// file is written in canonical form: the text before the first profile and
/// each profile's text are kept, trimmed, and headings list labels in upper
/// case.
///
/// # Errors
///
/// Returns a description when the current file has ignored parts, which
/// must be fixed by hand first, when a label is invalid, when the labels
/// belong to more than one profile, or when a removed label has no profile.
pub fn change_voices(
    current: Option<&str>,
    set: Option<(&[String], &str)>,
    remove: &[String],
) -> Result<String, String> {
    let current = current.unwrap_or_default().trim_start_matches('\u{feff}');
    let parsed = parse_voices(current);
    if let Some(diagnostic) = parsed.diagnostics.first() {
        return Err(format!(
            "aeria-voices.md has an invalid profile (line {}: {}); it must be fixed by hand before Angelica can change voice profiles",
            diagnostic.line, diagnostic.message
        ));
    }
    let (preamble, sections) = sections(current);
    // Every section is valid here, so each maps to one parsed profile.
    let mut profiles: Vec<(Vec<String>, String)> = sections
        .iter()
        .map(|section| {
            (
                heading_labels(section.heading).unwrap_or_default(),
                body_text(&section.body),
            )
        })
        .collect();
    let mut removals = Vec::new();
    for label in remove {
        let label =
            speaker_label(label).ok_or_else(|| format!("{label:?} is not a speaker label"))?;
        let profile = profiles
            .iter_mut()
            .find(|(speakers, _)| speakers.contains(&label))
            .ok_or_else(|| format!("{label} has no voice profile"))?;
        profile.0.retain(|speaker| *speaker != label);
        removals.push(label);
    }
    if let Some((speakers, text)) = set {
        let text = text.trim();
        if text.is_empty() {
            return Err("a voice profile needs text".to_owned());
        }
        let labels = heading_labels(&speakers.join(","))?;
        if labels.iter().any(|label| removals.contains(label)) {
            return Err("a speaker label cannot be set and removed at once".to_owned());
        }
        let owners: Vec<usize> = profiles
            .iter()
            .enumerate()
            .filter(|(_, (existing, _))| existing.iter().any(|label| labels.contains(label)))
            .map(|(index, _)| index)
            .collect();
        match owners.as_slice() {
            [] => profiles.push((labels, text.to_owned())),
            [index] => profiles[*index] = (labels, text.to_owned()),
            _ => {
                return Err(
                    "those speaker labels belong to different profiles; change them one profile at a time"
                        .to_owned(),
                );
            }
        }
    }
    let mut file = String::new();
    let preamble = preamble.join("\n");
    let preamble = preamble.trim();
    if !preamble.is_empty() {
        file.push_str(preamble);
        file.push_str("\n\n");
    }
    for (speakers, text) in profiles.iter().filter(|(speakers, _)| !speakers.is_empty()) {
        let _ = write!(file, "## {}\n\n{text}\n\n", speakers.join(", "));
    }
    let file = file.trim_end().to_owned();
    if file.is_empty() {
        return Ok(String::new());
    }
    Ok(file + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\u{feff}# Voices\n\nNotes for people.\n\n## URIANGER\n\nАрхаичная речь, «вы».\n\n### Examples\nAye → Воистину\n\n## alphinaud, ALPHINAUD_YOUNG\nВежливый, книжный.\n";

    fn labels(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn profiles_are_found_by_speaker_label() {
        let voices = parse_voices(FILE);
        assert!(voices.diagnostics.is_empty(), "{:?}", voices.diagnostics);
        assert_eq!(
            voices.speakers(),
            ["URIANGER", "ALPHINAUD", "ALPHINAUD_YOUNG"]
        );
        let urianger = voices.find("urianger").expect("profile");
        assert_eq!(
            urianger.text,
            "Архаичная речь, «вы».\n\n### Examples\nAye → Воистину"
        );
        assert_eq!(
            voices
                .for_speakers(["ALPHINAUD_YOUNG", "SYSTEM", "ALPHINAUD", "URIANGER"])
                .len(),
            2
        );
        assert!(voices.find("SYSTEM").is_none());
    }

    #[test]
    fn invalid_headings_empty_profiles_and_repeated_labels_are_ignored() {
        let voices = parse_voices(
            "## Urianger Augurelle\nText.\n## THANCRED\n\n## Y'SHTOLA\nText.\n## CID\nOne.\n## CID, NERO\nTwo.\n",
        );
        let lines: Vec<usize> = voices.diagnostics.iter().map(|d| d.line).collect();
        assert_eq!(lines, [1, 3, 5, 9]);
        assert_eq!(voices.speakers(), ["CID", "NERO"]);
        assert_eq!(voices.find("CID").expect("cid").text, "One.");
        assert_eq!(voices.find("NERO").expect("nero").text, "Two.");
    }

    #[test]
    fn a_change_replaces_one_profile_in_place_and_keeps_the_rest() {
        let set = labels(&["urianger"]);
        let changed =
            change_voices(Some(FILE), Some((&set, "Высокий стиль.")), &[]).expect("changed");
        assert_eq!(
            changed,
            "# Voices\n\nNotes for people.\n\n## URIANGER\n\nВысокий стиль.\n\n## ALPHINAUD, ALPHINAUD_YOUNG\n\nВежливый, книжный.\n"
        );
        let added = change_voices(None, Some((&labels(&["THANCRED"]), " Ироничный. ")), &[])
            .expect("added");
        assert_eq!(added, "## THANCRED\n\nИроничный.\n");
        let removed = change_voices(
            Some(&changed),
            None,
            &labels(&["ALPHINAUD", "ALPHINAUD_YOUNG"]),
        )
        .expect("removed");
        assert!(!removed.contains("ALPHINAUD"));
        assert_eq!(parse_voices(&removed).speakers(), ["URIANGER"]);
    }

    #[test]
    fn unsafe_changes_are_refused() {
        let across = labels(&["URIANGER", "ALPHINAUD"]);
        assert!(change_voices(Some(FILE), Some((&across, "Text.")), &[]).is_err());
        assert!(change_voices(Some(FILE), Some((&labels(&["URIANGER"]), " ")), &[]).is_err());
        assert!(change_voices(Some(FILE), None, &labels(&["NOBODY"])).is_err());
        assert!(change_voices(Some(FILE), Some((&labels(&["Y'SHTOLA"]), "Text.")), &[]).is_err());
        assert!(
            change_voices(Some("## bad label\nText.\n"), None, &[]).is_err(),
            "a file with ignored parts is not rewritten"
        );
    }
}
