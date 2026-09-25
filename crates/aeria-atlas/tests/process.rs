use std::path::PathBuf;

use aeria_atlas::{
    AtlasEncodeRunner, AtlasError, AtlasPackageRequest, AtlasPackageRunner, CancellationToken,
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-atlas"))
}

fn request(scenario: &str) -> (AtlasPackageRequest, tempfile::TempDir) {
    let temp = tempfile::tempdir().expect("temporary directory");
    let output = temp.path().join("source.hsp");
    (
        AtlasPackageRequest {
            executable_path: fixture(),
            game_path: temp.path().join(scenario),
            source_language: "en".to_owned(),
            output_path: output,
        },
        temp,
    )
}

fn run(scenario: &str) -> Result<(), AtlasError> {
    let (request, _temp) = request(scenario);
    AtlasPackageRunner::default().run(&request, |_| {}, &CancellationToken::default())?;
    Ok(())
}

#[test]
fn valid_protocol_succeeds() {
    assert!(run("success").is_ok());
}

#[test]
fn missing_completed_is_protocol_failure() {
    assert!(matches!(
        run("no-completed"),
        Err(AtlasError::Protocol { .. })
    ));
}

#[test]
fn completed_with_nonzero_exit_fails() {
    assert!(matches!(
        run("completed-nonzero"),
        Err(AtlasError::Exit { .. })
    ));
}

#[test]
fn failed_event_fails() {
    assert!(matches!(run("failed"), Err(AtlasError::Failed { .. })));
}

#[test]
fn malformed_and_unknown_events_fail_protocol() {
    for scenario in [
        "malformed",
        "unknown",
        "wrong-version",
        "duplicate-started",
        "after-terminal",
    ] {
        assert!(
            matches!(run(scenario), Err(AtlasError::Protocol { .. })),
            "{scenario}"
        );
    }
}

#[test]
fn stderr_is_bounded() {
    let (request, _temp) = request("large-stderr");
    let error = AtlasPackageRunner::new(1024)
        .run(&request, |_| {}, &CancellationToken::default())
        .expect_err("fixture does not emit a completed event");
    assert!(error.stderr_tail().len() <= 1024);
}

#[test]
fn cancellation_terminates_and_awaits_child() {
    let (request, _temp) = request("cancel");
    let (token, handle) = CancellationToken::new();
    let thread =
        std::thread::spawn(move || AtlasPackageRunner::default().run(&request, |_| {}, &token));
    std::thread::sleep(std::time::Duration::from_millis(100));
    handle.cancel();
    let result = thread.join().expect("runner thread");
    assert!(matches!(result, Err(AtlasError::Cancelled { .. })));
}

fn encode(
    macros: &[&str],
) -> Result<Vec<Result<Vec<u8>, aeria_atlas::EncodeRejection>>, AtlasError> {
    let temp = tempfile::Builder::new()
        .prefix("Анна Иванова ")
        .tempdir()
        .expect("temporary directory");
    let runner = AtlasEncodeRunner::new(fixture(), temp.path().join("encode work"));
    let result = runner.encode(macros, &CancellationToken::default());
    let leftovers = std::fs::read_dir(temp.path().join("encode work")).map_or(0, Iterator::count);
    assert_eq!(leftovers, 0, "request and result files are removed");
    result
}

#[test]
fn encode_returns_one_result_per_string_in_order() {
    let results = encode(&["Привет", "reject", "Мир"]).expect("encode succeeds");
    assert_eq!(results[0], Ok("Привет".as_bytes().to_vec()));
    assert_eq!(results[1].as_ref().unwrap_err().code, "invalidMacro");
    assert_eq!(results[2], Ok("Мир".as_bytes().to_vec()));
}

#[test]
fn encode_failure_and_missing_results_are_errors() {
    assert!(matches!(encode(&["crash"]), Err(AtlasError::Exit { .. })));
    assert!(matches!(
        encode(&["one", "short"]),
        Err(AtlasError::Protocol { .. })
    ));
}

#[test]
fn version_is_read_from_the_executable() {
    let runner = AtlasEncodeRunner::new(fixture(), std::env::temp_dir());
    assert_eq!(runner.version().expect("version"), "0.4.0-fake");
}
