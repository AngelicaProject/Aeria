//! The project server: one background process per project that keeps the
//! game, the workspace, and the caches of the `aeria` command open, so a
//! command is a request to it rather than a process that opens everything
//! again.
//!
//! The first command for a project starts the server (the same executable
//! with `__serve <root>`); it listens on a loopback port, and
//! `agents/<key>.server` in application data names the port, a random token
//! every request must carry, the server's process, and the build of the
//! executable. A command from another build asks the server to stop and
//! starts a new one. The server stops after [`IDLE_TIMEOUT`] without
//! requests.
//!
//! Reads run in parallel; writes take the project's cross-process write lock
//! and run one at a time. Before each request the server compares the
//! project's stamp with the last one it took in and reloads the workspace
//! when another process, such as the desktop, wrote it.

use std::io::{BufRead, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::project::{Env, Project};
use super::{Request, Response};
use crate::sync::{ProjectFiles, WriteLock};

/// A server with no requests for this long stops.
const IDLE_TIMEOUT: Duration = Duration::from_mins(15);
/// How long a command waits for a new server to open the project.
const START_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a command waits to connect to a running server.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// What `agents/<key>.server` says about the running server.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerInfo {
    pid: u32,
    build: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    /// Why the server could not open the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Envelope {
    token: String,
    build: String,
    request: Request,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum Reply {
    Response(Response),
    /// The server runs another build and stops; start a new one.
    Restart,
    Refused(String),
}

/// The build of this executable: requests from another build restart the
/// server, so a rebuilt or updated `aeria` never talks to an old one.
fn build_id() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            let metadata = std::fs::metadata(&exe).ok()?;
            let modified = metadata
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos();
            Some(format!(
                "{}-{}-{}-{modified}",
                env!("CARGO_PKG_VERSION"),
                exe.display(),
                metadata.len()
            ))
        })
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
}

fn random_token() -> String {
    let mut bytes = [0_u8; 16];
    // A token that cannot be drawn leaves only local processes of this user
    // able to connect, which the loopback port already limits.
    let _ = getrandom::fill(&mut bytes);
    bytes
        .iter()
        .fold(String::with_capacity(32), |mut token, byte| {
            use std::fmt::Write as _;
            let _ = write!(token, "{byte:02x}");
            token
        })
}

fn server_path(files: &ProjectFiles) -> PathBuf {
    files.path_with_extension("server")
}

fn read_info(files: &ProjectFiles) -> Option<ServerInfo> {
    serde_json::from_slice(&std::fs::read(server_path(files)).ok()?).ok()
}

fn write_info(files: &ProjectFiles, info: &ServerInfo) -> std::io::Result<()> {
    let path = server_path(files);
    let partial = path.with_extension("server.partial");
    std::fs::write(
        &partial,
        serde_json::to_vec(info).map_err(std::io::Error::other)?,
    )?;
    crate::sync::replace_file(&partial, &path)
}

/// The running server of a project.
pub(crate) struct Server {
    project: RwLock<Project>,
    files: Option<ProjectFiles>,
    /// The stamp the open workspace has taken in.
    seen: Mutex<Option<String>>,
    last_request: Mutex<Instant>,
}

impl Server {
    /// Reloads the workspace when another process wrote the project since
    /// the server last took its writes in. Call with the write lock held.
    fn take_in_writes(&self, project: &mut Project) -> Result<(), String> {
        let Some(files) = &self.files else {
            return Ok(());
        };
        let stamp = files.stamp();
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        if *seen != stamp {
            project
                .session
                .reload_workspace()
                .map_err(|error| format!("the project changed and cannot be reloaded: {error}"))?;
            *seen = stamp;
        }
        Ok(())
    }

    fn stale(&self) -> bool {
        self.files.as_ref().is_some_and(|files| {
            *self.seen.lock().unwrap_or_else(PoisonError::into_inner) != files.stamp()
        })
    }

    /// Runs a read of the open project.
    pub(crate) fn read<T>(
        &self,
        read: impl FnOnce(&Project) -> Result<T, String>,
    ) -> Result<T, String> {
        if self.stale() {
            let _lock = self.files.as_ref().map(WriteLock::acquire).transpose();
            let mut project = self.project.write().unwrap_or_else(PoisonError::into_inner);
            self.take_in_writes(&mut project)?;
        }
        let project = self.project.read().unwrap_or_else(PoisonError::into_inner);
        read(&project)
    }

    /// Runs a write of the open project under the project's write lock.
    pub(crate) fn write<T>(
        &self,
        write: impl FnOnce(&mut Project) -> Result<T, String>,
    ) -> Result<T, String> {
        let _lock = self
            .files
            .as_ref()
            .map(WriteLock::acquire)
            .transpose()
            .map_err(|error| format!("the project's write lock cannot be taken: {error}"))?;
        let mut project = self.project.write().unwrap_or_else(PoisonError::into_inner);
        self.take_in_writes(&mut project)?;
        let result = write(&mut project);
        // The server's own write is taken in already.
        if let Some(files) = &self.files {
            *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = files.stamp();
        }
        result
    }
}

/// Serves the project at `root` until it idles out. Returns the exit status.
pub(crate) fn serve(root: &Path) -> i32 {
    let env = Env::standalone();
    let Ok(Some(files)) = env.project_files(root) else {
        return 2;
    };
    // A process's working directory cannot be moved or deleted on Windows;
    // the server inherits the agent's, which is often the project. Requests
    // carry their own directory.
    if let Some(directory) = files.path_with_extension("server").parent() {
        let _ = std::env::set_current_dir(directory);
    }
    let build = std::env::var(SERVE_BUILD_VARIABLE).unwrap_or_else(|_| build_id());
    let pid = std::process::id();
    let project = match Project::open(root, &env) {
        Ok(project) => project,
        Err(error) => {
            let _ = write_info(
                &files,
                &ServerInfo {
                    pid,
                    build,
                    port: None,
                    token: None,
                    error: Some(error),
                },
            );
            return 2;
        }
    };
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
        return 2;
    };
    let Ok(port) = listener.local_addr().map(|address| address.port()) else {
        return 2;
    };
    let token = random_token();
    let server = std::sync::Arc::new(Server {
        project: RwLock::new(project),
        seen: Mutex::new(files.stamp()),
        files: Some(files.clone()),
        last_request: Mutex::new(Instant::now()),
    });
    if write_info(
        &files,
        &ServerInfo {
            pid,
            build: build.clone(),
            port: Some(port),
            token: Some(token.clone()),
            error: None,
        },
    )
    .is_err()
    {
        return 2;
    }
    let warm = std::sync::Arc::clone(&server);
    std::thread::spawn(move || {
        warm.project
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .warm();
    });
    let idle = std::sync::Arc::clone(&server);
    let idle_files = files.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(30));
            let quiet = idle
                .last_request
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .elapsed();
            if quiet >= IDLE_TIMEOUT {
                stop(&idle_files, pid);
            }
        }
    });
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        *server
            .last_request
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
        let server = std::sync::Arc::clone(&server);
        let token = token.clone();
        let build = build.clone();
        let files = files.clone();
        std::thread::spawn(move || {
            let restart = handle(stream, &server, &token, &build);
            if restart {
                stop(&files, pid);
            }
        });
    }
    0
}

/// Removes the server file if it still names this process, and exits.
fn stop(files: &ProjectFiles, pid: u32) -> ! {
    if read_info(files).is_some_and(|info| info.pid == pid) {
        let _ = std::fs::remove_file(server_path(files));
    }
    std::process::exit(0);
}

/// Answers one connection. Returns whether the server must restart.
fn handle(stream: TcpStream, server: &Server, token: &str, build: &str) -> bool {
    let mut line = String::new();
    let Ok(clone) = stream.try_clone() else {
        return false;
    };
    if BufReader::new(clone).read_line(&mut line).is_err() {
        return false;
    }
    let (reply, restart) = match serde_json::from_str::<Envelope>(&line) {
        Ok(envelope) if envelope.token != token => {
            (Reply::Refused("wrong token".to_owned()), false)
        }
        Ok(envelope) if envelope.build != build => (Reply::Restart, true),
        Ok(envelope) => (
            Reply::Response(super::execute(
                &envelope.request,
                &super::Access::Served(server),
            )),
            false,
        ),
        Err(error) => (Reply::Refused(error.to_string()), false),
    };
    let mut stream = stream;
    if let Ok(mut bytes) = serde_json::to_vec(&reply) {
        bytes.push(b'\n');
        let _ = stream.write_all(&bytes);
        let _ = stream.flush();
    }
    restart
}

/// Sends a request to the project's server, starting one when none runs.
/// `None` when no server can be used; the command then runs in-process.
pub(crate) fn request(root: &Path, request: &Request) -> Option<Response> {
    let files = Env::standalone().project_files(root).ok()??;
    let build = build_id();
    for _ in 0..3 {
        if let Some(info) = read_info(&files)
            && info.error.is_none()
            && let (Some(port), Some(token)) = (info.port, info.token.clone())
        {
            match send(port, &token, &build, request) {
                Some(Reply::Response(response)) => return Some(response),
                Some(Reply::Restart) => {
                    // The old server stops after answering; give it a moment.
                    std::thread::sleep(Duration::from_millis(100));
                }
                Some(Reply::Refused(_)) | None => {}
            }
        }
        match start(&files, root, &build) {
            Ok(()) => {}
            Err(StartError::Failed(message)) => {
                return Some(Response {
                    stdout: String::new(),
                    stderr: format!("aeria: {message}\n"),
                    code: super::FAILED,
                });
            }
            Err(StartError::Unavailable) => return None,
        }
    }
    None
}

fn send(port: u16, token: &str, build: &str, request: &Request) -> Option<Reply> {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).ok()?;
    let mut bytes = serde_json::to_vec(&Envelope {
        token: token.to_owned(),
        build: build.to_owned(),
        request: request.clone(),
    })
    .ok()?;
    bytes.push(b'\n');
    stream.write_all(&bytes).ok()?;
    stream.flush().ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

enum StartError {
    /// The server reported why it cannot open the project.
    Failed(String),
    /// No server could be started; run in-process.
    Unavailable,
}

/// Starts a server for the project unless another command just did, and
/// waits until it listens.
fn start(files: &ProjectFiles, root: &Path, build: &str) -> Result<(), StartError> {
    // One command at a time starts a server; the others wait and use it.
    let _spawning = crate::sync::FileLock::acquire(&files.path_with_extension("spawn"))
        .map_err(|_| StartError::Unavailable)?;
    if let Some(info) = read_info(files)
        && info.build == build
        && info.error.is_none()
        && let (Some(port), Some(token)) = (info.port, info.token.as_deref())
        && send_ping(port, token, build)
    {
        return Ok(());
    }
    let _ = std::fs::remove_file(server_path(files));
    let exe = server_executable(files, build).map_err(|_| StartError::Unavailable)?;
    let child = spawn_detached(&exe, root, build).map_err(|_| StartError::Unavailable)?;
    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT {
        if let Some(info) = read_info(files)
            && info.pid == child
        {
            return match info.error {
                Some(error) => Err(StartError::Failed(error)),
                None => Ok(()),
            };
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    Err(StartError::Unavailable)
}

/// Whether a server answers at all.
fn send_ping(port: u16, token: &str, build: &str) -> bool {
    matches!(
        send(
            port,
            token,
            build,
            &Request {
                cwd: PathBuf::new(),
                args: vec!["--version".to_owned()],
                stdin: None,
            },
        ),
        Some(Reply::Response(_))
    )
}

/// A copy of this executable for the server to run from: a running
/// executable cannot be replaced on Windows, and the server must never keep
/// a rebuild or an update of `aeria` from replacing it. Copies of other
/// builds that no server runs any more are removed.
fn server_executable(files: &ProjectFiles, build: &str) -> std::io::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let exe = std::env::current_exe()?;
    let digest = Sha256::digest(build.as_bytes());
    let name = digest[..8]
        .iter()
        .fold(String::from("aeria-server-"), |mut name, byte| {
            use std::fmt::Write as _;
            let _ = write!(name, "{byte:02x}");
            name
        });
    let directory = files
        .path_with_extension("server")
        .parent()
        .map(Path::to_owned)
        .unwrap_or_default();
    let copy = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if !copy.exists() {
        if let Ok(entries) = std::fs::read_dir(&directory) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("aeria-server-")
                {
                    // A copy a server still runs from cannot be removed.
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let partial = copy.with_extension("partial");
        std::fs::copy(&exe, &partial)?;
        crate::sync::replace_file(&partial, &copy)?;
    }
    Ok(copy)
}

/// The variable that names the project root to a server, which the server
/// reads when started with `__serve` and no root.
pub(crate) const SERVE_ROOT_VARIABLE: &str = "AERIA_SERVE_ROOT";
/// The variable that names the build of the command that started a server,
/// which runs from a copy of it.
const SERVE_BUILD_VARIABLE: &str = "AERIA_SERVE_BUILD";

/// Starts the server so that it outlives the command, and returns its
/// process ID.
///
/// A process started directly would inherit the command's standard output
/// and error, which the agent's harness reads until they close: the harness
/// would wait for the server to exit. `Start-Process` creates the server
/// through the shell, which passes no handles on; the root travels in an
/// environment variable, so no path needs quoting.
#[cfg(windows)]
fn spawn_detached(exe: &Path, root: &Path, build: &str) -> std::io::Result<u32> {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = exe.to_string_lossy().replace('\'', "''");
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "(Start-Process -FilePath '{exe}' -ArgumentList '__serve' -WindowStyle Hidden -PassThru).Id"
            ),
        ])
        .env(SERVE_ROOT_VARIABLE, root)
        .env(SERVE_BUILD_VARIABLE, build)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|_| std::io::Error::other("the server did not start"))
}

#[cfg(not(windows))]
fn spawn_detached(exe: &Path, root: &Path, build: &str) -> std::io::Result<u32> {
    std::process::Command::new(exe)
        .arg("__serve")
        .env(SERVE_ROOT_VARIABLE, root)
        .env(SERVE_BUILD_VARIABLE, build)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|child| child.id())
}
