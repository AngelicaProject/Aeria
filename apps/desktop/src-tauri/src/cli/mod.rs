//! The `aeria` command: the project for external agents.
//!
//! Agents such as Claude Code, Codex, or Hermes Agent run it in a project
//! directory to read the game and the project and to write checked
//! translations (see `docs/architecture/agents.md`). It works on the project
//! files directly; an open Aeria window reloads after its writes.
//!
//! A command is a thin client: it hands its arguments to the project's server
//! (see [`serve`]), which keeps the game, the workspace, and every cache open
//! between commands, and prints what the server answers. Help and version
//! are answered without a server; `AERIA_NO_SERVER` runs a command
//! in-process.

mod audit;
mod project;
mod plan;
mod read;
mod serve;
mod texts;
mod write;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{IsTerminal, Read as _, Write as _};
use std::path::{Path, PathBuf};

use project::{Env, Project};
use serde::{Deserialize, Serialize};

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
  plan        Split the untranslated strings of a scope into tasks for parallel agents.
  read        A scene: the strings of a sheet with everything needed to translate them.
  write       Write translations; each is checked, rejected ones say what to fix.
  check       The checks of `write`, without writing.
  find        Search the source text or the translations.
  audit       Deterministic checks across the project: inconsistent translations,
              forbidden terms, broken macros, gender, machine phrasing, length.
  review      Translations waiting for review, and those whose source is gone.
  flag        Mark translations for a person's review, with the reason.
  knowledge   Check the project knowledge in aeria-knowledge/.
  init        Write AGENTS.md and CLAUDE.md so agent harnesses find the command.

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

/// Output of `aeria read` at most, in bytes: agent harnesses cut
/// command output at about 30 000 characters or 50 KB.
const READ_MAX_BYTES: usize = 24_000;

const PLAN_HELP: &str = "\
aeria plan <pattern> [--size <n>] [--terms] [--from <n>] [--limit <n>]

The untranslated strings of the matching sheets, split into tasks that agents translate at
the same time without overlapping. A quest or cutscene is one scene and is never split;
other sheets are cut into chunks of at most --size strings, each read with
`aeria read <sheet> --untranslated --from <position> --limit <n>`, where the position
counts the sheet's whole listing, so writes by other agents do not shift it. Small
sheets and scenes are packed together up to --size. Before the tasks come the
instructions to give every agent that runs one.

  --size <n>   Strings per task at most (default 50); a larger scene stays whole.
  --terms      Also list the words that recur in these strings and have no term in
               aeria-knowledge/terms.csv, to settle before the tasks run.
  --from <n>   Start at task n (1 is the first).
  --limit <n>  At most n tasks (default 200); the end says how to go on.
";

const READ_HELP: &str = "\
aeria read <sheet> [--rows <from>-<to>] [--untranslated] [--from <n>] [--limit <n>]
                   [--max-bytes <n>] [--no-knowledge] [--no-similar]

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
  --from <n>       Start at the n-th string of the sheet's whole listing (1 is the
                   first). Positions count every string, so filters and writes by
                   other agents do not shift them.
  --limit <n>      At most n strings (default 400).
  --max-bytes <n>  At most n bytes of output (default 24000, 0 for no bound),
                   so terminals that cut long output do not lose strings. Whole strings
                   past it are left out, and the end gives the command that goes on.
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
The block form needs no escaping: quotes, apostrophes, and backslashes are written as
they are. Never put translations inside a shell command line, where quoting corrupts
them: write the blocks to a file with a file-writing tool and pass its path, or pipe
them on standard input (`aeria write -`). A file belongs in the system's temporary
folder, never in the project, and is deleted after the write. Writing also checks, so
`aeria check` first is not needed.

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

const AUDIT_HELP: &str = "\
aeria audit [<pattern>] [--check <name>[,<name>...]] [--limit <n>]

Checks every translation, or those of the sheets matching the pattern, without a
model: the same source translated differently (inconsistent), forbidden term variants
(forbidden), broken macros or structure (structure), both genders written at once
(both-genders), a source that varies with the player character's gender and a
translation that does not (gender), a term whose translation does not seem to be
used (terms), phrasing that reads machine-written (phrasing), and interface strings
much longer than their source (long). Each finding names whose translation it is;
fix an agent's with `aeria write`, and flag a person's with `aeria flag`.
Exit status 1 when there are findings.

  --check <names>  Only these checks, separated by commas.
  --limit <n>      At most n findings shown per check (default 50).
";

const REVIEW_HELP: &str = "\
aeria review [<pattern>] [--limit <n>]

Translations marked as needing review, by a game update, an agent's flag, or a
person, with their source, translation, and note, and the translations whose source
a game update removed.

  --limit <n>  At most n translations (default 100).
";

const FLAG_HELP: &str = "\
aeria flag <address>... --reason <text>

Marks translations as needing a person's review and adds \"[agent] <reason>\" to
their note, for anything an agent should not decide alone: a person's translation
that looks wrong, a choice of taste, a term to settle. Untranslated strings cannot be
flagged; tell the user about them.
";

const INIT_HELP: &str = "\
aeria init

Writes AGENTS.md and CLAUDE.md at the project root, which agent harnesses such as
Claude Code, Codex, and Hermes Agent read: they say this is an Aeria project and to
start with `aeria guide`. Text of your own in those files is kept; Aeria's part
between its markers is replaced.
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

/// A command's output: text, or one JSON value with `--json`, and warnings
/// for standard error.
pub(crate) struct Output {
    pub json: bool,
    pub text: String,
    value: Option<serde_json::Value>,
    warnings: Vec<String>,
}

impl Output {
    fn new(json: bool) -> Self {
        Self {
            json,
            text: String::new(),
            value: None,
            warnings: Vec::new(),
        }
    }

    pub(crate) fn json_value(&mut self, value: &serde_json::Value) {
        self.value = Some(value.clone());
    }

    pub(crate) fn warn(&mut self, warning: &str) {
        self.warnings.push(warning.to_owned());
    }

    fn response(self, code: i32) -> Response {
        let stdout = match self.value {
            Some(value) => serde_json::to_string_pretty(&value).unwrap_or_default() + "\n",
            None => self.text,
        };
        Response {
            stdout,
            stderr: self
                .warnings
                .iter()
                .fold(String::new(), |mut stderr, warning| {
                    let _ = writeln!(stderr, "aeria: {warning}");
                    stderr
                }),
            code,
        }
    }
}

/// One command as the client hands it over.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Request {
    /// The client's working directory; relative paths are resolved there.
    pub cwd: PathBuf,
    pub args: Vec<String>,
    /// Standard input, for `write` and `check` without a file.
    pub stdin: Option<String>,
}

/// What a command printed and its exit status.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Response {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

impl Response {
    fn failed(message: &str) -> Self {
        Self {
            stdout: String::new(),
            stderr: format!("aeria: {message}\n"),
            code: FAILED,
        }
    }
}

/// How a command reaches the project.
pub(crate) enum Access<'a> {
    /// Opens the project for this command alone.
    Direct(&'a Env),
    /// The project a server keeps open.
    Served(&'a serve::Server),
}

impl Access<'_> {
    fn read<T>(
        &self,
        start: &Path,
        read: impl FnOnce(&Project) -> Result<T, String>,
    ) -> Result<T, String> {
        match self {
            Self::Direct(env) => read(&Project::open(start, env)?),
            Self::Served(server) => server.read(read),
        }
    }

    /// A write opens the project under the write lock, so it starts from the
    /// workspace every earlier writer left.
    fn write<T>(
        &self,
        start: &Path,
        write: impl FnOnce(&mut Project) -> Result<T, String>,
    ) -> Result<T, String> {
        match self {
            Self::Direct(env) => {
                let _lock = env
                    .project_files(start)?
                    .as_ref()
                    .map(crate::sync::WriteLock::acquire)
                    .transpose()
                    .map_err(|error| {
                        format!("the project's write lock cannot be taken: {error}")
                    })?;
                write(&mut Project::open(start, env)?)
            }
            Self::Served(server) => server.write(write),
        }
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

    /// A non-negative number option.
    fn number(&self, name: &str, default: usize) -> Result<usize, String> {
        self.value(name).map_or(Ok(default), |value| {
            value
                .parse::<usize>()
                .map_err(|_| format!("--{name} {value:?} is not a number"))
        })
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

/// The translations a `write` or `check` gets: `--at`/`--text`, a file,
/// or standard input.
fn entries(args: &Arguments, request: &Request) -> Result<Vec<write::Entry>, String> {
    if let Some(address) = args.value("at") {
        let text = args.value("text").ok_or("--at needs --text")?;
        return Ok(vec![write::Entry {
            address: project::Address::parse(address)?,
            text: text.to_owned(),
        }]);
    }
    let input = match args.positional.first().map(String::as_str) {
        Some("-") | None => request
            .stdin
            .clone()
            .ok_or("give the translations in a file, on standard input, or with --at and --text")?,
        Some(path) => {
            let path = request.cwd.join(path);
            std::fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?
        }
    };
    let entries = write::parse_entries(&input)?;
    if entries.is_empty() {
        return Err("there are no translations in the input".to_owned());
    }
    Ok(entries)
}

/// The global options of a command line and where its command starts.
struct CommandLine {
    project: Option<PathBuf>,
    json: bool,
    command: Option<String>,
    rest: Vec<String>,
    /// `--help`, `--version`, or help for one command, answered locally.
    local: Option<String>,
}

fn command_line(args: &[String]) -> Result<CommandLine, String> {
    let mut line = CommandLine {
        project: None,
        json: false,
        command: None,
        rest: Vec::new(),
        local: None,
    };
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        match arg.as_str() {
            "--project" => {
                line.project = Some(PathBuf::from(
                    args.get(index + 1).ok_or("--project needs a directory")?,
                ));
                index += 2;
            }
            "--json" => {
                line.json = true;
                index += 1;
            }
            "--version" | "-V" => {
                line.local = Some(format!("aeria {}\n", env!("CARGO_PKG_VERSION")));
                return Ok(line);
            }
            "-h" | "--help" | "help" => {
                let topic = args.get(index + 1).map(String::as_str);
                line.local = Some(topic.and_then(command_help).unwrap_or(HELP).to_owned());
                return Ok(line);
            }
            other if other.starts_with("--project=") => {
                line.project = Some(PathBuf::from(&other["--project=".len()..]));
                index += 1;
            }
            _ => break,
        }
    }
    let Some(command) = args.get(index) else {
        line.local = Some(HELP.to_owned());
        return Ok(line);
    };
    line.rest = args[index + 1..]
        .iter()
        .filter(|arg| {
            if *arg == "--json" {
                line.json = true;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect();
    if line.rest.iter().any(|arg| arg == "--help" || arg == "-h") {
        line.local = Some(
            command_help(command)
                .ok_or_else(|| unknown(command))?
                .to_owned(),
        );
    }
    line.command = Some(command.clone());
    Ok(line)
}


/// Writes Aeria's part of `AGENTS.md` and `CLAUDE.md` at a project root.
///
/// # Errors
///
/// Returns a description when a file cannot be written.
pub(crate) fn write_agent_files(root: &Path) -> Result<String, String> {
    texts::init(root)
}

/// Runs the command of the process arguments and returns its exit status.
#[must_use]
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("__serve") {
        let root = args
            .get(1)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os(serve::SERVE_ROOT_VARIABLE).map(PathBuf::from));
        return root.map_or(FAILED, |root| serve::serve(&root));
    }
    let response = client(&args);
    let _ = std::io::stdout().write_all(response.stdout.as_bytes());
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(response.stderr.as_bytes());
    response.code
}

/// Answers locally what needs no project, and hands the rest to the
/// project's server, or runs it in-process when no server can be used.
fn client(args: &[String]) -> Response {
    let line = match command_line(args) {
        Ok(line) => line,
        Err(message) => return Response::failed(&message),
    };
    if let Some(text) = line.local {
        return Response {
            stdout: text,
            stderr: String::new(),
            code: OK,
        };
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let reads_input = matches!(line.command.as_deref(), Some("write" | "check"))
        && !line
            .rest
            .iter()
            .any(|arg| arg == "--at" || arg.starts_with("--at="))
        && line
            .rest
            .iter()
            .filter(|arg| !arg.starts_with("--"))
            .all(|arg| arg == "-");
    let stdin = if reads_input && !std::io::stdin().is_terminal() {
        let mut input = String::new();
        if let Err(error) = std::io::stdin().read_to_string(&mut input) {
            return Response::failed(&format!("standard input: {error}"));
        }
        Some(input)
    } else {
        None
    };
    let request = Request {
        cwd: cwd.clone(),
        args: args.to_vec(),
        stdin,
    };
    let start = cwd.join(line.project.unwrap_or_default());
    if std::env::var_os("AERIA_NO_SERVER").is_none()
        && let Ok(root) = project::find_root(&start)
        && let Some(response) = serve::request(&root, &request)
    {
        return response;
    }
    execute(&request, &Access::Direct(&Env::standalone()))
}

/// Runs one command against the project and returns what it printed.
pub(crate) fn execute(request: &Request, access: &Access<'_>) -> Response {
    let line = match command_line(&request.args) {
        Ok(line) => line,
        Err(message) => return Response::failed(&message),
    };
    if let Some(text) = line.local {
        return Response {
            stdout: text,
            stderr: String::new(),
            code: OK,
        };
    }
    let Some(command) = line.command.clone() else {
        return Response::failed("no command");
    };
    let start = request.cwd.join(line.project.clone().unwrap_or_default());
    let mut out = Output::new(line.json);
    match dispatch(&command, &line.rest, request, &start, access, &mut out) {
        Ok(code) => out.response(code),
        Err(message) => {
            let mut response = out.response(FAILED);
            response.stdout.clear();
            let _ = writeln!(response.stderr, "aeria: {message}");
            response
        }
    }
}

#[allow(clippy::too_many_lines)] // one dispatch
fn dispatch(
    command: &str,
    rest: &[String],
    request: &Request,
    start: &Path,
    access: &Access<'_>,
    out: &mut Output,
) -> Result<i32, String> {
    match command {
        "guide" => {
            Arguments::parse(rest, &[], &[])?;
            out.text = access.read(start, |project| Ok(texts::guide(project)))?;
            Ok(OK)
        }
        "brief" => {
            Arguments::parse(rest, &[], &[])?;
            out.text = access.read(start, |project| Ok(texts::brief(project)))?;
            Ok(OK)
        }
        "overview" => {
            let parsed = Arguments::parse(rest, &["limit"], &["folders", "untranslated"])?;
            let options = read::OverviewOptions {
                pattern: parsed.positional.first().cloned(),
                folders: parsed.switch("folders"),
                untranslated: parsed.switch("untranslated"),
                limit: parsed.limit(100)?,
            };
            access.read(start, |project| {
                read::overview(project, &options, out);
                Ok(())
            })?;
            Ok(OK)
        }
        "plan" => {
            let parsed = Arguments::parse(rest, &["size", "from", "limit"], &["terms"])?;
            let options = plan::PlanOptions {
                pattern: parsed
                    .positional
                    .first()
                    .ok_or("name the sheets to plan, such as BNpcName, quest/*, or cut_scene/*")?
                    .clone(),
                size: parsed.number("size", 50)?.max(1),
                from: parsed.number("from", 1)?.saturating_sub(1),
                limit: parsed.limit(200)?,
                terms: parsed.switch("terms"),
            };
            access.read(start, |project| plan::plan(project, &options, out))?;
            Ok(OK)
        }
        "read" => {
            let parsed = Arguments::parse(
                rest,
                &["rows", "from", "limit", "max-bytes"],
                &["untranslated", "no-knowledge", "no-similar"],
            )?;
            let sheet = parsed
                .positional
                .first()
                .ok_or("name the sheet to read; `aeria overview <pattern>` lists sheets")?
                .clone();
            let options = read::ReadOptions {
                sheet,
                rows: parsed.value("rows").map(row_range).transpose()?,
                untranslated: parsed.switch("untranslated"),
                from: parsed.number("from", 1)?.saturating_sub(1),
                limit: parsed
                    .value("limit")
                    .map(|_| parsed.limit(read::READ_LIMIT))
                    .transpose()?,
                max_bytes: parsed.number("max-bytes", READ_MAX_BYTES)?,
                knowledge: !parsed.switch("no-knowledge"),
                memory: !parsed.switch("no-similar"),
            };
            access.read(start, |project| read::read(project, &options, out))?;
            Ok(OK)
        }
        "check" => {
            let parsed = Arguments::parse(rest, &["at", "text"], &[])?;
            let entries = entries(&parsed, request)?;
            let clean = access.read(start, |project| write::check(project, &entries, out))?;
            Ok(if clean { OK } else { PARTIAL })
        }
        "write" => {
            let parsed = Arguments::parse(rest, &["at", "text"], &["needs-review"])?;
            let entries = entries(&parsed, request)?;
            let options = write::WriteOptions {
                needs_review: parsed.switch("needs-review"),
            };
            let clean = access.write(start, |project| {
                write::write(project, &entries, &options, out)
            })?;
            Ok(if clean { OK } else { PARTIAL })
        }
        "find" => {
            let parsed = Arguments::parse(rest, &["in", "sheet", "limit"], &[])?;
            let text = parsed.positional.join(" ");
            if text.trim().is_empty() {
                return Err("give the text to find".to_owned());
            }
            let within = match parsed.value("in").unwrap_or("source") {
                "source" => read::FindIn::Source,
                "translation" | "translations" | "target" => read::FindIn::Translation,
                other => return Err(format!("--in {other:?}: use source or translation")),
            };
            let options = read::FindOptions {
                text,
                within,
                sheet: parsed.value("sheet").map(str::to_owned),
                limit: parsed.limit(30)?,
            };
            access.read(start, |project| read::find(project, &options, out))?;
            Ok(OK)
        }
        "knowledge" => {
            Arguments::parse(rest, &[], &[])?;
            let clean = access.read(start, |project| Ok(read::knowledge(project, out)))?;
            Ok(if clean { OK } else { PARTIAL })
        }
        "audit" => {
            let parsed = Arguments::parse(rest, &["check", "limit"], &[])?;
            let options = audit::AuditOptions {
                pattern: parsed.positional.first().cloned(),
                checks: parsed
                    .value("check")
                    .map(|names| {
                        names
                            .split(',')
                            .filter(|name| !name.trim().is_empty())
                            .map(audit::Check::parse)
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
                limit: parsed.limit(50)?,
            };
            let clean = access.read(start, |project| Ok(audit::audit(project, &options, out)))?;
            Ok(if clean { OK } else { PARTIAL })
        }
        "review" => {
            let parsed = Arguments::parse(rest, &["limit"], &[])?;
            let options = audit::ReviewOptions {
                pattern: parsed.positional.first().cloned(),
                limit: parsed.limit(100)?,
            };
            access.read(start, |project| {
                audit::review(project, &options, out);
                Ok(())
            })?;
            Ok(OK)
        }
        "flag" => {
            let parsed = Arguments::parse(rest, &["reason"], &[])?;
            let reason = parsed
                .value("reason")
                .filter(|reason| !reason.trim().is_empty())
                .ok_or("say why with --reason")?
                .to_owned();
            if parsed.positional.is_empty() {
                return Err("name the strings to flag by their addresses".to_owned());
            }
            let addresses = parsed
                .positional
                .iter()
                .map(|address| project::Address::parse(address))
                .collect::<Result<Vec<_>, _>>()?;
            let clean = access.write(start, |project| {
                Ok(audit::flag(project, &addresses, &reason, out))
            })?;
            Ok(if clean { OK } else { PARTIAL })
        }
        "init" => {
            Arguments::parse(rest, &[], &[])?;
            let root = project::find_root(start)?;
            out.text = texts::init(&root)?;
            Ok(OK)
        }
        other => Err(unknown(other)),
    }
}

fn unknown(command: &str) -> String {
    format!("unknown command {command:?}; `aeria --help` lists the commands")
}

fn command_help(command: &str) -> Option<&'static str> {
    Some(match command {
        "overview" => OVERVIEW_HELP,
        "read" => READ_HELP,
        "plan" => PLAN_HELP,
        "write" => WRITE_HELP,
        "check" => CHECK_HELP,
        "find" => FIND_HELP,
        "knowledge" => KNOWLEDGE_HELP,
        "audit" => AUDIT_HELP,
        "review" => REVIEW_HELP,
        "flag" => FLAG_HELP,
        "init" => INIT_HELP,
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
            "audit",
            "review",
            "flag",
            "init",
        ] {
            assert!(command_help(command).is_some(), "{command}");
            assert!(HELP.contains(command), "{command}");
        }
        let env = Env::default();
        let run = |args: &[&str]| {
            execute(
                &Request {
                    cwd: PathBuf::from("."),
                    args: strings(args),
                    stdin: None,
                },
                &Access::Direct(&env),
            )
        };
        assert_eq!(run(&["--help"]).code, OK);
        assert!(run(&["--help"]).stdout.contains("Commands:"));
        assert_eq!(run(&["nonsense"]).code, FAILED);
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
        let response = execute(
            &Request {
                cwd: PathBuf::from("."),
                args: all,
                stdin: None,
            },
            &Access::Direct(env),
        );
        assert!(response.code != FAILED, "{}", response.stderr);
        response.code
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
