//! A run over a synthetic game and project, against a local server that
//! answers like the Codex backend.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use aeria_model::auth::{AccessToken, Secret};
use aeria_model::{Codex, Options, Run, Stop, TokenStore};
use aeria_source::{GameSource, SourceLanguage};
use aeria_sqpack::testing::{FakeGame, TextSheet};
use base64::Engine as _;
use serde_json::{Value, json};

struct NoStore;

impl TokenStore for NoStore {
    fn get(&self) -> Result<Option<Secret>, String> {
        Ok(None)
    }
    fn set(&self, _: &Secret) -> Result<(), String> {
        Ok(())
    }
    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

fn access() -> AccessToken {
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({ "exp": 4_000_000_000_u64 }).to_string());
    AccessToken::parse(&format!("eyJhbGciOiJub25lIn0.{claims}.signature")).expect("token")
}

/// Serves `/responses`: each request's strings translated by `answer`, and
/// every request's input kept.
fn serve(answer: fn(&str, &str) -> String) -> (String, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("header");
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().expect("length");
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("body");
            let body: Value = serde_json::from_slice(&body).expect("json");
            let input = body["input"][0]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let task: Value = serde_json::from_str(input.split("\n\n").next().unwrap_or_default())
                .expect("task json");
            seen.lock().expect("lock").push(task.clone());
            let mut answers = serde_json::Map::new();
            for string in task["strings"].as_array().into_iter().flatten() {
                let id = string["id"].as_str().expect("id");
                let source = string["source"].as_str().expect("source");
                answers.insert(id.to_owned(), Value::from(answer(id, source)));
            }
            let text = Value::from(Value::Object(answers).to_string());
            let events = format!(
                "data: {}\n\ndata: {}\n\n",
                json!({ "type": "response.output_text.delta", "delta": text }),
                json!({ "type": "response.completed", "response": { "status": "completed", "usage": { "input_tokens": 1000, "input_tokens_details": { "cached_tokens": 900 }, "output_tokens": 50 } } }),
            );
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{events}",
                events.len()
            );
        }
    });
    (base, requests)
}

fn translate(_: &str, source: &str) -> String {
    match source {
        "Hello, Minfilia." => "Здравствуй, Минфилия.".to_owned(),
        // Both genders at once: refused by the checks, and again on retry.
        "You are ready." => "Ты готов(а).".to_owned(),
        other => format!("«{other}»"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_translates_what_is_left_with_names_and_checks() {
    let directory = tempfile::tempdir().expect("directory");
    let game = directory.path().join("game");
    FakeGame::new("2026.09.15.0000.0000")
        .with_text(
            "ENpcResident",
            &TextSheet::new(1, &[0]).row(1, &[(0, "Minfilia")]),
        )
        .with_text(
            "Addon",
            &TextSheet::new(1, &[0])
                .row(1, &[(0, "Hello, Minfilia.")])
                .row(2, &[(0, "You are ready.")])
                .row(3, &[(0, "Cancel")]),
        )
        .write(&game)
        .expect("game");
    let source = Arc::new(GameSource::open(&game, SourceLanguage::English).expect("source"));
    let root = directory.path().join("project");
    let session = Arc::new(aeria_po::session::create(&root, source, "ru", 1).expect("project"));
    session
        .set_translation("ENpcResident", 1, 0, 0, "Минфилия")
        .expect("name");
    session
        .set_translation("Addon", 3, 0, 0, "Отмена")
        .expect("done");

    let (base, requests) = serve(translate);
    let codex = Arc::new(Codex::with_access(Arc::new(NoStore), &base, access()).expect("codex"));
    let handle = Arc::new(Run::new());
    aeria_model::run::run(
        Arc::clone(&session),
        codex,
        Options {
            paths: vec!["Addon.po".to_owned()],
            fuzzy: false,
            model: "test".to_owned(),
            effort: None,
        },
        Arc::clone(&handle),
    )
    .await;

    let status = handle.status();
    assert_eq!(status.stop, Some(Stop::Finished), "{status:?}");
    assert_eq!((status.strings, status.written, status.rejected), (2, 1, 1));
    assert_eq!(status.rejections[0].context, "Addon:2:0:0");
    assert_eq!(status.cached_tokens, 1800);
    let translated = session
        .translation("Addon", 1, 0, 0)
        .expect("read")
        .expect("written");
    assert_eq!(translated.text, "Здравствуй, Минфилия.");
    assert!(
        session
            .translation("Addon", 2, 0, 0)
            .expect("read")
            .is_none()
    );

    let requests = requests.lock().expect("lock");
    assert_eq!(requests[0]["names"][0]["translation"], "Минфилия");
    assert_eq!(requests[0]["examples"][0]["translation"], "Отмена");
    assert_eq!(requests[0]["strings"].as_array().expect("strings").len(), 2);

    // What was written is not sent again; what was refused is.
    let left =
        aeria_model::run::plan(session.root(), &["Addon.po".to_owned()], false).expect("plan");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].contexts, ["Addon:2:0:0"]);
}
