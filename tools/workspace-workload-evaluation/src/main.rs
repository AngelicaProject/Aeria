//! Measures Workspace Format v2 under realistic translation workloads.
//!
//! The Workspace Format v1 evaluation compared layouts on random edits to a
//! small dataset. This harness models how Aeria projects are actually used:
//! a corpus of large sheets, a partly translated base, AI jobs that draft a
//! whole sheet at once, and translators working in parallel. It writes the
//! current format with the real `aeria-workspace` writer and measures with
//! real Git:
//!
//! - repository size, file count, and packed size, and what the bytes are;
//! - what one AI job changes (files, lines, packed growth);
//! - how often parallel branches conflict with Git's text merge, and with
//!   Aeria's per-unit merge driver.
//!
//! Two alternatives are measured beside the current layout, to inform a
//! future format decision: one file per sheet sorted by row, and a compact
//! record encoding (positional fields, base64 hashes, and the sheet schema
//! hash in a per-sheet table).
//!
//! ```text
//! cargo run --release -p aeria-workspace-workload-evaluation -- [--scale 0.25]
//! ```

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use aeria_core::{
    ReviewState, Sha256Hash, SourceBinding, SourceFingerprint, SourceLayout, TranslationUnit,
    TranslationUnitId,
};
use aeria_workspace::{encode_unit_shard, unit_shard_path};

const MANIFEST: &str = "{\n  \"formatVersion\": 2,\n  \"sourceLanguage\": \"en\",\n  \"targetLanguage\": \"ru\",\n  \"contentId\": \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n}\n";

/// A sheet of the modelled corpus: name, rows, string columns, keyed rows,
/// and words per string.
struct Sheet {
    name: &'static str,
    rows: u32,
    columns: u32,
    keyed: bool,
    words: (usize, usize),
}

/// Roughly the shape of a large game: a few huge sheets and many small ones.
const SHEETS: [Sheet; 8] = [
    Sheet {
        name: "Item",
        rows: 40_000,
        columns: 2,
        keyed: false,
        words: (1, 25),
    },
    Sheet {
        name: "Action",
        rows: 20_000,
        columns: 2,
        keyed: false,
        words: (1, 30),
    },
    Sheet {
        name: "Addon",
        rows: 15_000,
        columns: 1,
        keyed: false,
        words: (1, 8),
    },
    Sheet {
        name: "Quest",
        rows: 8_000,
        columns: 4,
        keyed: true,
        words: (2, 20),
    },
    Sheet {
        name: "QuestText",
        rows: 60_000,
        columns: 1,
        keyed: true,
        words: (5, 40),
    },
    Sheet {
        name: "Status",
        rows: 5_000,
        columns: 2,
        keyed: false,
        words: (1, 20),
    },
    Sheet {
        name: "Achievement",
        rows: 4_000,
        columns: 2,
        keyed: false,
        words: (2, 15),
    },
    Sheet {
        name: "Mount",
        rows: 1_000,
        columns: 3,
        keyed: false,
        words: (1, 30),
    },
];

const WORDS: [&str; 24] = [
    "зелье",
    "силы",
    "урон",
    "всем",
    "врагам",
    "радиус",
    "ялмов",
    "скорость",
    "снижает",
    "восстанавливает",
    "здоровье",
    "эфир",
    "кристалл",
    "задание",
    "награда",
    "уровень",
    "класс",
    "умение",
    "эффект",
    "длительность",
    "секунд",
    "цель",
    "группа",
    "предмет",
];

/// Deterministic generator (`SplitMix64`).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound as u64).unwrap_or(0)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn hash(&mut self) -> Sha256Hash {
        let mut bytes = [0; 32];
        for chunk in bytes.chunks_mut(8) {
            chunk.copy_from_slice(&self.next().to_le_bytes()[..chunk.len()]);
        }
        Sha256Hash::from_bytes(bytes)
    }

    fn text(&mut self, words: (usize, usize)) -> String {
        let count = words.0 + self.below(words.1 - words.0 + 1);
        (0..count)
            .map(|_| WORDS[self.below(WORDS.len())])
            .collect::<Vec<_>>()
            .join(" ")
    }
}

type Units = BTreeMap<TranslationUnitId, TranslationUnit>;

/// A cell of the corpus.
#[derive(Clone, Copy)]
struct Cell {
    sheet: usize,
    row: u32,
    column: u32,
}

struct Corpus {
    base: Units,
    /// Untranslated cells per sheet, for jobs.
    open: Vec<Vec<Cell>>,
    /// Sheet schema hashes.
    layouts: Vec<Sha256Hash>,
}

fn scaled(rows: u32, scale: f64) -> u32 {
    // Truncation is intended; at least one row per sheet.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rows = (f64::from(rows) * scale) as u32;
    rows.max(1)
}

fn make_unit(rng: &mut Rng, corpus_layouts: &[Sha256Hash], cell: Cell) -> TranslationUnit {
    let sheet = &SHEETS[cell.sheet];
    let binding = SourceBinding::new(sheet.name, cell.row, 0, cell.column);
    let fingerprint =
        SourceFingerprint::new(rng.hash(), rng.chance(20).then(|| rng.hash()), rng.hash());
    let id = TranslationUnitId::derive("en", &binding, &fingerprint).expect("identity");
    let mut unit = TranslationUnit::new(id, binding, fingerprint, rng.text(sheet.words))
        .with_source_layout(SourceLayout::new(corpus_layouts[cell.sheet], cell.column))
        .with_source_row_key(sheet.keyed.then(|| rng.hash()));
    unit.set_review_state(match rng.below(10) {
        0..=6 => ReviewState::Draft,
        7 | 8 => ReviewState::Reviewed,
        _ => ReviewState::NeedsReview,
    });
    unit
}

fn corpus(scale: f64) -> Corpus {
    let mut rng = Rng(0x00ae_71a0);
    let layouts: Vec<_> = SHEETS.iter().map(|_| rng.hash()).collect();
    let mut base = Units::new();
    let mut open = Vec::new();
    for (index, sheet) in SHEETS.iter().enumerate() {
        let mut untranslated = Vec::new();
        for row in 0..scaled(sheet.rows, scale) {
            for column in 0..sheet.columns {
                let cell = Cell {
                    sheet: index,
                    row,
                    column,
                };
                if rng.chance(50) {
                    let unit = make_unit(&mut rng, &layouts, cell);
                    base.insert(unit.id(), unit);
                } else {
                    untranslated.push(cell);
                }
            }
        }
        open.push(untranslated);
    }
    Corpus {
        base,
        open,
        layouts,
    }
}

/// Where and how units are stored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Workspace Format v2: 256 shards by ID, canonical records.
    Current,
    /// One file per sheet, records sorted by row, canonical records.
    PerSheet,
    /// 256 shards by ID with a compact positional record encoding.
    Compact,
}

impl Layout {
    const ALL: [Self; 3] = [Self::Current, Self::PerSheet, Self::Compact];

    const fn name(self) -> &'static str {
        match self {
            Self::Current => "current (v2)",
            Self::PerSheet => "file per sheet",
            Self::Compact => "compact records",
        }
    }
}

fn canonical_line(unit: &TranslationUnit) -> Vec<u8> {
    let path = unit_shard_path(unit.id());
    encode_unit_shard(std::slice::from_ref(unit), Path::new(&path)).expect("canonical record")
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let value = chunk
            .iter()
            .enumerate()
            .fold(0_u32, |value, (index, byte)| {
                value | (u32::from(*byte) << (16 - 8 * index))
            });
        for index in 0..=chunk.len() {
            text.push(char::from(
                ALPHABET[((value >> (18 - 6 * index)) & 63) as usize],
            ));
        }
    }
    text
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            character if u32::from(character) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(character));
            }
            character => out.push(character),
        }
    }
    out.push('"');
    out
}

/// A positional record: the same facts without field names, hashes in
/// base64, and the sheet schema hash replaced by the sheet's index in a
/// per-sheet table.
fn compact_line(unit: &TranslationUnit) -> Vec<u8> {
    let binding = unit.source_binding();
    let fingerprint = unit.source_fingerprint();
    let sheet = SHEETS
        .iter()
        .position(|sheet| sheet.name == binding.sheet_name())
        .unwrap_or(0);
    let optional = |hash: Option<Sha256Hash>| {
        hash.map_or_else(
            || "null".to_owned(),
            |hash| json_string(&base64(hash.as_bytes())),
        )
    };
    let review = match unit.review_state() {
        ReviewState::Draft => 0,
        ReviewState::NeedsReview => 1,
        ReviewState::Reviewed => 2,
    };
    format!(
        "[{},{sheet},{},{},{},{},{},{},{},{},{},{review},{}]\n",
        json_string(&base64(unit.id().as_bytes())),
        binding.row_id(),
        binding.subrow_id(),
        binding.column_index(),
        json_string(&base64(fingerprint.macro_text_hash().as_bytes())),
        optional(fingerprint.raw_value_hash()),
        json_string(&base64(fingerprint.row_technical_hash().as_bytes())),
        unit.source_layout()
            .map_or(0, |layout| layout.column_offset()),
        optional(unit.source_row_key()),
        json_string(unit.target_macro()),
        unit.translator_note()
            .map_or_else(|| "null".to_owned(), json_string),
    )
    .into_bytes()
}

/// The files of `units` in `layout`, by repository path.
fn files(layout: Layout, units: &Units, layouts: &[Sha256Hash]) -> BTreeMap<String, Vec<u8>> {
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    files.insert(
        ".aeria/manifest.json".to_owned(),
        MANIFEST.as_bytes().to_vec(),
    );
    match layout {
        Layout::Current | Layout::Compact => {
            for unit in units.values() {
                let line = if layout == Layout::Current {
                    canonical_line(unit)
                } else {
                    compact_line(unit)
                };
                files
                    .entry(unit_shard_path(unit.id()))
                    .or_default()
                    .extend(line);
            }
            if layout == Layout::Compact {
                let mut table = String::new();
                for (sheet, hash) in SHEETS.iter().zip(layouts) {
                    let _ = writeln!(
                        table,
                        "[{},{}]",
                        json_string(sheet.name),
                        json_string(&base64(hash.as_bytes()))
                    );
                }
                files.insert(".aeria/sheets.jsonl".to_owned(), table.into_bytes());
            }
        }
        Layout::PerSheet => {
            let mut sorted: Vec<&TranslationUnit> = units.values().collect();
            sorted.sort_by_key(|unit| {
                let binding = unit.source_binding();
                (
                    binding.sheet_name().to_owned(),
                    binding.row_id(),
                    binding.subrow_id(),
                    binding.column_index(),
                )
            });
            for unit in sorted {
                let path = format!(".aeria/sheets/{}.jsonl", unit.source_binding().sheet_name());
                files.entry(path).or_default().extend(canonical_line(unit));
            }
        }
    }
    files
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("run git");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn git_checked(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(root)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?}");
}

/// Replaces the tracked files with `files` and commits them.
fn commit(root: &Path, files: &BTreeMap<String, Vec<u8>>, message: &str) {
    let aeria = root.join(".aeria");
    if aeria.exists() {
        fs::remove_dir_all(&aeria).expect("clear");
    }
    for (path, bytes) in files {
        let full = root.join(path);
        fs::create_dir_all(full.parent().expect("parent")).expect("folders");
        fs::write(full, bytes).expect("write");
    }
    git_checked(root, &["add", "--all"]);
    git_checked(root, &["commit", "--quiet", "--allow-empty", "-m", message]);
}

fn repository() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    let root = folder.path();
    git_checked(root, &["init", "--quiet", "--initial-branch=main"]);
    for (key, value) in [
        ("user.name", "Evaluation"),
        ("user.email", "evaluation@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
        ("gc.auto", "0"),
    ] {
        git_checked(root, &["config", key, value]);
    }
    folder
}

/// Packed repository size in KiB after `git gc`.
fn packed_kib(root: &Path) -> u64 {
    git_checked(root, &["gc", "--quiet"]);
    git(root, &["count-objects", "-v"])
        .lines()
        .find_map(|line| line.strip_prefix("size-pack: "))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

/// Files changed and lines inserted by the last commit.
fn last_commit_stat(root: &Path) -> (usize, usize) {
    let stat = git(root, &["diff", "--shortstat", "HEAD~1", "HEAD"]);
    let number = |label: &str| {
        stat.split(',')
            .find(|part| part.contains(label))
            .and_then(|part| part.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    };
    (number("file"), number("insertion"))
}

fn add_cells(units: &mut Units, rng: &mut Rng, layouts: &[Sha256Hash], cells: &[Cell]) {
    for cell in cells {
        let unit = make_unit(rng, layouts, *cell);
        units.insert(unit.id(), unit);
    }
}

/// Edits `count` strings of `sheet`, spread over the sheet like real work
/// rather than taken in ID order, which would cluster them in a few shards.
/// Both branches of a scenario see the same order, so `skip` picks strings
/// the other branch does not edit.
fn edit_sheet(units: &mut Units, rng: &mut Rng, sheet: &str, count: usize, skip: usize) {
    let mut ids: Vec<TranslationUnitId> = units
        .values()
        .filter(|unit| unit.source_binding().sheet_name() == sheet)
        .map(TranslationUnit::id)
        .collect();
    let mut order = Rng(0x5eed);
    for index in (1..ids.len()).rev() {
        ids.swap(index, order.below(index + 1));
    }
    for id in ids.into_iter().skip(skip).take(count) {
        let words = SHEETS
            .iter()
            .find(|entry| entry.name == sheet)
            .map_or((1, 10), |entry| entry.words);
        let text = rng.text(words);
        if let Some(unit) = units.get_mut(&id) {
            unit.set_target_macro(text);
        }
    }
}

/// A change one branch makes.
type Change = Box<dyn Fn(&mut Units, &mut Rng)>;

/// One scenario of two branches from the same base.
struct Parallel {
    name: &'static str,
    left: Change,
    right: Change,
}

struct MergeOutcome {
    text_conflicted_files: usize,
    driver_conflicting_units: Option<usize>,
}

fn merge(layout: Layout, corpus: &Corpus, scenario: &Parallel) -> MergeOutcome {
    let folder = repository();
    let root = folder.path();
    let base_files = files(layout, &corpus.base, &corpus.layouts);
    commit(root, &base_files, "base");
    let mut left = corpus.base.clone();
    (scenario.left)(&mut left, &mut Rng(1));
    let mut right = corpus.base.clone();
    (scenario.right)(&mut right, &mut Rng(2));
    let left_files = files(layout, &left, &corpus.layouts);
    let right_files = files(layout, &right, &corpus.layouts);

    git_checked(root, &["switch", "--quiet", "-c", "left"]);
    commit(root, &left_files, "left");
    git_checked(root, &["switch", "--quiet", "main"]);
    commit(root, &right_files, "right");
    let _ = Command::new("git")
        .current_dir(root)
        .args(["merge", "--quiet", "--no-edit", "left"])
        .output();
    let text_conflicted_files = git(root, &["diff", "--name-only", "--diff-filter=U"])
        .lines()
        .count();

    let driver_conflicting_units = (layout == Layout::Current).then(|| {
        let empty = Vec::new();
        base_files
            .keys()
            .chain(left_files.keys())
            .chain(right_files.keys())
            .filter(|path| path.starts_with(".aeria/units/"))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|path| {
                aeria_git::merge_shard_for_driver(
                    path,
                    base_files.get(path).unwrap_or(&empty),
                    right_files.get(path).unwrap_or(&empty),
                    left_files.get(path).unwrap_or(&empty),
                )
                .expect("driver merge")
                .conflicts
            })
            .sum()
    });
    MergeOutcome {
        text_conflicted_files,
        driver_conflicting_units,
    }
}

fn scenarios(corpus: &Corpus) -> Vec<Parallel> {
    let open = |sheet: &str| {
        corpus.open[SHEETS
            .iter()
            .position(|entry| entry.name == sheet)
            .expect("sheet")]
        .clone()
    };
    let layouts = corpus.layouts.clone();
    let (addon, status, item) = (open("Addon"), open("Status"), open("Item"));
    let (half, rest) = item.split_at(item.len() / 2);
    let (half, rest) = (half.to_vec(), rest.to_vec());
    let (l1, l2, l3, l4) = (layouts.clone(), layouts.clone(), layouts.clone(), layouts);
    vec![
        Parallel {
            name: "Two translators, different sheets (300 edits each)",
            left: Box::new(|units, rng| edit_sheet(units, rng, "Quest", 300, 0)),
            right: Box::new(|units, rng| edit_sheet(units, rng, "Action", 300, 0)),
        },
        Parallel {
            name: "Two translators, same sheet, different strings (300 each)",
            left: Box::new(|units, rng| edit_sheet(units, rng, "Item", 300, 0)),
            right: Box::new(|units, rng| edit_sheet(units, rng, "Item", 300, 300)),
        },
        Parallel {
            name: "Two AI jobs, different sheets (Addon and Status)",
            left: Box::new(move |units, rng| add_cells(units, rng, &l1, &addon)),
            right: Box::new(move |units, rng| add_cells(units, rng, &l2, &status)),
        },
        Parallel {
            name: "Two AI jobs, halves of one sheet (Item)",
            left: Box::new(move |units, rng| add_cells(units, rng, &l3, &half)),
            right: Box::new(move |units, rng| add_cells(units, rng, &l4, &rest)),
        },
        Parallel {
            name: "Control: both edit the same 20 strings",
            left: Box::new(|units, rng| edit_sheet(units, rng, "Mount", 20, 0)),
            right: Box::new(|units, rng| edit_sheet(units, rng, "Mount", 20, 0)),
        },
    ]
}

/// Average canonical record size and the shares taken by hex hashes, JSON
/// field names, and targets.
fn composition(units: &Units) -> (usize, f64, f64, f64) {
    let (mut total, mut hashes, mut names, mut targets) = (0_usize, 0_usize, 0_usize, 0_usize);
    for unit in units.values() {
        let line = String::from_utf8(canonical_line(unit)).expect("UTF-8");
        total += line.len();
        targets += json_string(unit.target_macro()).len();
        let optional = usize::from(unit.source_fingerprint().raw_value_hash().is_some())
            + usize::from(unit.source_row_key().is_some());
        // ID, macro text, row technical, and schema hashes, plus the optional ones.
        hashes += 64 * (4 + optional);
        names += line
            .split(['{', ','])
            .filter_map(|part| part.strip_prefix('"')?.split_once("\":"))
            .map(|(name, _)| name.len() + 3)
            .sum::<usize>();
    }
    #[allow(clippy::cast_precision_loss)]
    let share = |part: usize| part as f64 * 100.0 / total.max(1) as f64;
    (
        total / units.len().max(1),
        share(hashes),
        share(names),
        share(targets),
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scale = args
        .iter()
        .position(|arg| arg == "--scale")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(1.0_f64);
    let corpus = corpus(scale);
    let open: usize = corpus.open.iter().map(Vec::len).sum();
    println!("# Workspace workload evaluation\n");
    println!(
        "Scale {scale}: {} translated units in the base, {open} untranslated cells.\n",
        corpus.base.len()
    );

    let (average, hashes, names, targets) = composition(&corpus.base);
    println!(
        "Canonical record: {average} bytes on average; hex hashes {hashes:.0} %, field names {names:.0} %, targets {targets:.0} %.\n"
    );

    println!("## Size and one AI job\n");
    println!("The job drafts every untranslated cell of Item.\n");
    println!(
        "| Layout | Files | Working tree | Packed | Job: files changed | Job: lines added | Job: packed growth |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
    let item = corpus.open[0].clone();
    for layout in Layout::ALL {
        let folder = repository();
        let root = folder.path();
        let base = files(layout, &corpus.base, &corpus.layouts);
        let bytes: usize = base.values().map(Vec::len).sum();
        commit(root, &base, "base");
        let before = packed_kib(root);
        let mut job = corpus.base.clone();
        add_cells(&mut job, &mut Rng(3), &corpus.layouts, &item);
        commit(root, &files(layout, &job, &corpus.layouts), "job");
        let (changed, added) = last_commit_stat(root);
        let after = packed_kib(root);
        println!(
            "| {} | {} | {:.1} MiB | {:.1} MiB | {changed} | {added} | {:.1} MiB |",
            layout.name(),
            base.len(),
            to_mib(bytes as u64),
            to_mib(before * 1024),
            to_mib(after.saturating_sub(before) * 1024)
        );
    }

    println!("\n## Parallel branches\n");
    println!(
        "Conflicted files with Git's text merge; for the current layout also the units Aeria's per-unit merge (Sync, Pull, or the command-line driver) leaves to a person.\n"
    );
    println!(
        "| Scenario | current: text | current: per-unit | file per sheet: text | compact: text |"
    );
    println!("| --- | ---: | ---: | ---: | ---: |");
    for scenario in scenarios(&corpus) {
        let current = merge(Layout::Current, &corpus, &scenario);
        let per_sheet = merge(Layout::PerSheet, &corpus, &scenario);
        let compact = merge(Layout::Compact, &corpus, &scenario);
        println!(
            "| {} | {} | {} | {} | {} |",
            scenario.name,
            current.text_conflicted_files,
            current
                .driver_conflicting_units
                .map_or_else(String::new, |units| format!("{units} units")),
            per_sheet.text_conflicted_files,
            compact.text_conflicted_files
        );
    }
}

#[allow(clippy::cast_precision_loss)]
fn to_mib(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}
