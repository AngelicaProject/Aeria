//! The `aeria` command: the project's files for people and agents.
//!
//! A project is `aeria.json` and the PO files of `po/`, kept in Git (see
//! `docs/architecture/po-project.md`). Agents such as Claude Code, Codex, or
//! Hermes Agent translate by editing `po/`; the command makes a project
//! (`init`), checks the files (`check`), and brings them to a new game version
//! (`update`).
//!
//! A command is a thin client: it hands its arguments to the project's server
//! (see [`serve`]), which keeps the installed game open between commands, and
//! prints what the server answers. Help and version are answered without a
//! server; `AERIA_NO_SERVER` runs a command in-process.

mod check;
mod git;
mod project;
mod serve;
mod texts;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use project::{Env, Project};
use serde::{Deserialize, Serialize};

/// Exit status: done.
const OK: i32 = 0;
/// Exit status: done, but the files or the knowledge have problems.
const PARTIAL: i32 = 1;
/// Exit status: the command could not run.
const FAILED: i32 = 2;

const HELP: &str = "\
aeria — the Aeria project of a translation of FINAL FANTASY XIV

Usage: aeria [--project <dir>] [--json] <command> [options]

Commands:
  init        Make a project: every string of the installed game as PO files in po/.
  check       Check the files of po/ and the project knowledge; list each problem.
  update      Bring po/ to the installed game version, as one commit.

po/README.md describes the files, the rules of a translation, and working with Git.

Options:
  --project <dir>  The project, or a directory inside it (default: the current directory).
  --json           Machine-readable output.
  -h, --help       Help; `aeria <command> --help` for one command.
  --version        The version of Aeria.

Exit status: 0 done, 1 done with problems, 2 error.
";

const INIT_HELP: &str = "\
aeria init --language <tag> [--source <language>]

Makes the directory a project: aeria.json, and in po/ every translatable string of the
installed game as gettext PO files, with the other client languages, speakers, and macro
legends as comments; po/README.md with the layout and the rules; aeria-knowledge/ with
its files and no entries yet; AGENTS.md and CLAUDE.md, which agent harnesses read. Makes the directory a Git repository if it is not
one and commits the project. In an existing project it only adds the files around po/
that are missing, rewrites Aeria's sections and po/README.md, and commits those.

  --language <tag>     The target language, such as ru, es, or pt-BR.
  --source <language>  The source language: en (default), ja, de, or fr.
";

const CHECK_HELP: &str = "\
aeria check [--all]

Checks the PO files changed since the last commit (every file with --all, or outside
a Git repository) and the project knowledge, and lists each problem as file:line: the
PO format, Git conflict markers, a msgctxt or msgid that is not the installed game's,
a file made for another game version, and translations that break the macros, have a
line break the source does not, use a forbidden variant of a term, or write both
genders at once. Advice, such as a term that seems missing, is listed separately.
Changes no translation. Exit status 1 when there are problems.

  --all  Every file, not only those changed since the last commit.
";

const UPDATE_HELP: &str = "\
aeria update

Brings po/ to the installed game version: every file is made again from the game and
each translation is carried over by msgctxt. A translation whose source changed is
marked fuzzy, with the source it was written for as #| msgid; one whose string the game
no longer has becomes an obsolete entry (#~). Runs only when po/ and aeria.json have no
changes that are not committed, and commits the result, so the update is one commit to
review and revert.
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
    // A project that does not exist yet has no server.
    if line.command.as_deref() != Some("init")
        && std::env::var_os("AERIA_NO_SERVER").is_none()
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
        "init" => {
            let parsed = Arguments::parse(rest, &["language", "source"], &[])?;
            let language = parsed
                .value("language")
                .ok_or("name the target language with --language, such as --language ru")?;
            let env = match access {
                Access::Direct(env) => (*env).clone(),
                Access::Served(_) => Env::standalone(),
            };
            init(
                start,
                &env,
                language,
                parsed.value("source").unwrap_or("en"),
                out,
            )?;
            Ok(OK)
        }
        "check" => {
            let parsed = Arguments::parse(rest, &[], &["all"])?;
            let options = check::CheckOptions {
                all: parsed.switch("all"),
            };
            let clean = access.read(start, |project| check::check(project, &options, out))?;
            Ok(if clean { OK } else { PARTIAL })
        }
        "update" => {
            Arguments::parse(rest, &[], &[])?;
            access.read(start, |project| update(project, out))?;
            Ok(OK)
        }
        other => Err(unknown(other)),
    }
}

/// Whether a language tag is plain enough to be one: letters, digits, and
/// hyphens.
fn language_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 35
        && tag
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// The project files `init` commits.
const PROJECT_FILES: [&str; 6] = [
    aeria_po::SETTINGS_FILE,
    aeria_po::PO_DIR,
    aeria_knowledge::KNOWLEDGE_DIR,
    ".gitattributes",
    "AGENTS.md",
    "CLAUDE.md",
];

/// `.gitattributes` keeps PO files LF on every checkout.
fn keep_lf(root: &Path) -> Result<(), String> {
    let path = root.join(".gitattributes");
    let rule = "*.po text eol=lf";
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current.lines().any(|line| line.trim() == rule) {
        return Ok(());
    }
    let mut next = current.trim_end().to_owned();
    if !next.is_empty() {
        next.push('\n');
    }
    next.push_str(rule);
    next.push('\n');
    std::fs::write(&path, next).map_err(|error| format!("{}: {error}", path.display()))
}

fn init(
    start: &Path,
    env: &Env,
    language: &str,
    source_language: &str,
    out: &mut Output,
) -> Result<(), String> {
    if !language_tag(language) {
        return Err(format!(
            "{language:?} is not a language tag, such as ru, es, or pt-BR"
        ));
    }
    std::fs::create_dir_all(start).map_err(|error| format!("{}: {error}", start.display()))?;
    if let Ok(root) = project::find_root(start) {
        return complete(&root, env, out);
    }
    let root =
        std::fs::canonicalize(start).map_err(|error| format!("{}: {error}", start.display()))?;
    let source = env.open_game(source_language)?;
    let files = aeria_po::create(&root, &source, language, Project::threads())
        .map_err(|error| error.to_string())?;
    let project = Project::open(&root, env)?;
    aeria_knowledge::create_empty(&root)?;
    texts::write_readme(&project)?;
    keep_lf(&root)?;
    texts::init(&root)?;
    git::init(&root)?;
    let version = source.version().to_string();
    let committed = git::commit(
        &root,
        &PROJECT_FILES,
        &format!("Make the project for game version {version}"),
    );
    let _ = writeln!(
        out.text,
        "{}: a project for game version {version}, {source_language} → {language}: {} files in {}/",
        project.root_display(),
        fmt_count(files),
        aeria_po::PO_DIR
    );
    match committed {
        Ok(Some(hash)) => {
            let _ = writeln!(out.text, "committed as {hash}");
        }
        Ok(None) => {}
        Err(error) => out.warn(&format!(
            "the project was made but not committed ({error}); commit it before translating"
        )),
    }
    Ok(())
}

/// The files around `po/` that a project made by an earlier version of
/// Aeria may lack: adds what is missing, changes nothing that exists but
/// Aeria's sections and `po/README.md`, and commits them.
fn complete(root: &Path, env: &Env, out: &mut Output) -> Result<(), String> {
    let project = Project::open(root, env)?;
    let added = aeria_knowledge::create_empty(root)?;
    texts::write_readme(&project)?;
    keep_lf(root)?;
    texts::init(root)?;
    let files: Vec<&str> = PROJECT_FILES
        .iter()
        .copied()
        .filter(|path| *path != aeria_po::PO_DIR && *path != aeria_po::SETTINGS_FILE)
        .chain([aeria_po::README_PATH])
        .collect();
    let _ = writeln!(
        out.text,
        "{} is already an Aeria project; added what it lacked: {}",
        project.root_display(),
        if added.is_empty() {
            "nothing".to_owned()
        } else {
            added.join(", ")
        }
    );
    if git::is_repository(root) {
        match git::commit(root, &files, "Add the project files Aeria writes") {
            Ok(Some(hash)) => {
                let _ = writeln!(out.text, "committed as {hash}");
            }
            Ok(None) => {}
            Err(error) => out.warn(&format!("not committed ({error}); commit these files")),
        }
    }
    Ok(())
}

fn update(project: &Project, out: &mut Output) -> Result<(), String> {
    let root = &project.root;
    let paths = [aeria_po::PO_DIR, aeria_po::SETTINGS_FILE];
    let repository = git::is_repository(root);
    if repository && !git::committed(root, &paths)? {
        return Err(format!(
            "{} or {} has changes that are not committed: commit them first, so the update is one commit of its own",
            aeria_po::PO_DIR,
            aeria_po::SETTINGS_FILE
        ));
    }
    let version = project.source.version().to_string();
    let updated = aeria_po::update(root, &project.source, Project::threads())
        .map_err(|error| error.to_string())?;
    texts::write_readme(project)?;
    let _ = writeln!(
        out.text,
        "game version {version}: {} files changed · {} translations fuzzy · {} obsolete",
        fmt_count(updated.files),
        fmt_count(updated.fuzzy),
        fmt_count(updated.obsolete)
    );
    if repository {
        let commit = git::commit(root, &paths, &format!("Update to game version {version}"))?;
        let _ = writeln!(
            out.text,
            "{}",
            commit.map_or_else(
                || "nothing changed".to_owned(),
                |hash| format!("committed as {hash}")
            )
        );
    }
    Ok(())
}

fn unknown(command: &str) -> String {
    format!("unknown command {command:?}; `aeria --help` lists the commands")
}

fn command_help(command: &str) -> Option<&'static str> {
    Some(match command {
        "init" => INIT_HELP,
        "check" => CHECK_HELP,
        "update" => UPDATE_HELP,
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
        let parsed = Arguments::parse(
            &strings(&["x", "--all", "--language=ru"]),
            &["language"],
            &["all"],
        )
        .expect("parse");
        assert_eq!(parsed.positional, ["x"]);
        assert!(parsed.switch("all"));
        assert_eq!(parsed.value("language"), Some("ru"));
        assert!(Arguments::parse(&strings(&["--nope"]), &[], &[]).is_err());
        assert!(language_tag("pt-BR"));
        assert!(!language_tag("ru ru"));

        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(1234), "1 234");
        assert_eq!(fmt_count(860_000), "860 000");
    }

    #[test]
    fn every_command_has_help() {
        for command in ["init", "check", "update"] {
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

    fn run_in(root: &Path, env: &Env, args: &[&str]) -> Response {
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

    /// Replaces one line of the entry with `context` in `po/Addon.po`.
    fn edit_entry(root: &Path, context: &str, field: &str, value: &str) {
        let path = root.join("po").join("Addon.po");
        let text = std::fs::read_to_string(&path).expect("file");
        let at = text
            .find(&format!("msgctxt \"{context}\"\n"))
            .expect("entry");
        let start = at + text[at..].find(&format!("{field} ")).expect("field");
        let end = start + text[start..].find('\n').expect("line end");
        let next = format!("{}{field} \"{value}\"{}", &text[..start], &text[end..]);
        std::fs::write(&path, next).expect("write");
    }

    fn git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn a_project_is_made_checked_and_updated() {
        let directory = tempfile::tempdir().expect("directory");
        let game = crate::test_support::test_game();
        let env = Env {
            data_dir: Some(directory.path().join("data")),
            game_path: Some(game.path().to_string_lossy().into_owned()),
        };
        let root = directory.path().join("project");
        std::fs::create_dir_all(&root).expect("root");
        // The commit of `init` needs an identity; the test's own repository
        // has one before `init` runs.
        git(&root, &["init", "--quiet"]);
        git(&root, &["config", "user.name", "Test"]);
        git(&root, &["config", "user.email", "test@example.com"]);

        let made = run_in(&root, &env, &["init", "--language", "ru"]);
        // Running it again only adds what is missing.
        std::fs::remove_file(root.join("aeria-knowledge").join("style.md")).expect("remove");
        let again = run_in(&root, &env, &["init", "--language", "ru"]);
        assert!(
            again.stdout.contains("aeria-knowledge/style.md"),
            "{}",
            again.stdout
        );
        assert!(root.join("aeria-knowledge").join("style.md").is_file());
        assert_eq!(made.code, OK, "{}{}", made.stdout, made.stderr);
        assert!(root.join("aeria.json").is_file());
        assert!(root.join("po").join("README.md").is_file());
        assert!(root.join("po").join("Addon.po").is_file());
        assert!(root.join("aeria-knowledge").join("terms.csv").is_file());
        assert!(
            std::fs::read_to_string(root.join("AGENTS.md"))
                .expect("agents")
                .contains("po/README.md")
        );
        assert_eq!(run_in(&root, &env, &["check"]).code, OK);

        // A good translation passes; a broken one and a changed msgid are
        // reported with their file and line.
        edit_entry(&root, "Addon:ADDON_OK:1", "msgstr", "ОК");
        let checked = run_in(&root, &env, &["check"]);
        assert_eq!(checked.code, OK, "{}", checked.stdout);
        edit_entry(&root, "Addon:ADDON_CANCEL:1", "msgstr", "<i>Отмена");
        let checked = run_in(&root, &env, &["check"]);
        assert_eq!(checked.code, PARTIAL);
        assert!(
            checked.stdout.contains("po/Addon.po:"),
            "{}",
            checked.stdout
        );
        edit_entry(&root, "Addon:ADDON_CANCEL:1", "msgstr", "Отмена");
        edit_entry(&root, "Addon:ADDON_CANCEL:1", "msgid", "Something else");
        let checked = run_in(&root, &env, &["check"]);
        assert!(
            checked.stdout.contains("msgid is not the game's text"),
            "{}",
            checked.stdout
        );

        // An update needs committed files, then changes nothing on the same
        // game version.
        let refused = execute(
            &Request {
                cwd: PathBuf::from("."),
                args: strings(&["--project", &root.to_string_lossy(), "update"]),
            },
            &Access::Direct(&env),
        );
        assert_eq!(refused.code, FAILED);
        assert!(
            refused.stderr.contains("not committed"),
            "{}",
            refused.stderr
        );
        edit_entry(&root, "Addon:ADDON_CANCEL:1", "msgid", "Cancel");
        git(&root, &["add", "--all"]);
        git(&root, &["commit", "--quiet", "-m", "Translate"]);
        let updated = run_in(&root, &env, &["update"]);
        assert!(
            updated.stdout.contains("nothing changed"),
            "{}",
            updated.stdout
        );
    }
}
