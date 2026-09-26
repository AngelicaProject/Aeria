//! `aeria-check`: validates an Aeria translation repository in CI.
//!
//! ```text
//! aeria-check [integrity|translations|merge|all] [--project DIR] [--base REV]
//! ```
//!
//! Exits 0 when no stage found an error (warnings allowed), 1 when one did,
//! and 2 for invalid arguments. Inside GitHub Actions findings become
//! annotations and each stage adds a section to the job summary.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use aeria_check::{Finding, Severity, Stage, StageReport, run_stage};

const USAGE: &str =
    "usage: aeria-check [integrity|translations|merge|all] [--project DIR] [--base REV]
       aeria-check merge-driver BASE OURS THEIRS PATH

Stages:
  integrity     conflict markers, the manifest, and project settings
  translations  every unit and translation, and canonical files
  merge         compared with --base: source or language changes, removed units
  all           every stage (default)

Options:
  --project DIR  the folder that contains .aeria/ (default: .)
  --base REV     the revision the change merges into, for the merge stage
  --version      print the version";

struct Arguments {
    stages: Vec<Stage>,
    project: PathBuf,
    base: Option<String>,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Arguments>, String> {
    let mut stages = None;
    let mut project = PathBuf::from(".");
    let mut base = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--version" | "-V" => {
                println!("aeria-check {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--project" => project = args.next().ok_or("--project needs a folder")?.into(),
            "--base" => base = Some(args.next().ok_or("--base needs a revision")?),
            "integrity" | "translations" | "merge" | "all" if stages.is_none() => {
                stages = Some(match arg.as_str() {
                    "integrity" => vec![Stage::Integrity],
                    "translations" => vec![Stage::Translations],
                    "merge" => vec![Stage::Merge],
                    _ => vec![Stage::Integrity, Stage::Translations, Stage::Merge],
                });
            }
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }
    Ok(Some(Arguments {
        stages: stages.unwrap_or_else(|| vec![Stage::Integrity, Stage::Translations, Stage::Merge]),
        project,
        base,
    }))
}

/// Escapes a value for a GitHub Actions workflow command.
fn escape(value: &str, property: bool) -> String {
    let value = value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A");
    if property {
        value.replace(':', "%3A").replace(',', "%2C")
    } else {
        value
    }
}

fn annotation(finding: &Finding, prefix: &str) -> String {
    let command = match finding.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Notice => "notice",
    };
    let mut properties = vec![format!("title={}", escape("Aeria check", true))];
    if let Some(path) = &finding.path {
        properties.push(format!("file={}", escape(&format!("{prefix}{path}"), true)));
    }
    if let Some(line) = finding.line {
        properties.push(format!("line={line}"));
    }
    format!(
        "::{command} {}::{}",
        properties.join(","),
        escape(&finding.message, false)
    )
}

fn summary(report: &StageReport) -> String {
    let status = if report.failed() {
        "❌ failed"
    } else if report
        .findings
        .iter()
        .any(|finding| finding.severity == Severity::Warning)
    {
        "⚠️ passed with warnings"
    } else {
        "✅ passed"
    };
    let mut text = format!("### {} — {status}\n\n", report.stage.title());
    for finding in &report.findings {
        let icon = match finding.severity {
            Severity::Error => "❌",
            Severity::Warning => "⚠️",
            Severity::Notice => "ℹ️",
        };
        let _ = writeln!(text, "- {icon} {finding}");
    }
    text.push('\n');
    text
}

/// `merge-driver BASE OURS THEIRS PATH`: Git's merge driver for unit shards
/// (`%O %A %B %P`). Exits 0 for a clean merge, 1 when units conflict (they
/// are marked in the file), and 2 when a version is not a valid shard, which
/// leaves the local version for Git to report as a conflict.
fn merge_driver(args: &[String]) -> ExitCode {
    let [base, ours, theirs, path] = args else {
        eprintln!("aeria-check: merge-driver needs BASE OURS THEIRS PATH\n\n{USAGE}");
        return ExitCode::from(2);
    };
    match aeria_git::run_merge_driver(base.as_ref(), ours.as_ref(), theirs.as_ref(), path) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(conflicts) => {
            eprintln!(
                "aeria-check: {conflicts} translation unit(s) in {path} changed differently on both sides; keep one version of each marked unit"
            );
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("aeria-check: cannot merge {path} per unit: {error}");
            ExitCode::from(2)
        }
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("merge-driver") {
        return merge_driver(&raw[1..]);
    }
    let arguments = match parse(raw.into_iter()) {
        Ok(Some(arguments)) => arguments,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("aeria-check: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let github = std::env::var_os("GITHUB_ACTIONS").is_some_and(|value| value == "true");
    // Annotations name files relative to the repository, not the project.
    let prefix = if github {
        std::process::Command::new("git")
            .current_dir(&arguments.project)
            .args(["rev-parse", "--show-prefix"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let project = std::fs::canonicalize(&arguments.project).unwrap_or(arguments.project);

    let mut failed = false;
    let mut markdown = String::new();
    for stage in arguments.stages {
        let report = run_stage(stage, &project, arguments.base.as_deref());
        println!("== {} ({})", stage.title(), stage.name());
        for finding in &report.findings {
            if github {
                println!("{}", annotation(finding, &prefix));
            } else {
                let label = match finding.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                    Severity::Notice => "note",
                };
                println!("{label}: {finding}");
            }
        }
        println!(
            "{}",
            if report.failed() {
                "stage failed"
            } else {
                "stage passed"
            }
        );
        failed |= report.failed();
        markdown.push_str(&summary(&report));
    }
    if github
        && let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY")
        && let Ok(mut file) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
    {
        let _ = file.write_all(markdown.as_bytes());
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> impl Iterator<Item = String> {
        values
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn arguments_pick_stages_project_and_base() {
        let parsed = parse(args(&["merge", "--project", "sub", "--base", "HEAD^1"]))
            .expect("parse")
            .expect("arguments");
        assert_eq!(parsed.stages, [Stage::Merge]);
        assert_eq!(parsed.project, PathBuf::from("sub"));
        assert_eq!(parsed.base.as_deref(), Some("HEAD^1"));
        assert_eq!(
            parse(args(&[]))
                .expect("parse")
                .expect("arguments")
                .stages
                .len(),
            3
        );
        assert!(parse(args(&["integrity", "merge"])).is_err());
        assert!(parse(args(&["--base"])).is_err());
    }

    #[test]
    fn annotations_escape_their_properties_and_message() {
        let finding = Finding {
            severity: Severity::Error,
            path: Some(".aeria/units/00.jsonl".to_owned()),
            line: Some(3),
            message: "bad: 50%\nnext".to_owned(),
        };
        assert_eq!(
            annotation(&finding, "project/"),
            "::error title=Aeria check,file=project/.aeria/units/00.jsonl,line=3::bad: 50%25%0Anext"
        );
    }
}
