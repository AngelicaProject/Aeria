//! Character voices: `aeria-knowledge/characters.md`, Voice Profiles Format
//! v1 (`docs/formats/voices-v1.md`). Each `## ` heading names one character
//! by the speaker labels of the game's dialogue keys, such as
//! `## URIANGER`, and the text below it describes how the character speaks
//! in the target language.

use serde::Serialize;

use crate::sections::split_meta;

/// Most speaker labels one profile heading may name.
pub const MAX_PROFILE_SPEAKERS: usize = 20;

/// One character's profile.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProfile {
    /// Speaker labels in upper case, in heading order.
    pub speakers: Vec<String>,
    pub text: String,
    /// A person decided the profile; agents do not change it without asking.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub settled: bool,
}

/// Why part of the file is ignored.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceDiagnostic {
    /// 1-based line of the heading.
    pub line: usize,
    pub message: String,
}

/// The profiles of a voices file and the parts that were ignored.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VoiceProfiles {
    pub profiles: Vec<VoiceProfile>,
    pub diagnostics: Vec<VoiceDiagnostic>,
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

/// Reads a voices file. Text before the first `## ` heading is for people
/// and is not used. A heading that is not a list of speaker labels, a
/// profile without text, and a label named by an earlier profile are
/// reported and ignored. A metadata line before the text may mark the
/// profile `settled=yes`.
#[must_use]
pub fn parse_voices(text: &str) -> VoiceProfiles {
    let text = text.trim_start_matches('\u{feff}');
    let mut sections: Vec<(usize, &str, Vec<&str>)> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(heading) = line.strip_prefix("## ") {
            sections.push((index + 1, heading, Vec::new()));
        } else if let Some((_, _, body)) = sections.last_mut() {
            body.push(line);
        }
    }
    let mut result = VoiceProfiles::default();
    for (line, heading, body) in sections {
        let labels = match heading_labels(heading) {
            Ok(labels) => labels,
            Err(message) => {
                result.diagnostics.push(VoiceDiagnostic { line, message });
                continue;
            }
        };
        let (meta, body) = split_meta(&body);
        let body = body.join("\n").trim().to_owned();
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
                settled: meta.get("settled").is_some_and(|value| {
                    matches!(value.to_ascii_lowercase().as_str(), "yes" | "true" | "1")
                }),
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

    /// Every speaker label with a profile, in file order.
    #[must_use]
    pub fn speakers(&self) -> Vec<&str> {
        self.profiles
            .iter()
            .flat_map(|profile| profile.speakers.iter().map(String::as_str))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\u{feff}# Voices\n\nNotes for people.\n\n## URIANGER\n<!-- aeria: settled=yes -->\nАрхаичная речь, «вы».\n\n### Examples\nAye → Воистину\n\n## alphinaud, ALPHINAUD_YOUNG\nВежливый, книжный.\n";

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
        assert!(urianger.settled);
        assert!(!voices.find("ALPHINAUD").expect("alphinaud").settled);
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
}
