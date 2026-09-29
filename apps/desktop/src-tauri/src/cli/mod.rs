//! The `aeria` command: the project for external agents.
//!
//! Agents such as Claude Code, Codex, or Hermes Agent run it in a project
//! directory to read the game and the project and to write checked
//! translations (see `docs/architecture/agents.md`). It works on the project
//! files directly; an open Aeria window reloads after its writes.

mod project;
mod read;
mod texts;
mod write;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Read as _, Write as _};
use std::path::PathBuf;

use project::{Env, Project};

/// Exit status: done.
const OK: i32 = 0;
/// Exit status: done, but some translations were rejected or the knowledge
/// has problems.
const PARTIAL: i32 = 1;
/// Exit status: the command could not run.
const FAILED: i32 = 2;

const HELP: &str = "\
aeria — read and translate an Aeria project of FINAL FANTASY XIV

Usage: aeria [--project <dir>] [--json] <command> [options]

Commands:
  guide       How the project is organized and how to work on it. Start here.
  brief       The rules every translation follows, for agents that translate.
  overview    The project's areas and progress, or the sheets matching a pattern.
  read        A scene: the strings of a sheet with everything needed to translate them.
  write       Write translations; each is checked, rejected ones say what to fix.
  check       The checks of `write`, without writing.
  find        Search the source text or the translations.
  knowledge   Check the project knowledge in aeria-knowledge/.

Options:
  --project <dir>  The project, or a directory inside it (default: the current directory).
  --json           Machine-readable output.
  -h, --help       Help; `aeria <command> --help` for one command.
  --version        The version of Aeria.

Exit status: 0 done, 1 done with rejected or failed translations or knowledge problems,
2 error.
";

const OVERVIEW_HELP: &str = "\
aeria overview [<pattern>] [--folders] [--untranslated] [--limit <n>]

Without a pattern: the project's languages, game version, progress, areas (quests,
cutscenes, names, items, actions, lore, interface), and the state of the project
knowledge. With a pattern: the matching sheets and their progress. A pattern with
* or ? matches whole names (quest/*, *Gla*); without them it matches names that
contain it (item). Case is ignored.

  --folders       Also list folders, such as quest/000.
  --untranslated  Only sheets with untranslated strings.
  --limit <n>     At most n sheets (default 100).
";

const READ_HELP: &str = "\
aeria read <sheet> [--rows <from>-<to>] [--untranslated] [--limit <n>]
                   [--no-knowledge] [--no-similar]

A scene: the translatable strings of a sheet, a quest or cutscene in play order and any
other sheet in row order. Each string starts with its address, @sheet:row:subrow:column,
which `aeria write` takes, then its kind or speaker and state; a translation marked
(keep) belongs to a person. Below it: the source, the game's other client languages
(the Japanese original and the English, German, and French localizations), what its
macros do, the row's other cells, the current translation, and similar translated
strings. Before the lines comes the project knowledge they need: style, lessons,
terms, character voices, and the story so far.

  --rows <a>-<b>   Only rows a to b; `--rows 500-` from row 500, `--rows 7` one row.
  --untranslated   Only strings without a translation.
  --limit <n>      At most n strings (default 400); the end says how to go on.
  --no-knowledge   Leave out the project knowledge.
  --no-similar     Leave out similar translations (they need the search index).
";

const WRITE_HELP: &str = "\
aeria write [<file>] [--at <address> --text <translation>] [--needs-review]

Writes translations, read from <file>, from standard input, or from --at and --text.
One block per string: an @address line, as `aeria read` prints it (the rest of that
line is ignored), and the translation on the next line. A translation is one line of
macro text; write <br> where the game breaks the line.

  @quest/000/ClsGla001_00001:3:0:1
  Ну наконец-то! Тебя уже заждались.

JSON Lines work too: {\"at\": \"<address>\", \"text\": \"<translation>\"} per line.

Each translation is checked: its macros and structure against the source, forbidden
term variants, and forms that write both genders at once. A rejected one is not
written and the reason says what to fix. Translations a person wrote or changed, and
reviewed ones, are skipped: agents never replace them. Advice, such as a term that
seems missing or phrasing that reads machine-written, comes back with written ones.
Written translations are drafts.

  --needs-review   Mark written translations as needing a person's review.

`aeria check` takes the same input and writes nothing: a dry run. A line reported
FAILED passed its checks but could not be saved, for example because the disk refused
the write; write it again. Several agents may write at once: writes wait for each other.
";

const CHECK_HELP: &str = "\
aeria check [<file>] [--at <address> --text <translation>]

The checks of `aeria write`, input in the same form, without writing anything: a dry
run of a write.
";

const FIND_HELP: &str = "\
aeria find <text> [--in source|translation] [--sheet <pattern>] [--limit <n>]

Finds strings whose text contains <text>, ignoring case and tags: in the source text
(the default; the first search builds the search index of the game version, which
takes a few minutes) or in the project's translations.

  --in <where>       source (default) or translation.
  --sheet <pattern>  Only sheets matching the pattern, as in `aeria overview`.
  --limit <n>        At most n matches (default 30).
";

const KNOWLEDGE_HELP: &str = "\
aeria knowledge

Checks the project knowledge in aeria-knowledge/ (style.md, terms.csv, characters.md,
story.md, lessons.md) and reports every problem with its file and line. Exit status 1
when there are problems. `aeria guide` describes the files.
";

const BRIEF_HELP: &str = "\
aeria brief

The rules every translation of this project follows: what the game's texts in each
language are for, how a translation reads, how the player character is addressed,
how macros are written, and how to write with `aeria write`. Give it to every agent
that translates.
";

const GUIDE_HELP: &str = "\
aeria guide

How the project is organized, the project knowledge, the commands, and how to work on
a large scope with several agents.
";

/// A command's output: text, or one JSON value with `--json`.
pub(crate) struct Output {
    pub json: bool,
    pub text: String,
    value: Option<serde_json::Value>,
}

impl Output {
    fn new(json: bool) -> Self {
        Self {
            json,
            text: String::new(),
            value: None,
        }
    }

    pub(crate) fn json_value(&mut self, value: &serde_json::Value) {
        self.value = Some(value.clone());
    }

    fn print(self) {
        let mut stdout = std::io::stdout().lock();
        let text = match self.value {
            Some(value) => serde_json::to_string_pretty(&value).unwrap_or_default() + "\n",
            None => self.text,
        };
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    }
}

/// A count with thin groups: 12 345.
pub(crate) fn fmt_count(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    grouped
}

/// Parsed arguments of one command.
#[derive(Default)]
struct Arguments {
    positional: Vec<String>,
    switches: BTreeSet<&'static str>,
    values: BTreeMap<&'static str, String>,
}

impl Arguments {
    fn parse(
        args: &[String],
        valued: &[&'static str],
        switches: &[&'static str],
    ) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "--" {
                parsed.positional.extend(iter.by_ref().cloned());
                break;
            }
            let Some(name) = arg.strip_prefix("--") else {
                parsed.positional.push(arg.clone());
                continue;
            };
            let (name, inline) = match name.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (name, None),
            };
            if let Some(known) = valued.iter().find(|known| **known == name) {
                let value = match inline {
                    Some(value) => value,
                    None => iter
                        .next()
                        .cloned()
                        .ok_or_else(|| format!("--{name} needs a value"))?,
                };
                parsed.values.insert(known, value);
            } else if let Some(known) = switches.iter().find(|known| **known == name) {
                parsed.switches.insert(known);
            } else {
                return Err(format!("unknown option --{name}"));
            }
        }
        Ok(parsed)
    }

    fn switch(&self, name: &str) -> bool {
        self.switches.contains(name)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn limit(&self, default: usize) -> Result<usize, String> {
        self.value("limit").map_or(Ok(default), |value| {
            value
                .parse()
                .ok()
                .filter(|limit| *limit > 0)
                .ok_or_else(|| format!("--limit {value:?} is not a positive number"))
        })
    }
}

/// `a-b`, `a-`, or `a`, as an inclusive row range.
fn row_range(text: &str) -> Result<(u32, u32), String> {
    let invalid = || format!("--rows {text:?} is not a row range such as 10-40, 10-, or 10");
    let number = |part: &str| part.trim().parse::<u32>().map_err(|_| invalid());
    match text.split_once('-') {
        Some((first, "")) => Ok((number(first)?, u32::MAX)),
        Some((first, last)) => Ok((number(first)?, number(last)?)),
        None => number(text).map(|row| (row, row)),
    }
}

/// The translations a `write` or `check` gets.
fn entries(args: &Arguments) -> Result<Vec<write::Entry>, String> {
    if let Some(address) = args.value("at") {
        let text = args.value("text").ok_or("--at needs --text")?;
        return Ok(vec![write::Entry {
            address: project::Address::parse(address)?,
            text: text.to_owned(),
        }]);
    }
    let input = match args.positional.first().map(String::as_str) {
        Some("-") | None => {
            let mut stdin = std::io::stdin();
            if stdin.is_terminal() {
                return Err(
                    "give the translations in a file, on standard input, or with --at and --text"
                        .to_owned(),
                );
            }
            let mut input = String::new();
            stdin
                .read_to_string(&mut input)
                .map_err(|error| format!("standard input: {error}"))?;
            input
        }
        Some(path) => std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?,
    };
    let entries = write::parse_entries(&input)?;
    if entries.is_empty() {
        return Err("there are no translations in the input".to_owned());
    }
    Ok(entries)
}

/// Runs the command with the process arguments and returns its exit status.
#[must_use]
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run_with(&args, &Env::standalone()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("aeria: {message}");
            FAILED
        }
    }
}

#[allow(clippy::too_many_lines)] // one dispatch
fn run_with(args: &[String], env: &Env) -> Result<i32, String> {
    // Global options come before the command.
    let mut project_dir: Option<PathBuf> = None;
    let mut json = false;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--project" => {
                project_dir = Some(PathBuf::from(
                    args.get(index + 1).ok_or("--project needs a directory")?,
                ));
                index += 2;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            "--version" | "-V" => {
                println!("aeria {}", env!("CARGO_PKG_VERSION"));
                return Ok(OK);
            }
            "-h" | "--help" | "help" => {
                let topic = args.get(index + 1).map(String::as_str);
                print!("{}", topic.and_then(command_help).unwrap_or(HELP));
                return Ok(OK);
            }
            other if other.starts_with("--project=") => {
                project_dir = Some(PathBuf::from(&other["--project=".len()..]));
                index += 1;
            }
            _ => break,
        }
    }
    let Some(command) = args.get(index) else {
        print!("{HELP}");
        return Ok(OK);
    };
    let rest: Vec<String> = args[index + 1..]
        .iter()
        .filter(|arg| {
            if *arg == "--json" {
                json = true;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect();
    if rest.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{}", command_help(command).ok_or_else(|| unknown(command))?);
        return Ok(OK);
    }
    let start = project_dir.unwrap_or_else(|| PathBuf::from("."));
    let mut out = Output::new(json);
    let code = match command.as_str() {
        "guide" => {
            Arguments::parse(&rest, &[], &[])?;
            let project = Project::open(&start, env)?;
            out.text = texts::guide(&project);
            OK
        }
        "brief" => {
            Arguments::parse(&rest, &[], &[])?;
            let project = Project::open(&start, env)?;
            out.text = texts::brief(&project);
            OK
        }
        "overview" => {
            let parsed = Arguments::parse(&rest, &["limit"], &["folders", "untranslated"])?;
            let project = Project::open(&start, env)?;
            read::overview(
                &project,
                &read::OverviewOptions {
                    pattern: parsed.positional.first().cloned(),
                    folders: parsed.switch("folders"),
                    untranslated: parsed.switch("untranslated"),
                    limit: parsed.limit(100)?,
                },
                &mut out,
            );
            OK
        }
        "read" => {
            let parsed = Arguments::parse(
                &rest,
                &["rows", "limit"],
                &["untranslated", "no-knowledge", "no-similar"],
            )?;
            let sheet = parsed
                .positional
                .first()
                .ok_or("name the sheet to read; `aeria overview <pattern>` lists sheets")?
                .clone();
            let project = Project::open(&start, env)?;
            read::read(
                &project,
                &read::ReadOptions {
                    sheet,
                    rows: parsed.value("rows").map(row_range).transpose()?,
                    untranslated: parsed.switch("untranslated"),
                    limit: parsed.limit(400)?,
                    knowledge: !parsed.switch("no-knowledge"),
                    memory: !parsed.switch("no-similar"),
                },
                &mut out,
            )?;
            OK
        }
        "write" | "check" => {
            let dry_run = command == "check";
            let parsed = if dry_run {
                Arguments::parse(&rest, &["at", "text"], &[])?
            } else {
                Arguments::parse(&rest, &["at", "text"], &["needs-review"])?
            };
            let entries = entries(&parsed)?;
            let clean = write::write(
                &start,
                env,
                &entries,
                &write::WriteOptions {
                    dry_run,
                    needs_review: parsed.switch("needs-review"),
                },
                &mut out,
            )?;
            if clean { OK } else { PARTIAL }
        }
        "find" => {
            let parsed = Arguments::parse(&rest, &["in", "sheet", "limit"], &[])?;
            let text = parsed.positional.join(" ");
            if text.trim().is_empty() {
                return Err("give the text to find".to_owned());
            }
            let within = match parsed.value("in").unwrap_or("source") {
                "source" => read::FindIn::Source,
                "translation" | "translations" | "target" => read::FindIn::Translation,
                other => return Err(format!("--in {other:?}: use source or translation")),
            };
            let project = Project::open(&start, env)?;
            read::find(
                &project,
                &read::FindOptions {
                    text,
                    within,
                    sheet: parsed.value("sheet").map(str::to_owned),
                    limit: parsed.limit(30)?,
                },
                &mut out,
            )?;
            OK
        }
        "knowledge" => {
            Arguments::parse(&rest, &[], &[])?;
            let project = Project::open(&start, env)?;
            if read::knowledge(&project, &mut out) {
                OK
            } else {
                PARTIAL
            }
        }
        other => return Err(unknown(other)),
    };
    out.print();
    Ok(code)
}

fn unknown(command: &str) -> String {
    format!("unknown command {command:?}; `aeria --help` lists the commands")
}

fn command_help(command: &str) -> Option<&'static str> {
    Some(match command {
        "overview" => OVERVIEW_HELP,
        "read" => READ_HELP,
        "write" => WRITE_HELP,
        "check" => CHECK_HELP,
        "find" => FIND_HELP,
        "knowledge" => KNOWLEDGE_HELP,
        "brief" => BRIEF_HELP,
        "guide" => GUIDE_HELP,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn arguments_ranges_and_counts_parse() {
        let parsed = Arguments::parse(
            &strings(&["quest/*", "--limit", "5", "--untranslated", "--rows=3-9"]),
            &["limit", "rows"],
            &["untranslated"],
        )
        .expect("parse");
        assert_eq!(parsed.positional, ["quest/*"]);
        assert_eq!(parsed.limit(100).expect("limit"), 5);
        assert!(parsed.switch("untranslated"));
        assert_eq!(parsed.value("rows"), Some("3-9"));
        assert!(Arguments::parse(&strings(&["--nope"]), &[], &[]).is_err());
        assert!(Arguments::parse(&strings(&["--limit"]), &["limit"], &[]).is_err());

        assert_eq!(row_range("10-40").expect("range"), (10, 40));
        assert_eq!(row_range("10-").expect("open"), (10, u32::MAX));
        assert_eq!(row_range("7").expect("one"), (7, 7));
        assert!(row_range("x").is_err());

        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(1234), "1 234");
        assert_eq!(fmt_count(860_000), "860 000");
    }

    #[test]
    fn every_command_has_help() {
        for command in [
            "overview",
            "read",
            "write",
            "check",
            "find",
            "knowledge",
            "brief",
            "guide",
        ] {
            assert!(command_help(command).is_some(), "{command}");
            assert!(HELP.contains(command), "{command}");
        }
        let env = Env::default();
        assert_eq!(run_with(&strings(&["--help"]), &env).expect("help"), OK);
        assert!(run_with(&strings(&["nonsense"]), &env).is_err());
    }

    /// A Russian project over the synthetic game, with its own data folder.
    fn project() -> (
        tempfile::TempDir,
        crate::test_support::TestGame,
        Env,
        PathBuf,
    ) {
        let directory = tempfile::tempdir().expect("directory");
        let game = crate::test_support::test_game();
        let root = directory.path().join("project");
        std::fs::create_dir_all(&root).expect("root");
        aeria_workspace::ProjectSession::initialize(
            &root,
            crate::test_support::open(game.path()),
            "ru",
        )
        .expect("project");
        let env = Env {
            data_dir: Some(directory.path().join("data")),
            cache_dir: Some(directory.path().join("cache")),
            game_path: Some(game.path().to_string_lossy().into_owned()),
        };
        (directory, game, env, root)
    }

    fn run_in(root: &std::path::Path, env: &Env, args: &[&str]) -> i32 {
        let mut all = vec!["--project".to_owned(), root.to_string_lossy().into_owned()];
        all.extend(strings(args));
        run_with(&all, env).expect("runs")
    }

    #[test]
    fn agents_write_their_own_strings_and_never_a_persons() {
        let (_directory, _game, env, root) = project();
        let at = "Addon:1:0:1";
        assert_eq!(
            run_in(&root, &env, &["check", "--at", at, "--text", "ОК"]),
            OK
        );
        assert_eq!(
            run_in(&root, &env, &["write", "--at", at, "--text", "ОК"]),
            OK
        );
        let address = project::Address::parse(at).expect("address");
        let author = |env: &Env| {
            let project = Project::open(&root, env).expect("open");
            let ledger = project.ledger();
            project::current(&project.session, ledger.as_ref(), &address).expect("translated")
        };
        let written = author(&env);
        assert_eq!(
            (written.target.as_str(), written.author),
            ("ОК", project::Author::Agent)
        );
        let stamp = env
            .project_files(&root)
            .expect("files")
            .expect("data")
            .stamp();
        assert!(stamp.is_some(), "open windows are told");

        // An agent replaces its own translation.
        assert_eq!(
            run_in(&root, &env, &["write", "--at", at, "--text", "Готово"]),
            OK
        );
        assert_eq!(author(&env).target, "Готово");

        // A person's edit makes the string theirs; agents skip it.
        let mut project = Project::open(&root, &env).expect("open");
        project
            .session
            .set_target(&address.binding(), "Подтвердить")
            .expect("person");
        drop(project);
        assert_eq!(author(&env).author, project::Author::Person);
        assert_eq!(
            run_in(&root, &env, &["write", "--at", at, "--text", "ОК"]),
            OK
        );
        assert_eq!(author(&env).target, "Подтвердить");

        // A broken translation is rejected and nothing is written.
        assert_eq!(
            run_in(
                &root,
                &env,
                &["write", "--at", "Addon:2:0:1", "--text", "<i>Отмена"]
            ),
            PARTIAL
        );
        let project = Project::open(&root, &env).expect("open");
        assert!(
            project::current(
                &project.session,
                None,
                &project::Address::parse("Addon:2:0:1").expect("address")
            )
            .is_none()
        );
    }
}
