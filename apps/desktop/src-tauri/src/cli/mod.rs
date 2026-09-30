//! The `aeria` command: the project for external agents.
//!
//! Agents such as Claude Code, Codex, or Hermes Agent translate the game's
//! text in `game/`, which `aeria corpus` makes, and save it with
//! `aeria check` (see `docs/architecture/agents.md`). It works on the project
//! files directly; an open Aeria window reloads after its writes.
//!
//! A command is a thin client: it hands its arguments to the project's server
//! (see [`serve`]), which keeps the game, the workspace, and every cache open
//! between commands, and prints what the server answers. Help and version
//! are answered without a server; `AERIA_NO_SERVER` runs a command
//! in-process.

mod corpus;
mod project;
mod serve;
mod texts;
mod write;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use project::{Env, Project};
use serde::{Deserialize, Serialize};

/// Exit status: done.
const OK: i32 = 0;
/// Exit status: done, but some translations were not saved or the knowledge
/// has problems.
const PARTIAL: i32 = 1;
/// Exit status: the command could not run.
const FAILED: i32 = 2;

const HELP: &str = "\
aeria — the text of FINAL FANTASY XIV for translating an Aeria project

Usage: aeria [--project <dir>] [--json] <command> [options]

Commands:
  corpus      Write the whole game text into game/ as PO files to read and translate.
  check       Save the translations changed in game/ and bring it up to date.
  init        Write AGENTS.md and CLAUDE.md so agent harnesses find game/.

game/README.md describes the files and the rules of a translation.

Options:
  --project <dir>  The project, or a directory inside it (default: the current directory).
  --json           Machine-readable output.
  -h, --help       Help; `aeria <command> --help` for one command.
  --version        The version of Aeria.

Exit status: 0 done, 1 done with translations not saved or knowledge problems, 2 error.
";

const CHECK_HELP: &str = "\
aeria check

Saves every msgstr changed in the PO files of game/ that passes the checks: its macros
and structure against the source, forbidden term variants, and forms that write both
genders at once. Each problem is listed as file:line; advice, such as a term that seems
missing, comes with saved ones. A translation that was not saved stays in its file, and
the next check reads it again. A translation a person wrote or reviewed (`#, keep`), one
that changed in the project after its file was made, and one written for a source the
game has since changed are never saved. Problems in aeria-knowledge/ are listed too.
Then the files are brought up to date with the project.
";

const CORPUS_HELP: &str = "\
aeria corpus [<pattern>] [--force]

Writes the whole game text into game/ in the project, as gettext PO files to read,
search, and translate with any tool: a quest or cutscene is one file in play order,
another sheet one file or a folder of files of 200 strings. Each entry has the address
(msgctxt), the source (msgid), the translation (msgstr), and the other client languages,
the speaker, and what the macros do as comments; game/README.md explains the layout and
the rules. `aeria check` saves what changed. game/ is added to .gitignore: it is the
game's text and is never committed. Files with changes not saved yet are kept.

  <pattern>  Only the matching sheets, such as BNpcName or quest/*.
  --force    Also replace files with changes `aeria check` has not saved.
";

const INIT_HELP: &str = "\
aeria init

Writes AGENTS.md and CLAUDE.md at the project root, which agent harnesses such as
Claude Code, Codex, and Hermes Agent read: they say this is an Aeria project, that the
game's text is in game/, and how translations are saved. Text of your own in those files
is kept; Aeria's part between its markers is replaced.
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
                let mut project = Project::open(start, env)?;
                let result = write(&mut project);
                // game/ shows what was written; a file that cannot be
                // written now is written by the next check.
                let _ = corpus::refresh(&project);
                result
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
    let request = Request {
        cwd: cwd.clone(),
        args: args.to_vec(),
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
    match dispatch(&command, &line.rest, &start, access, &mut out) {
        Ok(code) => out.response(code),
        Err(message) => {
            let mut response = out.response(FAILED);
            response.stdout.clear();
            let _ = writeln!(response.stderr, "aeria: {message}");
            response
        }
    }
}

fn dispatch(
    command: &str,
    rest: &[String],
    start: &Path,
    access: &Access<'_>,
    out: &mut Output,
) -> Result<i32, String> {
    match command {
        "corpus" => {
            let parsed = Arguments::parse(rest, &[], &["force"])?;
            let options = corpus::CorpusOptions {
                pattern: parsed.positional.first().cloned(),
                force: parsed.switch("force"),
            };
            access.read(start, |project| corpus::corpus(project, &options, out))?;
            Ok(OK)
        }
        "check" => {
            Arguments::parse(rest, &[], &[])?;
            let clean = access.write(start, |project| corpus::check_files(project, out))?;
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
        "check" => CHECK_HELP,
        "corpus" => CORPUS_HELP,
        "init" => INIT_HELP,
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
    fn arguments_and_counts_parse() {
        let parsed =
            Arguments::parse(&strings(&["quest/*", "--force"]), &[], &["force"]).expect("parse");
        assert_eq!(parsed.positional, ["quest/*"]);
        assert!(parsed.switch("force"));
        assert!(Arguments::parse(&strings(&["--nope"]), &[], &[]).is_err());

        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(1234), "1 234");
        assert_eq!(fmt_count(860_000), "860 000");
    }

    #[test]
    fn every_command_has_help() {
        for command in ["corpus", "check", "init"] {
            assert!(command_help(command).is_some(), "{command}");
            assert!(HELP.contains(command), "{command}");
        }
        let env = Env::default();
        let run = |args: &[&str]| {
            execute(
                &Request {
                    cwd: PathBuf::from("."),
                    args: strings(args),
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

    fn run_in(root: &std::path::Path, env: &Env, args: &[&str]) -> Response {
        let mut all = vec!["--project".to_owned(), root.to_string_lossy().into_owned()];
        all.extend(strings(args));
        let response = execute(
            &Request {
                cwd: PathBuf::from("."),
                args: all,
            },
            &Access::Direct(env),
        );
        assert!(response.code != FAILED, "{}", response.stderr);
        response
    }

    /// Replaces the msgstr of the entry with `address` in a corpus file.
    fn translate_in_file(root: &std::path::Path, address: &str, translation: &str) {
        let path = root.join("game").join("Addon.po");
        let text = std::fs::read_to_string(&path).expect("file");
        let context = format!("msgctxt \"{address}\"\n");
        let at = text.find(&context).expect("entry");
        let msgstr = at + text[at..].find("msgstr ").expect("msgstr");
        let end = msgstr + text[msgstr..].find('\n').expect("line end");
        let next = format!(
            "{}msgstr \"{translation}\"{}",
            &text[..msgstr],
            &text[end..]
        );
        std::fs::write(&path, next).expect("write");
    }

    #[test]
    fn agents_translate_in_files_and_never_replace_a_persons_translation() {
        let (_directory, _game, env, root) = project();
        run_in(&root, &env, &["corpus", "Addon"]);
        assert!(root.join("game").join("README.md").is_file());
        let at = "Addon:1:0:1";
        let address = project::Address::parse(at).expect("address");
        let now = |env: &Env| {
            let project = Project::open(&root, env).expect("open");
            let ledger = project.ledger();
            project::current(&project.session, ledger.as_ref(), &address)
        };

        translate_in_file(&root, at, "ОК");
        assert_eq!(run_in(&root, &env, &["check"]).code, OK);
        let written = now(&env).expect("translated");
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
        translate_in_file(&root, at, "Готово");
        assert_eq!(run_in(&root, &env, &["check"]).code, OK);
        assert_eq!(now(&env).expect("translated").target, "Готово");

        // A person's edit reaches the file and makes the string theirs.
        let mut project = Project::open(&root, &env).expect("open");
        project
            .session
            .set_target(&address.binding(), "Подтвердить")
            .expect("person");
        drop(project);
        assert_eq!(run_in(&root, &env, &["check"]).code, OK);
        let file = std::fs::read_to_string(root.join("game").join("Addon.po")).expect("file");
        assert!(file.contains("#, keep\nmsgctxt \"Addon:1:0:1\""), "{file}");
        assert!(file.contains("msgstr \"Подтвердить\""), "{file}");
        translate_in_file(&root, at, "ОК");
        assert_eq!(run_in(&root, &env, &["check"]).code, PARTIAL);
        assert_eq!(now(&env).expect("translated").target, "Подтвердить");

        // A broken translation is not saved and is reported with its line.
        translate_in_file(&root, at, "Подтвердить");
        translate_in_file(&root, "Addon:2:0:1", "<i>Отмена");
        let response = run_in(&root, &env, &["check"]);
        assert_eq!(response.code, PARTIAL);
        assert!(
            response.stdout.contains("game/Addon.po:"),
            "{}",
            response.stdout
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
