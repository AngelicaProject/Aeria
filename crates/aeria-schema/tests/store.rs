use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use aeria_core::GameVersion;
use aeria_schema::{SchemaStore, SheetFields};

const OLD: &str = "1111111111111111111111111111111111111111";
const NEW: &str = "2222222222222222222222222222222222222222";

/// A one-request-per-connection HTTP/1.1 server answering the EXDSchema
/// refs and two schema files, recording every request target. While
/// `online` is false, it closes every connection unanswered.
fn serve(online: Arc<Mutex<bool>>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let log = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&log);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("request line");
            let target = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).expect("header");
                if header.trim_end().is_empty() {
                    break;
                }
            }
            if !*online.lock().expect("online") {
                continue;
            }
            recorded.lock().expect("log").push(target.clone());
            let refs = format!(
                r#"[{{"ref":"refs/heads/ver/2026.04.21.0000.0000","object":{{"sha":"{OLD}"}}}},{{"ref":"refs/heads/ver/2026.09.01.0000.0000","object":{{"sha":"{NEW}"}}}},{{"ref":"refs/heads/ver/2027.01.01.0000.0000","object":{{"sha":"{OLD}"}}}}]"#
            );
            let (status, body) = match target.as_str() {
                "/repos/xivdev/EXDSchema/git/matching-refs/heads/ver/" => (200, refs),
                path if path == format!("/xivdev/EXDSchema/{NEW}/PlaceName.yml") => (
                    200,
                    "name: PlaceName\nfields:\n  - name: Name\n  - name: Order\n".to_owned(),
                ),
                path if path == format!("/xivdev/EXDSchema/{NEW}/Broken.yml") => {
                    (200, "name: Broken\n".to_owned())
                }
                _ => (404, "404: Not Found".to_owned()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (base, log)
}

fn game() -> GameVersion {
    "2026.10.06.0000.0000".parse().expect("version")
}

fn place_name() -> SheetFields {
    SheetFields {
        version: "2026.09.01.0000.0000".to_owned(),
        fields: vec!["Name".to_owned(), "Order".to_owned()],
    }
}

#[tokio::test]
async fn schemas_come_from_the_newest_branch_not_newer_than_the_game_and_are_cached() {
    let cache = tempfile::tempdir().expect("cache");
    let online = Arc::new(Mutex::new(true));
    let (base, log) = serve(Arc::clone(&online));
    let store = SchemaStore::with_base_urls(cache.path().to_owned(), &base, &base).expect("store");

    assert_eq!(
        store
            .sheet_fields(&game(), "PlaceName")
            .await
            .expect("fields"),
        Some(place_name())
    );
    assert_eq!(
        store
            .sheet_fields(&game(), "Missing")
            .await
            .expect("fields"),
        None,
        "a sheet without a file names nothing"
    );
    assert_eq!(
        store
            .sheet_fields(&game(), "quest/000/ClsArc001_00001")
            .await
            .expect("fields"),
        None,
        "quest sheets are not asked for"
    );
    assert!(store.sheet_fields(&game(), "Broken").await.is_err());
    // Asked again, everything comes from memory and the cache.
    store
        .sheet_fields(&game(), "PlaceName")
        .await
        .expect("fields");
    store
        .sheet_fields(&game(), "Missing")
        .await
        .expect("fields");
    assert_eq!(
        *log.lock().expect("log"),
        [
            "/repos/xivdev/EXDSchema/git/matching-refs/heads/ver/".to_owned(),
            format!("/xivdev/EXDSchema/{NEW}/PlaceName.yml"),
            format!("/xivdev/EXDSchema/{NEW}/Missing.yml"),
            format!("/xivdev/EXDSchema/{NEW}/Broken.yml"),
        ]
    );

    // A later run without network uses the resolution and files it kept.
    *online.lock().expect("online") = false;
    let offline =
        SchemaStore::with_base_urls(cache.path().to_owned(), &base, &base).expect("store");
    assert_eq!(
        offline
            .sheet_fields(&game(), "PlaceName")
            .await
            .expect("fields"),
        Some(place_name())
    );
    assert_eq!(
        offline
            .sheet_fields(&game(), "Missing")
            .await
            .expect("fields"),
        None
    );
}

#[tokio::test]
async fn a_game_older_than_every_branch_has_no_schema() {
    let cache = tempfile::tempdir().expect("cache");
    let (base, log) = serve(Arc::new(Mutex::new(true)));
    let store = SchemaStore::with_base_urls(cache.path().to_owned(), &base, &base).expect("store");
    let old: GameVersion = "2020.01.01.0000.0000".parse().expect("version");
    assert_eq!(
        store.sheet_fields(&old, "PlaceName").await.expect("fields"),
        None
    );
    assert_eq!(log.lock().expect("log").len(), 1, "no file is asked for");
}

#[tokio::test]
async fn without_network_and_cache_the_failure_is_reported() {
    let cache = tempfile::tempdir().expect("cache");
    let (base, _) = serve(Arc::new(Mutex::new(false)));
    let store = SchemaStore::with_base_urls(cache.path().to_owned(), &base, &base).expect("store");
    assert!(store.sheet_fields(&game(), "PlaceName").await.is_err());
}
