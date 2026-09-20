use std::env;
use std::fs;
use std::thread;
use std::time::Duration;

use serde_json::json;

fn main() {
    let args = env::args().collect::<Vec<_>>();
    let game_path = args
        .windows(2)
        .find(|pair| pair[0] == "--game-path")
        .map_or_else(|| "success".to_owned(), |pair| pair[1].clone());
    let scenario = std::path::Path::new(&game_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("success")
        .to_owned();
    let output = args
        .windows(2)
        .find(|pair| pair[0] == "--output")
        .map_or_else(|| "source.hsp".to_owned(), |pair| pair[1].clone());
    let package_id = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    if scenario == "large-stderr" {
        eprintln!("{}", "x".repeat(128 * 1024));
    }
    emit(&json!({"type": "started", "protocolVersion": 1, "language": "en"}));

    match scenario.as_str() {
        "no-completed" | "large-stderr" => {}
        "completed-nonzero" => {
            emit(
                &json!({"type": "completed", "protocolVersion": 1, "packageId": package_id, "outputPath": output}),
            );
            let _ = fs::write(&output, b"fake");
            std::process::exit(3);
        }
        "failed" => {
            emit(
                &json!({"type": "failed", "protocolVersion": 1, "code": "fakeFailure", "message": "fixture failure"}),
            );
            std::process::exit(1);
        }
        "malformed" => println!("not json"),
        "unknown" => emit(&json!({"type": "unknown", "protocolVersion": 1})),
        "wrong-version" => {
            emit(&json!({"type": "phase", "protocolVersion": 2, "phase": "inspectInstallation"}));
        }
        "duplicate-started" => emit(&json!({"type": "started", "protocolVersion": 1})),
        "after-terminal" => {
            emit(
                &json!({"type": "completed", "protocolVersion": 1, "packageId": package_id, "outputPath": output}),
            );
            emit(&json!({"type": "phase", "protocolVersion": 1, "phase": "late"}));
            let _ = fs::write(&output, b"fake");
        }
        "cancel" => loop {
            thread::sleep(Duration::from_millis(50));
        },
        _ => {
            emit(&json!({"type": "phase", "protocolVersion": 1, "phase": "inspectInstallation"}));
            emit(
                &json!({"type": "progress", "protocolVersion": 1, "phase": "extractSource", "sheet": "Item", "rowsProcessed": 1}),
            );
            let _ = fs::write(&output, b"fake");
            emit(
                &json!({"type": "completed", "protocolVersion": 1, "packageId": package_id, "outputPath": output, "elapsedMs": 1}),
            );
        }
    }
}

fn emit(value: &serde_json::Value) {
    println!("{value}");
}
