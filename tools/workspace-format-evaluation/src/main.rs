#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use aeria_core::{
    ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, TranslationUnit, TranslationUnitId,
    WorkspaceMetadata,
};

const SHARD_COUNT: usize = 256;
const DEFAULT_SCENARIO_SIZE: usize = 1_000;
const SMOKE_SIZE: usize = 64;
const GENERATOR_SEED: u64 = 0x9e37_79b9_7f4a_7c15;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Candidate {
    OneFilePerUnit,
    SingleJsonl,
    ShardedJsonl,
    ShardedPrettyJson,
}

impl Candidate {
    const ALL: [Self; 4] = [
        Self::OneFilePerUnit,
        Self::SingleJsonl,
        Self::ShardedJsonl,
        Self::ShardedPrettyJson,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::OneFilePerUnit => "one-file",
            Self::SingleJsonl => "single-jsonl",
            Self::ShardedJsonl => "sharded-jsonl",
            Self::ShardedPrettyJson => "sharded-pretty-json",
        }
    }
}

#[derive(Clone, Debug)]
struct Dataset {
    metadata: WorkspaceMetadata,
    units: Vec<TranslationUnit>,
}

#[derive(Clone, Copy, Debug)]
enum DiffScenario {
    ChangeTarget,
    ChangeReviewState,
    ChangeNote,
    ChangeSourceFacts,
    AddOne,
    DeleteOne,
    ChangeOneHundred,
    AddOneHundred,
}

impl DiffScenario {
    const ALL: [Self; 8] = [
        Self::ChangeTarget,
        Self::ChangeReviewState,
        Self::ChangeNote,
        Self::ChangeSourceFacts,
        Self::AddOne,
        Self::DeleteOne,
        Self::ChangeOneHundred,
        Self::AddOneHundred,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::ChangeTarget => "target",
            Self::ChangeReviewState => "review-state",
            Self::ChangeNote => "note",
            Self::ChangeSourceFacts => "source-facts",
            Self::AddOne => "add-one",
            Self::DeleteOne => "delete-one",
            Self::ChangeOneHundred => "change-100",
            Self::AddOneHundred => "add-100",
        }
    }

    const fn expected_changed_records(self) -> usize {
        match self {
            Self::ChangeTarget
            | Self::ChangeReviewState
            | Self::ChangeNote
            | Self::ChangeSourceFacts => 1,
            Self::AddOne | Self::DeleteOne | Self::AddOneHundred => 0,
            Self::ChangeOneHundred => 100,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum MergeScenario {
    IndependentDifferentShards,
    IndependentSameShard,
    IndependentNearbyIds,
    IndependentInsertsDifferentShards,
    IndependentInsertsSameShard,
    SameUnit,
    DeleteVersusEdit,
}

impl MergeScenario {
    const ALL: [Self; 7] = [
        Self::IndependentDifferentShards,
        Self::IndependentSameShard,
        Self::IndependentNearbyIds,
        Self::IndependentInsertsDifferentShards,
        Self::IndependentInsertsSameShard,
        Self::SameUnit,
        Self::DeleteVersusEdit,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::IndependentDifferentShards => "edit-different-shards",
            Self::IndependentSameShard => "edit-same-shard",
            Self::IndependentNearbyIds => "edit-nearby-ids",
            Self::IndependentInsertsDifferentShards => "insert-different-shards",
            Self::IndependentInsertsSameShard => "insert-same-shard",
            Self::SameUnit => "same-unit",
            Self::DeleteVersusEdit => "delete-versus-edit",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DiffResult {
    files_changed: usize,
    added_lines: usize,
    deleted_lines: usize,
    unrelated_records_reserialized: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MergeResult {
    outcome: &'static str,
    conflict_files: usize,
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let smoke = arguments.iter().any(|argument| argument == "--smoke");
    let scale_only = arguments.iter().any(|argument| argument == "--scale-only");
    let sizes = value_after(&arguments, "--sizes").map_or_else(
        || {
            if smoke {
                vec![SMOKE_SIZE]
            } else {
                vec![1_000, 50_000, 250_000]
            }
        },
        |value| parse_sizes(&value),
    );
    let scenario_size = value_after(&arguments, "--scenario-size")
        .map_or(if smoke { 128 } else { DEFAULT_SCENARIO_SIZE }, |value| {
            value.parse().expect("--scenario-size must be an integer")
        });

    println!("kind,candidate,size,files,bytes");
    for size in sizes {
        let dataset = generate_dataset(size);
        for candidate in Candidate::ALL {
            let temporary = TemporaryDirectory::new("scale");
            write_layout(temporary.path(), candidate, &dataset).expect("write scale layout");
            let (files, bytes) = file_stats(temporary.path()).expect("read scale layout");
            println!("scale,{},{size},{files},{bytes}", candidate.name());
        }
    }

    if scale_only {
        return;
    }

    let scenario_dataset = generate_dataset(scenario_size);
    println!(
        "kind,candidate,scenario,files_changed,added_lines,deleted_lines,unrelated_records_reserialized"
    );
    for candidate in Candidate::ALL {
        for scenario in DiffScenario::ALL {
            let result = measure_diff(candidate, &scenario_dataset, scenario);
            println!(
                "diff,{},{},{},{},{},{}",
                candidate.name(),
                scenario.name(),
                result.files_changed,
                result.added_lines,
                result.deleted_lines,
                result.unrelated_records_reserialized,
            );
        }
    }

    println!("kind,candidate,scenario,outcome,conflict_files");
    for candidate in Candidate::ALL {
        for scenario in MergeScenario::ALL {
            let result = measure_merge(candidate, &scenario_dataset, scenario);
            println!(
                "merge,{},{},{},{}",
                candidate.name(),
                scenario.name(),
                result.outcome,
                result.conflict_files,
            );
        }
    }
}

fn value_after(arguments: &[String], flag: &str) -> Option<String> {
    arguments
        .windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}

fn parse_sizes(value: &str) -> Vec<usize> {
    value
        .split(',')
        .map(|part| {
            part.parse()
                .expect("every --sizes value must be an integer")
        })
        .collect()
}

fn generate_dataset(size: usize) -> Dataset {
    let metadata = WorkspaceMetadata::new(
        "en",
        "fr",
        format!("sha256:{}", hex(&deterministic_bytes(0, 0xC0, 32))),
        format!("sha256:{}", hex(&deterministic_bytes(1, 0xD0, 32))),
    )
    .expect("evaluation metadata is valid");
    let mut units = (0..size)
        .map(|index| make_unit(u32::try_from(index).expect("evaluation size fits in u32")))
        .collect::<Vec<_>>();
    units.sort_by_key(TranslationUnit::id);
    Dataset { metadata, units }
}

fn make_unit(index: u32) -> TranslationUnit {
    let sheet_name = match index % 5 {
        0 => "Dialog",
        1 => "Item",
        2 => "Quest",
        3 => "翻訳表",
        _ => "Ui_Strings",
    };
    let binding = SourceBinding::new(
        sheet_name,
        index.wrapping_mul(17).wrapping_add(7),
        u16::try_from(index % 8).expect("modulo fits in u16"),
        index.wrapping_mul(5) % 32,
    );
    let fingerprint = SourceFingerprint::new(
        Sha256Hash::from_bytes(
            deterministic_bytes(index, 0x10, 32)
                .try_into()
                .expect("32 bytes"),
        ),
        index.is_multiple_of(3).then(|| {
            Sha256Hash::from_bytes(
                deterministic_bytes(index, 0x20, 32)
                    .try_into()
                    .expect("32 bytes"),
            )
        }),
        Sha256Hash::from_bytes(
            deterministic_bytes(index, 0x30, 32)
                .try_into()
                .expect("32 bytes"),
        ),
    );
    let id = TranslationUnitId::derive("en", &binding, &fingerprint)
        .expect("evaluation identity inputs fit v1 framing");
    let target = match index % 5 {
        0 => String::new(),
        1 => format!("Target text {index} — café"),
        2 => format!("{{If(Equal({index}, 1), \"… \\\\path\", \"line {index}\nnext\")}}"),
        3 => format!("Quote \"unit {index}\" and backslash \\\\"),
        _ => format!("Unicode 日本語 / Ελληνικά / 😀 {index}"),
    };
    let mut unit = TranslationUnit::new(id, binding, fingerprint, target);
    unit.set_review_state(match index % 3 {
        0 => ReviewState::Draft,
        1 => ReviewState::Reviewed,
        _ => ReviewState::NeedsReview,
    });
    if index.is_multiple_of(4) {
        unit.set_translator_note(Some(format!(
            "Note \"{index}\" with backslash \\\\ and newline\nline {index} — 東京"
        )));
    }
    unit
}

fn deterministic_bytes(index: u32, salt: u64, length: usize) -> Vec<u8> {
    let mut state = GENERATOR_SEED ^ u64::from(index).wrapping_mul(0x517c_c1b7_2722_0a95) ^ salt;
    let mut result = Vec::with_capacity(length);
    while result.len() < length {
        state ^= state >> 30;
        state = state.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        state ^= state >> 27;
        state = state.wrapping_mul(0x94d0_49bb_1331_11eb);
        state ^= state >> 31;
        result.extend_from_slice(&state.to_le_bytes());
    }
    result.truncate(length);
    result
}

fn hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").expect("writing hex to String cannot fail");
    }
    result
}

fn write_layout(root: &Path, candidate: Candidate, dataset: &Dataset) -> io::Result<()> {
    clear_layout(root)?;
    fs::create_dir_all(root)?;
    let data_root = root.join(".aeria");
    fs::create_dir_all(&data_root)?;
    fs::write(
        data_root.join("manifest.json"),
        manifest_json(&dataset.metadata),
    )?;
    match candidate {
        Candidate::OneFilePerUnit => write_one_file_layout(&data_root, &dataset.units),
        Candidate::SingleJsonl => write_single_jsonl(&data_root, &dataset.units),
        Candidate::ShardedJsonl => write_sharded_jsonl(&data_root, &dataset.units),
        Candidate::ShardedPrettyJson => write_sharded_pretty_json(&data_root, &dataset.units),
    }
}

fn clear_layout(root: &Path) -> io::Result<()> {
    match fs::remove_dir_all(root.join(".aeria")) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn manifest_json(metadata: &WorkspaceMetadata) -> String {
    let mut output = String::new();
    output.push_str("{\n");
    push_pretty_string_field(&mut output, 2, "formatVersion", "1", true);
    push_pretty_string_field(
        &mut output,
        2,
        "sourceLanguage",
        metadata.source_language(),
        true,
    );
    push_pretty_string_field(
        &mut output,
        2,
        "targetLanguage",
        metadata.target_language(),
        true,
    );
    push_pretty_string_field(
        &mut output,
        2,
        "contentId",
        metadata.source_content_id(),
        true,
    );
    push_pretty_string_field(
        &mut output,
        2,
        "snapshotId",
        metadata.source_snapshot_id(),
        false,
    );
    output.push_str("}\n");
    output
}

fn write_one_file_layout(root: &Path, units: &[TranslationUnit]) -> io::Result<()> {
    for unit in units {
        let path = unit_path(root, unit, "json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{}\n", record_json(unit)))?;
    }
    Ok(())
}

fn write_single_jsonl(root: &Path, units: &[TranslationUnit]) -> io::Result<()> {
    let file = File::create(root.join("units.jsonl"))?;
    let mut writer = BufWriter::new(file);
    for unit in units {
        writeln!(writer, "{}", record_json(unit))?;
    }
    writer.flush()
}

fn write_sharded_jsonl(root: &Path, units: &[TranslationUnit]) -> io::Result<()> {
    let shards = shard_units(units);
    for (shard, shard_units) in shards.iter().enumerate() {
        if shard_units.is_empty() {
            continue;
        }
        let path = root.join("units").join(format!("{shard:02x}.jsonl"));
        fs::create_dir_all(path.parent().expect("shard path has a parent"))?;
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        for unit in shard_units {
            writeln!(writer, "{}", record_json(unit))?;
        }
        writer.flush()?;
    }
    Ok(())
}

fn write_sharded_pretty_json(root: &Path, units: &[TranslationUnit]) -> io::Result<()> {
    let shards = shard_units(units);
    for (shard, shard_units) in shards.iter().enumerate() {
        if shard_units.is_empty() {
            continue;
        }
        let path = root.join("units").join(format!("{shard:02x}.json"));
        fs::create_dir_all(path.parent().expect("shard path has a parent"))?;
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(b"[\n")?;
        for (index, unit) in shard_units.iter().enumerate() {
            for line in pretty_record_json(unit).lines() {
                writeln!(writer, "  {line}")?;
            }
            if index + 1 != shard_units.len() {
                writer.write_all(b"  ,\n")?;
            }
        }
        writer.write_all(b"]\n")?;
        writer.flush()?;
    }
    Ok(())
}

fn shard_units(units: &[TranslationUnit]) -> Vec<Vec<&TranslationUnit>> {
    let mut shards = (0..SHARD_COUNT)
        .map(|_| Vec::new())
        .collect::<Vec<Vec<&TranslationUnit>>>();
    for unit in units {
        shards[usize::from(unit.id().as_bytes()[0])].push(unit);
    }
    shards
}

fn unit_path(root: &Path, unit: &TranslationUnit, extension: &str) -> PathBuf {
    let id = unit.id().to_string();
    let filename_id = id
        .strip_prefix("tu1:")
        .map(|digest| format!("tu1-{digest}"))
        .expect("evaluation IDs use the canonical tu1 prefix");
    root.join("units")
        .join(format!("{:02x}", unit.id().as_bytes()[0]))
        .join(format!("{filename_id}.{extension}"))
}

fn record_json(unit: &TranslationUnit) -> String {
    let binding = unit.source_binding();
    let fingerprint = unit.source_fingerprint();
    let mut output = String::new();
    output.push('{');
    push_compact_string_field(&mut output, "id", &unit.id().to_string(), true);
    output.push_str("\"sourceBinding\":{");
    push_compact_string_field(&mut output, "sheetName", binding.sheet_name(), true);
    push_compact_number_field(&mut output, "rowId", binding.row_id(), true);
    push_compact_number_field(&mut output, "subrowId", binding.subrow_id(), true);
    push_compact_number_field(&mut output, "columnIndex", binding.column_index(), false);
    output.push_str("},\"sourceFingerprint\":{");
    push_compact_string_field(
        &mut output,
        "macroTextHash",
        &fingerprint.macro_text_hash().to_hex(),
        true,
    );
    output.push_str("\"rawValueHash\":");
    match fingerprint.raw_value_hash() {
        Some(hash) => push_json_string(&mut output, &hash.to_hex()),
        None => output.push_str("null"),
    }
    output.push(',');
    push_compact_string_field(
        &mut output,
        "rowTechnicalHash",
        &fingerprint.row_technical_hash().to_hex(),
        false,
    );
    output.push_str("},");
    push_compact_string_field(&mut output, "targetMacro", unit.target_macro(), true);
    push_compact_string_field(
        &mut output,
        "reviewState",
        review_state_name(unit.review_state()),
        true,
    );
    output.push_str("\"translatorNote\":");
    match unit.translator_note() {
        Some(note) => push_json_string(&mut output, note),
        None => output.push_str("null"),
    }
    output.push('}');
    output
}

fn pretty_record_json(unit: &TranslationUnit) -> String {
    let binding = unit.source_binding();
    let fingerprint = unit.source_fingerprint();
    let mut output = String::new();
    output.push('{');
    push_pretty_string_field(&mut output, 2, "id", &unit.id().to_string(), true);
    output.push_str("  \"sourceBinding\": {\n");
    push_pretty_string_field(&mut output, 4, "sheetName", binding.sheet_name(), true);
    push_pretty_number_field(&mut output, 4, "rowId", binding.row_id(), true);
    push_pretty_number_field(&mut output, 4, "subrowId", binding.subrow_id(), true);
    push_pretty_number_field(&mut output, 4, "columnIndex", binding.column_index(), false);
    output.push_str("  },\n");
    output.push_str("  \"sourceFingerprint\": {\n");
    push_pretty_string_field(
        &mut output,
        4,
        "macroTextHash",
        &fingerprint.macro_text_hash().to_hex(),
        true,
    );
    push_pretty_optional_string_field(
        &mut output,
        4,
        "rawValueHash",
        fingerprint.raw_value_hash(),
        true,
    );
    push_pretty_string_field(
        &mut output,
        4,
        "rowTechnicalHash",
        &fingerprint.row_technical_hash().to_hex(),
        false,
    );
    output.push_str("  },\n");
    push_pretty_string_field(&mut output, 2, "targetMacro", unit.target_macro(), true);
    push_pretty_string_field(
        &mut output,
        2,
        "reviewState",
        review_state_name(unit.review_state()),
        true,
    );
    push_pretty_optional_text_field(
        &mut output,
        2,
        "translatorNote",
        unit.translator_note(),
        false,
    );
    output.push('}');
    output
}

fn review_state_name(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Draft => "draft",
        ReviewState::Reviewed => "reviewed",
        ReviewState::NeedsReview => "needs-review",
    }
}

fn push_compact_string_field(output: &mut String, name: &str, value: &str, comma: bool) {
    push_json_string(output, name);
    output.push(':');
    push_json_string(output, value);
    if comma {
        output.push(',');
    }
}

fn push_compact_number_field<T: std::fmt::Display>(
    output: &mut String,
    name: &str,
    value: T,
    comma: bool,
) {
    push_json_string(output, name);
    output.push(':');
    write!(output, "{value}").expect("writing JSON number to String cannot fail");
    if comma {
        output.push(',');
    }
}

fn push_pretty_string_field(
    output: &mut String,
    indent: usize,
    name: &str,
    value: &str,
    comma: bool,
) {
    write!(output, "{}", " ".repeat(indent)).expect("writing indentation cannot fail");
    push_json_string(output, name);
    output.push_str(": ");
    if name == "formatVersion" {
        output.push_str(value);
    } else {
        push_json_string(output, value);
    }
    if comma {
        output.push(',');
    }
    output.push('\n');
}

fn push_pretty_number_field<T: std::fmt::Display>(
    output: &mut String,
    indent: usize,
    name: &str,
    value: T,
    comma: bool,
) {
    write!(output, "{}\"{name}\": {value}", " ".repeat(indent))
        .expect("writing JSON number to String cannot fail");
    if comma {
        output.push(',');
    }
    output.push('\n');
}

fn push_pretty_optional_string_field(
    output: &mut String,
    indent: usize,
    name: &str,
    value: Option<Sha256Hash>,
    comma: bool,
) {
    write!(output, "{}\"{name}\": ", " ".repeat(indent))
        .expect("writing JSON field to String cannot fail");
    match value {
        Some(value) => push_json_string(output, &value.to_hex()),
        None => output.push_str("null"),
    }
    if comma {
        output.push(',');
    }
    output.push('\n');
}

fn push_pretty_optional_text_field(
    output: &mut String,
    indent: usize,
    name: &str,
    value: Option<&str>,
    comma: bool,
) {
    write!(output, "{}\"{name}\": ", " ".repeat(indent))
        .expect("writing JSON field to String cannot fail");
    match value {
        Some(value) => push_json_string(output, value),
        None => output.push_str("null"),
    }
    if comma {
        output.push(',');
    }
    output.push('\n');
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            character if character <= '\u{1f}' => {
                write!(output, "\\u{:04x}", character as u32)
                    .expect("writing JSON escape to String cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

fn file_stats(root: &Path) -> io::Result<(usize, u64)> {
    let mut files = 0;
    let mut bytes = 0;
    collect_file_stats(root, &mut files, &mut bytes)?;
    Ok((files, bytes))
}

fn collect_file_stats(path: &Path, files: &mut usize, bytes: &mut u64) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_file_stats(&entry.path(), files, bytes)?;
        } else if metadata.is_file() {
            *files += 1;
            *bytes += metadata.len();
        }
    }
    Ok(())
}

fn measure_diff(candidate: Candidate, base: &Dataset, scenario: DiffScenario) -> DiffResult {
    let temporary = TemporaryDirectory::new("diff");
    write_layout(temporary.path(), candidate, base).expect("write diff base");
    git_checked(temporary.path(), ["init", "--quiet"]);
    configure_git(temporary.path());
    git_checked(temporary.path(), ["add", "--all", "--force"]);
    git_checked(temporary.path(), ["commit", "--quiet", "-m", "base"]);

    let mut changed = base.clone();
    apply_diff_scenario(&mut changed, scenario);
    write_layout(temporary.path(), candidate, &changed).expect("write diff change");
    git_checked(temporary.path(), ["add", "--all", "--force"]);
    let numstat = git_checked(temporary.path(), ["diff", "--cached", "--numstat"]);
    let names = git_checked(temporary.path(), ["diff", "--cached", "--name-only"]);
    let (added_lines, deleted_lines) = parse_numstat(&String::from_utf8_lossy(&numstat.stdout));
    let files_changed = nonempty_lines(&String::from_utf8_lossy(&names.stdout));
    let unrelated = count_unrelated_record_changes(base, &changed, scenario);
    DiffResult {
        files_changed,
        added_lines,
        deleted_lines,
        unrelated_records_reserialized: unrelated,
    }
}

fn apply_diff_scenario(dataset: &mut Dataset, scenario: DiffScenario) {
    match scenario {
        DiffScenario::ChangeTarget => mutate_unit(dataset, 0, |unit| {
            unit.set_target_macro("Changed target — \"quoted\"\nnext");
        }),
        DiffScenario::ChangeReviewState => mutate_unit(dataset, 1, |unit| {
            unit.set_review_state(if unit.review_state() == ReviewState::Reviewed {
                ReviewState::Draft
            } else {
                ReviewState::Reviewed
            });
        }),
        DiffScenario::ChangeNote => mutate_unit(dataset, 2, |unit| {
            unit.set_translator_note(Some(
                "Changed note with \"quotes\" and \\\\slash".to_owned(),
            ));
        }),
        DiffScenario::ChangeSourceFacts => mutate_unit(dataset, 3, |unit| {
            let new_binding = SourceBinding::new("Rebased_翻訳表", 4_000_003, 7, 31);
            let new_fingerprint = SourceFingerprint::new(
                Sha256Hash::from_bytes([0xA5; 32]),
                Some(Sha256Hash::from_bytes([0x5A; 32])),
                Sha256Hash::from_bytes([0x3C; 32]),
            );
            unit.update_source_after_known_change(new_binding, new_fingerprint);
        }),
        DiffScenario::AddOne => dataset.units.push(make_unit(1_000_000)),
        DiffScenario::DeleteOne => {
            let id = dataset.units[5].id();
            dataset.units.retain(|unit| unit.id() != id);
        }
        DiffScenario::ChangeOneHundred => {
            for index in distinct_indices(dataset.units.len(), 100) {
                mutate_unit(dataset, index, |unit| {
                    unit.set_target_macro(format!("Random edit {}", unit.id()));
                });
            }
        }
        DiffScenario::AddOneHundred => {
            for offset in 0..100_u32 {
                dataset.units.push(make_unit(1_100_000 + offset * 7_919));
            }
        }
    }
    dataset.units.sort_by_key(TranslationUnit::id);
}

fn mutate_unit(dataset: &mut Dataset, index: usize, mutation: impl FnOnce(&mut TranslationUnit)) {
    mutation(
        dataset
            .units
            .get_mut(index)
            .expect("scenario dataset is large enough"),
    );
}

fn distinct_indices(length: usize, count: usize) -> Vec<usize> {
    let mut indices = BTreeSet::new();
    let mut value = 17_u64;
    let modulus = u64::try_from(length).expect("evaluation dataset length fits in u64");
    while indices.len() < count {
        value = value
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        indices.insert(usize::try_from(value % modulus).expect("index modulo fits in usize"));
    }
    indices.into_iter().collect()
}

fn count_unrelated_record_changes(
    before: &Dataset,
    after: &Dataset,
    scenario: DiffScenario,
) -> usize {
    let before_records = before
        .units
        .iter()
        .map(|unit| (unit.id(), record_json(unit)))
        .collect::<BTreeMap<_, _>>();
    let after_records = after
        .units
        .iter()
        .map(|unit| (unit.id(), record_json(unit)))
        .collect::<BTreeMap<_, _>>();
    let changed = before_records
        .iter()
        .filter_map(|(id, record)| after_records.get(id).map(|after| (record, after)))
        .filter(|(record, after)| *record != *after)
        .count();
    changed.saturating_sub(scenario.expected_changed_records())
}

fn parse_numstat(value: &str) -> (usize, usize) {
    value.lines().fold((0, 0), |(added, deleted), line| {
        let mut fields = line.split('\t');
        let added_line = fields
            .next()
            .and_then(|field| field.parse().ok())
            .unwrap_or(0);
        let deleted_line = fields
            .next()
            .and_then(|field| field.parse().ok())
            .unwrap_or(0);
        (added + added_line, deleted + deleted_line)
    })
}

fn nonempty_lines(value: &str) -> usize {
    value.lines().filter(|line| !line.is_empty()).count()
}

fn measure_merge(candidate: Candidate, base: &Dataset, scenario: MergeScenario) -> MergeResult {
    let temporary = TemporaryDirectory::new("merge");
    write_layout(temporary.path(), candidate, base).expect("write merge base");
    git_checked(temporary.path(), ["init", "--quiet"]);
    configure_git(temporary.path());
    git_checked(temporary.path(), ["add", "--all", "--force"]);
    git_checked(temporary.path(), ["commit", "--quiet", "-m", "base"]);
    git_checked(temporary.path(), ["branch", "branch-a"]);
    git_checked(temporary.path(), ["branch", "branch-b"]);

    let (branch_a, branch_b) = merge_datasets(base, scenario);
    git_checked(temporary.path(), ["checkout", "--quiet", "branch-a"]);
    write_layout(temporary.path(), candidate, &branch_a).expect("write merge branch A");
    git_checked(temporary.path(), ["add", "--all", "--force"]);
    git_checked(temporary.path(), ["commit", "--quiet", "-m", "branch-a"]);
    git_checked(temporary.path(), ["checkout", "--quiet", "branch-b"]);
    write_layout(temporary.path(), candidate, &branch_b).expect("write merge branch B");
    git_checked(temporary.path(), ["add", "--all", "--force"]);
    git_checked(temporary.path(), ["commit", "--quiet", "-m", "branch-b"]);
    git_checked(temporary.path(), ["checkout", "--quiet", "branch-a"]);

    let merge = git(temporary.path(), ["merge", "--no-edit", "branch-b"]);
    if merge.status.success() {
        MergeResult {
            outcome: "succeeded",
            conflict_files: 0,
        }
    } else {
        let conflicts = git_checked(temporary.path(), ["diff", "--name-only", "--diff-filter=U"]);
        MergeResult {
            outcome: "conflict",
            conflict_files: nonempty_lines(&String::from_utf8_lossy(&conflicts.stdout)),
        }
    }
}

fn merge_datasets(base: &Dataset, scenario: MergeScenario) -> (Dataset, Dataset) {
    let mut branch_a = base.clone();
    let mut branch_b = base.clone();
    match scenario {
        MergeScenario::IndependentDifferentShards => {
            let (left, right) = pair_different_shards(&base.units);
            edit_target(&mut branch_a, left, "branch A different shard");
            edit_target(&mut branch_b, right, "branch B different shard");
        }
        MergeScenario::IndependentSameShard => {
            let (left, right) = pair_same_shard(&base.units);
            edit_target(&mut branch_a, left, "branch A same shard");
            edit_target(&mut branch_b, right, "branch B same shard");
        }
        MergeScenario::IndependentNearbyIds => {
            let (left, right) = pair_nearby_ids(&base.units);
            edit_target(&mut branch_a, left, "branch A nearby ID");
            edit_target(&mut branch_b, right, "branch B nearby ID");
        }
        MergeScenario::IndependentInsertsDifferentShards => {
            let (left, right) = extra_pair_different_shards(&base.units);
            branch_a.units.push(left);
            branch_b.units.push(right);
        }
        MergeScenario::IndependentInsertsSameShard => {
            let (left, right) = extra_pair_same_shard(&base.units);
            branch_a.units.push(left);
            branch_b.units.push(right);
        }
        MergeScenario::SameUnit => {
            let id = base.units[0].id();
            edit_target_by_id(&mut branch_a, id, "branch A same unit");
            edit_target_by_id(&mut branch_b, id, "branch B same unit");
        }
        MergeScenario::DeleteVersusEdit => {
            let id = base.units[1].id();
            branch_a.units.retain(|unit| unit.id() != id);
            edit_target_by_id(&mut branch_b, id, "branch B edited deleted unit");
        }
    }
    branch_a.units.sort_by_key(TranslationUnit::id);
    branch_b.units.sort_by_key(TranslationUnit::id);
    (branch_a, branch_b)
}

fn edit_target(dataset: &mut Dataset, index: usize, target: &str) {
    let id = dataset.units[index].id();
    edit_target_by_id(dataset, id, target);
}

fn edit_target_by_id(dataset: &mut Dataset, id: TranslationUnitId, target: &str) {
    let unit = dataset
        .units
        .iter_mut()
        .find(|unit| unit.id() == id)
        .expect("merge unit exists");
    unit.set_target_macro(target);
}

fn pair_different_shards(units: &[TranslationUnit]) -> (usize, usize) {
    for (left, unit) in units.iter().enumerate() {
        if let Some((right, _)) = units
            .iter()
            .enumerate()
            .skip(left + 1)
            .find(|(_, other)| other.id().as_bytes()[0] != unit.id().as_bytes()[0])
        {
            return (left, right);
        }
    }
    panic!("dataset must contain units in different shards");
}

fn pair_same_shard(units: &[TranslationUnit]) -> (usize, usize) {
    for (left, unit) in units.iter().enumerate() {
        if let Some((right, _)) = units
            .iter()
            .enumerate()
            .skip(left + 1)
            .find(|(right, other)| {
                *right >= left + 3 && other.id().as_bytes()[0] == unit.id().as_bytes()[0]
            })
        {
            return (left, right);
        }
    }
    pair_nearby_ids(units)
}

fn pair_nearby_ids(units: &[TranslationUnit]) -> (usize, usize) {
    units
        .windows(2)
        .position(|pair| pair[0].id().as_bytes()[0] == pair[1].id().as_bytes()[0])
        .map(|index| (index, index + 1))
        .expect("dataset must contain nearby IDs in the same shard")
}

fn extra_pair_different_shards(units: &[TranslationUnit]) -> (TranslationUnit, TranslationUnit) {
    let existing = units.iter().map(TranslationUnit::id).collect::<Vec<_>>();
    let mut first: Option<TranslationUnit> = None;
    for index in 2_000_000..2_001_000 {
        let candidate = make_unit(index);
        if existing.contains(&candidate.id()) {
            continue;
        }
        if let Some(previous) = first.take() {
            if previous.id().as_bytes()[0] != candidate.id().as_bytes()[0] {
                return (previous, candidate);
            }
            first = Some(previous);
        } else {
            first = Some(candidate);
        }
    }
    panic!("could not find extra units in different shards");
}

fn extra_pair_same_shard(units: &[TranslationUnit]) -> (TranslationUnit, TranslationUnit) {
    let existing = units.iter().map(TranslationUnit::id).collect::<Vec<_>>();
    let mut first_by_shard = BTreeMap::new();
    for index in 2_100_000..2_102_000 {
        let candidate = make_unit(index);
        if existing.contains(&candidate.id()) {
            continue;
        }
        let shard = candidate.id().as_bytes()[0];
        if let Some(previous) = first_by_shard.remove(&shard) {
            return (previous, candidate);
        }
        first_by_shard.insert(shard, candidate);
    }
    panic!("could not find extra units in the same shard");
}

fn configure_git(root: &Path) {
    git_checked(root, ["config", "user.name", "Aeria format evaluation"]);
    git_checked(
        root,
        ["config", "user.email", "format-evaluation@example.invalid"],
    );
    git_checked(root, ["config", "core.autocrlf", "false"]);
}

fn git<const N: usize>(root: &Path, arguments: [&str; N]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .expect("Git CLI is required for the evaluation")
}

fn git_checked<const N: usize>(root: &Path, arguments: [&str; N]) -> Output {
    let output = git(root, arguments);
    assert!(
        output.status.success(),
        "Git command failed: status={:?}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "aeria-workspace-format-{label}-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary evaluation directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_json_is_canonical() {
        let metadata = WorkspaceMetadata::new(
            "en",
            "fr",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "sha256:1111111111111111111111111111111111111111111111111111111111111111",
        )
        .expect("golden metadata is valid");

        assert_eq!(
            manifest_json(&metadata),
            "{\n  \"formatVersion\": 1,\n  \"sourceLanguage\": \"en\",\n  \"targetLanguage\": \"fr\",\n  \"contentId\": \"sha256:0000000000000000000000000000000000000000000000000000000000000000\",\n  \"snapshotId\": \"sha256:1111111111111111111111111111111111111111111111111111111111111111\"\n}\n"
        );
    }

    #[test]
    fn smoke_evaluation_is_deterministic_and_mergeable() {
        let dataset = generate_dataset(SMOKE_SIZE);
        let escaped_record = dataset
            .units
            .iter()
            .find(|unit| unit.target_macro().contains('\n'))
            .map(record_json)
            .expect("smoke data includes a newline-containing target");
        assert!(!escaped_record.contains('\n'));
        assert!(escaped_record.contains("\\n"));
        assert!(escaped_record.contains("\\\""));

        for candidate in Candidate::ALL {
            let first = TemporaryDirectory::new("smoke-first");
            let second = TemporaryDirectory::new("smoke-second");
            write_layout(first.path(), candidate, &dataset).expect("write first smoke layout");
            write_layout(second.path(), candidate, &dataset).expect("write second smoke layout");
            assert_eq!(
                collect_files(first.path()).expect("read first smoke layout"),
                collect_files(second.path()).expect("read second smoke layout"),
                "candidate {} must serialize deterministically",
                candidate.name()
            );

            let diff = measure_diff(candidate, &dataset, DiffScenario::ChangeTarget);
            assert_eq!(
                diff.files_changed,
                1,
                "candidate {} diff files",
                candidate.name()
            );
            assert_eq!(diff.unrelated_records_reserialized, 0);
        }

        let selected = Candidate::ShardedJsonl;
        let target_diff = measure_diff(selected, &dataset, DiffScenario::ChangeTarget);
        assert_eq!(target_diff.files_changed, 1);
        assert_eq!(target_diff.added_lines, 1);
        assert_eq!(target_diff.deleted_lines, 1);
        assert_eq!(target_diff.unrelated_records_reserialized, 0);

        assert_eq!(
            measure_merge(
                selected,
                &dataset,
                MergeScenario::IndependentDifferentShards,
            )
            .outcome,
            "succeeded"
        );
        assert_eq!(
            measure_merge(
                selected,
                &dataset,
                MergeScenario::IndependentInsertsDifferentShards,
            )
            .outcome,
            "succeeded"
        );
        assert_eq!(
            measure_merge(selected, &dataset, MergeScenario::SameUnit).outcome,
            "conflict"
        );
        assert_eq!(
            measure_merge(selected, &dataset, MergeScenario::DeleteVersusEdit).outcome,
            "conflict"
        );
    }

    fn collect_files(root: &Path) -> io::Result<BTreeMap<PathBuf, Vec<u8>>> {
        let mut result = BTreeMap::new();
        collect_files_recursively(root, root, &mut result)?;
        Ok(result)
    }

    fn collect_files_recursively(
        root: &Path,
        current: &Path,
        result: &mut BTreeMap<PathBuf, Vec<u8>>,
    ) -> io::Result<()> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_files_recursively(root, &path, result)?;
            } else {
                let relative = path
                    .strip_prefix(root)
                    .expect("file is under evaluation root")
                    .to_owned();
                result.insert(relative, fs::read(path)?);
            }
        }
        Ok(())
    }
}
