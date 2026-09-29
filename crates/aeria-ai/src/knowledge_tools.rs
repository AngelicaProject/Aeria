//! Angelica's tools for the project knowledge: `get_knowledge` reads both
//! layers, and `set_knowledge` writes the agent layer at once (see
//! [`crate::knowledge`]). Human entries are shown as locked and are never
//! changed; Angelica proposes changes to them with the glossary, guidance,
//! and voice tools, which wait for the user.

use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::chat::ToolDefinition;
use crate::guidance::GlossaryEntry;
use crate::knowledge::{
    Domain, Knowledge, KnowledgeFile, Lesson, LessonStatus, Section, set_characters, set_lesson,
    set_section, set_terms,
};
use crate::tools::ToolError;

/// Entries of one kind `get_knowledge` returns, at most.
const MAX_LISTED: usize = 100;
/// Entries one `set_knowledge` call writes, at most, per kind.
const MAX_SET: usize = 50;

/// The read tool, offered in every mode.
#[must_use]
pub fn read_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "get_knowledge",
        description: "The project knowledge the localizer and its critics use: style per kind of text, terms, character profiles, the story so far per quest, and lessons learned. Human entries (guidance, glossary, voices) are marked locked; the others are written by agents in aeria-knowledge/.",
        parameters: json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["style", "terms", "characters", "story", "lessons"], "description": "Omit for counts of every kind and the style." },
                "query": { "type": "string", "description": "Only entries whose key or text contains this, ignoring case." },
            },
            "additionalProperties": false,
        }),
    }]
}

/// The write tool, offered in Ask and Auto-draft modes.
#[must_use]
pub fn write_definitions() -> Vec<ToolDefinition> {
    let domains: Vec<&str> = Domain::ALL.iter().map(|domain| domain.as_str()).collect();
    vec![ToolDefinition {
        name: "set_knowledge",
        description: "Writes the agent layer of the project knowledge at once, without waiting for the user: style entries per kind of text, terms, character profiles, and lessons. Use it for decisions the user made in this conversation, such as a style chosen from variants, and for corrections the user asked for. Entries of the human files are never changed; the result lists them as skipped.",
        parameters: json!({
            "type": "object",
            "properties": {
                "style": { "type": "array", "maxItems": MAX_SET, "items": { "type": "object", "properties": {
                    "domain": { "type": "string", "enum": domains },
                    "text": { "type": "string", "description": "The whole style entry of the domain, as rules with short examples; it replaces the current entry." },
                }, "required": ["domain", "text"], "additionalProperties": false } },
                "terms": { "type": "array", "maxItems": MAX_SET, "items": { "type": "object", "properties": {
                    "term": { "type": "string" },
                    "translation": { "type": "string" },
                    "note": { "type": "string" },
                    "forbidden": { "type": "array", "items": { "type": "string" } },
                }, "required": ["term", "translation"], "additionalProperties": false } },
                "characters": { "type": "array", "maxItems": MAX_SET, "items": { "type": "object", "properties": {
                    "speakers": { "type": "array", "minItems": 1, "items": { "type": "string" }, "description": "Speaker labels, such as URIANGER." },
                    "text": { "type": "string", "description": "The whole profile; it replaces the current one." },
                }, "required": ["speakers", "text"], "additionalProperties": false } },
                "lessons": { "type": "array", "maxItems": MAX_SET, "items": { "type": "object", "properties": {
                    "id": { "type": "string", "description": "A short kebab-case identifier; an existing one is replaced." },
                    "text": { "type": "string" },
                    "domain": { "type": "string", "enum": domains },
                    "status": { "type": "string", "enum": ["active", "trial", "dropped"], "description": "active (the default) for a decision of the user; dropped to stop using a lesson." },
                }, "required": ["id", "text"], "additionalProperties": false } },
            },
            "additionalProperties": false,
        }),
    }]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetArgs {
    kind: Option<String>,
    query: Option<String>,
}

fn matches(query: Option<&str>, parts: &[&str]) -> bool {
    query.is_none_or(|query| {
        let query = query.to_lowercase();
        parts
            .iter()
            .any(|part| part.to_lowercase().contains(&query))
    })
}

fn sections(sections: &[Section], query: Option<&str>) -> Vec<Value> {
    sections
        .iter()
        .filter(|section| matches(query, &[&section.key, &section.text]))
        .take(MAX_LISTED)
        .map(|section| json!({ "key": section.key, "meta": section.meta, "text": section.text }))
        .collect()
}

/// Runs `get_knowledge` over the knowledge of a repository.
///
/// # Errors
///
/// Returns an error for invalid arguments.
pub fn get_knowledge(root: &Path, arguments: &str) -> Result<Value, ToolError> {
    let args: GetArgs = crate::tools::parse(arguments)?;
    let knowledge = Knowledge::load(root);
    let query = args
        .query
        .as_deref()
        .filter(|query| !query.trim().is_empty());
    let value = match args.kind.as_deref() {
        None => json!({
            "counts": {
                "style": knowledge.style.len(),
                "terms": knowledge.terms.len() + knowledge.guide.glossary.as_ref().map_or(0, |glossary| glossary.entries.len()),
                "characters": knowledge.characters.profiles.len() + knowledge.guide.voices.as_ref().map_or(0, |voices| voices.profiles.len()),
                "story": knowledge.story.len(),
                "lessons": knowledge.lessons.len(),
            },
            "guidance": knowledge.guide.guidance_for_prompt(),
            "style": sections(&knowledge.style, query),
            "problems": knowledge.problems,
        }),
        Some("style") => {
            json!({ "guidance": knowledge.guide.guidance_for_prompt(), "style": sections(&knowledge.style, query) })
        }
        Some("story") => json!({ "story": sections(&knowledge.story, query) }),
        Some("lessons") => json!({ "lessons": knowledge.lessons.iter()
            .filter(|lesson| matches(query, &[&lesson.id, &lesson.text]))
            .take(MAX_LISTED)
            .map(|lesson| json!({ "id": lesson.id, "status": lesson.status.as_str(), "domain": lesson.domain.map(Domain::as_str), "text": lesson.text, "meta": lesson.meta }))
            .collect::<Vec<_>>() }),
        Some("terms") => {
            let human = knowledge
                .guide
                .glossary
                .as_ref()
                .map(|glossary| glossary.entries.as_slice())
                .unwrap_or_default();
            let entry = |entry: &GlossaryEntry, locked: bool| json!({ "term": entry.term, "translation": entry.translation, "note": entry.note, "forbidden": entry.forbidden, "locked": locked });
            let listed: Vec<Value> = human
                .iter()
                .map(|found| (found, true))
                .chain(
                    knowledge
                        .terms
                        .iter()
                        .filter(|own| {
                            !human
                                .iter()
                                .any(|known| known.term.eq_ignore_ascii_case(&own.term))
                        })
                        .map(|found| (found, false)),
                )
                .filter(|(found, _)| matches(query, &[&found.term, &found.translation]))
                .take(MAX_LISTED)
                .map(|(found, locked)| entry(found, locked))
                .collect();
            json!({ "terms": listed })
        }
        Some("characters") => {
            let human = knowledge
                .guide
                .voices
                .as_ref()
                .map(|voices| voices.profiles.as_slice())
                .unwrap_or_default();
            let listed: Vec<Value> = human
                .iter()
                .map(|profile| (profile, true))
                .chain(knowledge.characters.profiles.iter().map(|profile| (profile, false)))
                .filter(|(profile, _)| matches(query, &[&profile.speakers.join(" "), &profile.text]))
                .take(MAX_LISTED)
                .map(|(profile, locked)| json!({ "speakers": profile.speakers, "text": profile.text, "locked": locked }))
                .collect();
            json!({ "characters": listed })
        }
        Some(other) => return Err(ToolError::new(format!("unknown kind {other:?}"))),
    };
    Ok(value)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleItem {
    domain: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TermItem {
    term: String,
    translation: String,
    note: Option<String>,
    #[serde(default)]
    forbidden: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterItem {
    speakers: Vec<String>,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LessonItem {
    id: String,
    text: String,
    domain: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetArgs {
    #[serde(default)]
    style: Vec<StyleItem>,
    #[serde(default)]
    terms: Vec<TermItem>,
    #[serde(default)]
    characters: Vec<CharacterItem>,
    #[serde(default)]
    lessons: Vec<LessonItem>,
}

/// Runs `set_knowledge` on the agent layer of a repository.
///
/// # Errors
///
/// Returns an error for invalid arguments or when a file cannot be written.
pub fn set_knowledge(root: &Path, arguments: &str) -> Result<Value, ToolError> {
    let args: SetArgs = crate::tools::parse(arguments)?;
    if args.style.len() > MAX_SET
        || args.terms.len() > MAX_SET
        || args.characters.len() > MAX_SET
        || args.lessons.len() > MAX_SET
    {
        return Err(ToolError::new(format!(
            "write at most {MAX_SET} entries of each kind at once"
        )));
    }
    let mut style = 0;
    for item in &args.style {
        let domain = Domain::parse(&item.domain)
            .ok_or_else(|| ToolError::new(format!("unknown domain {:?}", item.domain)))?;
        set_section(
            root,
            KnowledgeFile::Style,
            Section::new(domain.as_str(), &item.text).with("source", "angelica"),
        )
        .map_err(ToolError::new)?;
        style += 1;
    }
    let entries: Vec<GlossaryEntry> = args
        .terms
        .into_iter()
        .map(|item| GlossaryEntry {
            term: item.term,
            translation: item.translation,
            note: item.note.filter(|note| !note.trim().is_empty()),
            forbidden: item.forbidden,
        })
        .collect();
    let written = set_terms(root, &entries, true).map_err(ToolError::new)?;
    let skipped_terms: Vec<&str> = entries
        .iter()
        .map(|entry| entry.term.as_str())
        .filter(|term| !written.iter().any(|done| done.eq_ignore_ascii_case(term)))
        .collect();
    let profiles: Vec<(Vec<String>, String)> = args
        .characters
        .into_iter()
        .map(|item| (item.speakers, item.text))
        .collect();
    let characters = set_characters(root, &profiles).map_err(ToolError::new)?;
    let mut lessons = 0;
    for item in args.lessons {
        let status = match item.status.as_deref() {
            None | Some("active") => LessonStatus::Active,
            Some("trial") => LessonStatus::Trial,
            Some("dropped") => LessonStatus::Dropped,
            Some(other) => return Err(ToolError::new(format!("unknown status {other:?}"))),
        };
        let mut meta = std::collections::BTreeMap::new();
        meta.insert("source".to_owned(), "angelica".to_owned());
        set_lesson(
            root,
            &Lesson {
                id: item.id.trim().to_owned(),
                status,
                domain: item.domain.as_deref().and_then(Domain::parse),
                text: item.text,
                meta,
            },
        )
        .map_err(ToolError::new)?;
        lessons += 1;
    }
    Ok(json!({
        "written": { "style": style, "terms": written.len(), "characters": characters, "lessons": lessons },
        "skippedHumanTerms": skipped_terms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angelica_writes_the_agent_layer_and_reads_both() {
        let directory = tempfile::tempdir().expect("temp");
        let root = directory.path();
        std::fs::write(
            root.join("aeria-glossary.csv"),
            "term,translation\nAether,Эфир\n",
        )
        .expect("glossary");
        let written = set_knowledge(
            root,
            r#"{"style": [{"domain": "journal", "text": "Use вы."}],
                "terms": [{"term": "Aether", "translation": "эфир-агент"}, {"term": "Fire Shard", "translation": "огненный осколок"}],
                "characters": [{"speakers": ["LYNGSATH"], "text": "Rough cook."}],
                "lessons": [{"id": "no-hm", "text": "Never open with Хм.", "domain": "dialogue"}]}"#,
        )
        .expect("set");
        assert_eq!(
            written["written"],
            json!({ "style": 1, "terms": 1, "characters": 1, "lessons": 1 })
        );
        assert_eq!(written["skippedHumanTerms"], json!(["Aether"]));

        let terms = get_knowledge(root, r#"{"kind": "terms"}"#).expect("terms");
        assert_eq!(terms["terms"][0]["locked"], json!(true));
        assert_eq!(terms["terms"][1]["translation"], json!("огненный осколок"));
        let lessons =
            get_knowledge(root, r#"{"kind": "lessons", "query": "хм"}"#).expect("lessons");
        assert_eq!(lessons["lessons"][0]["status"], json!("active"));
        let overview = get_knowledge(root, "{}").expect("overview");
        assert_eq!(overview["counts"]["characters"], json!(1));
        assert!(set_knowledge(root, r#"{"style": [{"domain": "poetry", "text": "x"}]}"#).is_err());
    }
}
