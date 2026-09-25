use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use aeria_publish::{GitHubClient, GitHubRepository, PublishError, ReleaseAsset, ReleaseRequest};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct Recorded {
    method: String,
    target: String,
    authorization: String,
    body: Vec<u8>,
}

type Responder = dyn Fn(&str, &str, &str) -> (u16, String) + Send + Sync;

/// A one-request-per-connection HTTP/1.1 server. `respond` receives the
/// server's base URL, the method, and the request target.
fn serve(respond: Box<Responder>) -> (String, Arc<Mutex<Vec<Recorded>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let log = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&log);
    let server_base = base.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("request line");
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_owned();
            let target = parts.next().unwrap_or_default().to_owned();
            let mut length = 0;
            let mut authorization = String::new();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).expect("header");
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (name, value) = header.split_once(':').expect("header");
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => length = value.trim().parse().expect("length"),
                    "authorization" => value.trim().clone_into(&mut authorization),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).expect("body");
            let (status, response) = respond(&server_base, &method, &target);
            recorded.lock().expect("log").push(Recorded {
                method,
                target,
                authorization,
                body,
            });
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            );
        }
    });
    (base, log)
}

fn calls(log: &Mutex<Vec<Recorded>>) -> Vec<String> {
    log.lock()
        .expect("log")
        .iter()
        .map(|call| format!("{} {}", call.method, call.target))
        .collect()
}

fn draft(base: &str, id: u32) -> String {
    json!({
        "id": id,
        "tag_name": "harmonia/7",
        "draft": true,
        "upload_url": format!("{base}/uploads/repos/owner/ru/releases/{id}/assets{{?name,label}}"),
    })
    .to_string()
}

fn repository() -> GitHubRepository {
    GitHubRepository {
        owner: "owner".to_owned(),
        name: "ru".to_owned(),
    }
}

fn request() -> ReleaseRequest {
    ReleaseRequest {
        sequence: 7,
        commit: "a".repeat(40),
        name: "Russian 2026.09.25".to_owned(),
        body: "Notes".to_owned(),
        prerelease: false,
        assets: vec![
            ReleaseAsset {
                name: "ru-main-7.hpk.br".to_owned(),
                content_type: "application/octet-stream",
                bytes: vec![1, 2, 3],
            },
            ReleaseAsset {
                name: "feed-entry.json".to_owned(),
                content_type: "application/json",
                bytes: b"{}".to_vec(),
            },
        ],
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
fn release_is_drafted_filled_and_then_published() {
    let (base, log) = serve(Box::new(|base, method, target| match (method, target) {
        ("POST", "/repos/owner/ru/releases") => (201, draft(base, 5)),
        ("POST", _) => (201, json!({ "id": 1, "tag_name": "x" }).to_string()),
        ("PATCH", _) => (
            200,
            json!({
                "id": 5,
                "tag_name": "harmonia/7",
                "html_url": "https://github.com/owner/ru/releases/tag/harmonia/7",
            })
            .to_string(),
        ),
        _ => (500, String::new()),
    }));
    let client = GitHubClient::with_base_urls(&base, &format!("{base}/uploads/")).expect("client");
    let published = runtime()
        .block_on(client.publish_release(&repository(), "token-1", &request()))
        .expect("published");
    assert_eq!(
        published.html_url,
        "https://github.com/owner/ru/releases/tag/harmonia/7"
    );
    assert_eq!(
        calls(&log),
        [
            "POST /repos/owner/ru/releases",
            "POST /uploads/repos/owner/ru/releases/5/assets?name=ru-main-7.hpk.br",
            "POST /uploads/repos/owner/ru/releases/5/assets?name=feed-entry.json",
            "PATCH /repos/owner/ru/releases/5",
        ]
    );
    let log = log.lock().expect("log").clone();
    assert!(
        log.iter()
            .all(|call| call.authorization == "Bearer token-1")
    );
    let created: Value = serde_json::from_slice(&log[0].body).expect("json");
    assert_eq!(created["tag_name"], "harmonia/7");
    assert_eq!(created["target_commitish"], "a".repeat(40));
    assert_eq!(created["draft"], true);
    assert_eq!(log[1].body, [1, 2, 3]);
    let published: Value = serde_json::from_slice(&log[3].body).expect("json");
    assert_eq!(published["draft"], false);
}

#[test]
fn a_failed_upload_deletes_the_draft() {
    let (base, log) = serve(Box::new(|base, method, target| match (method, target) {
        ("POST", "/repos/owner/ru/releases") => (201, draft(base, 9)),
        ("POST", _) => (
            422,
            json!({ "message": "Validation Failed", "errors": [{ "code": "already_exists" }] })
                .to_string(),
        ),
        ("DELETE", _) => (204, String::new()),
        _ => (500, String::new()),
    }));
    let client = GitHubClient::with_base_urls(&base, &format!("{base}/uploads/")).expect("client");
    let error = runtime()
        .block_on(client.publish_release(&repository(), "token-1", &request()))
        .expect_err("upload fails");
    assert!(
        matches!(error, PublishError::Rejected { ref message } if message.contains("already_exists"))
    );
    assert_eq!(
        calls(&log).last().map(String::as_str),
        Some("DELETE /repos/owner/ru/releases/9")
    );
}

#[test]
fn upload_urls_off_github_are_refused_before_sending_the_token() {
    let (base, log) = serve(Box::new(|_, method, target| match (method, target) {
        ("POST", "/repos/owner/ru/releases") => (
            201,
            json!({
                "id": 3,
                "tag_name": "harmonia/7",
                "upload_url": "https://evil.example/assets{?name}",
            })
            .to_string(),
        ),
        _ => (204, String::new()),
    }));
    let client =
        GitHubClient::with_base_urls(&base, "https://uploads.github.com/").expect("client");
    let error = runtime()
        .block_on(client.publish_release(&repository(), "token-1", &request()))
        .expect_err("refused");
    assert!(matches!(error, PublishError::InvalidResponse { .. }));
    assert_eq!(
        calls(&log),
        [
            "POST /repos/owner/ru/releases",
            "DELETE /repos/owner/ru/releases/3"
        ]
    );
}

#[test]
fn pack_releases_are_read_from_every_page() {
    let (base, _log) = serve(Box::new(|_, _, target| {
        let releases: Vec<Value> = if target.ends_with("page=1") {
            (0..99)
                .map(|index| json!({ "id": index, "tag_name": format!("v{index}") }))
                .chain([json!({ "id": 100, "tag_name": "harmonia/3" })])
                .collect()
        } else {
            vec![
                json!({ "id": 101, "tag_name": "harmonia/12", "draft": true }),
                json!({ "id": 102, "tag_name": "harmonia/x" }),
            ]
        };
        (200, Value::from(releases).to_string())
    }));
    let client = GitHubClient::with_base_urls(&base, &base).expect("client");
    let releases = runtime()
        .block_on(client.pack_releases(&repository(), "token-1"))
        .expect("releases");
    let found: Vec<(u64, bool)> = releases
        .iter()
        .map(|release| (release.sequence, release.draft))
        .collect();
    assert_eq!(found, [(12, true), (3, false)]);
}
