//! Fixed generated SDK -> loopback test router -> real worker/command -> codec.
//! The router is test scaffolding, restricted to one predetermined command.
//! It is NOT a guest server: no auth/init, public registry, defaults or VM.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]

#[path = "sdk_protocol/owned.rs"]
mod owned;

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use sandproof_guest_runtime::protocol::{
    Event, ProcessConfig, ProcessInfo, Request, Response as CommandResponse, decode_request,
    encode_event, encode_response, encode_stream_end,
};
use sandproof_guest_runtime::{
    CommandEvent, CommandProcess, LaunchObservation, LaunchSpec, ProcessLimits, ProcessSignal,
    PtySize,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::oneshot;
use tokio::time::timeout;

const WORKER: &str = env!("CARGO_BIN_EXE_sandproof-guest-worker");
const LIMIT: usize = 16384;

struct Fixture {
    directory: tempfile::TempDir,
    spec: LaunchSpec,
    signal: bool,
    truncate: bool,
    started: AtomicBool,
    completed: Arc<AtomicBool>,
    owned_body: bool,
    body_requests: Arc<tokio::sync::Semaphore>,
}

struct Owned(CommandProcess);

impl Drop for Owned {
    fn drop(&mut self) {
        // Test-only failure cleanup, using only the originally retained pidfd.
        // This is not the production HTTP disconnect policy.
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}

struct Stream {
    process: Owned,
    stage: Stage,
    signal: bool,
    truncate: bool,
    completed: Arc<AtomicBool>,
}

enum Stage {
    Start,
    Output,
    Finish,
}

// Preselected wire metadata only: this is not a live process registry. The
// Python probe checks absent/default/present fields through both fixed clients.
async fn list(headers: HeaderMap, body: Bytes) -> Response {
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(decode_request("List", &body, LIMIT).unwrap(), Request::List);
    let empty = ProcessConfig::default();
    let config = ProcessConfig {
        cmd: "/bin/cat".into(),
        args: vec![String::new(), "\"\\\n中💡".into()],
        envs: BTreeMap::from([
            (String::new(), String::new()),
            ("KEY".into(), "value".into()),
        ]),
        cwd: Some(String::new()),
    };
    let entries = match headers["x-fixture-list"].to_str().unwrap() {
        "empty" => vec![],
        "absent" => vec![ProcessInfo {
            config: None,
            pid: 0,
            tag: None,
        }],
        "present" => vec![
            ProcessInfo {
                config: Some(&empty),
                pid: 123,
                tag: None,
            },
            ProcessInfo {
                config: Some(&config),
                pid: u32::MAX,
                tag: Some(""),
            },
        ],
        _ => panic!("unknown controlled list"),
    };
    (
        [("content-type", "application/json")],
        encode_response(CommandResponse::List(&entries), LIMIT).unwrap(),
    )
        .into_response()
}

async fn start(State(fixture): State<Arc<Fixture>>, headers: HeaderMap, body: Bytes) -> Response {
    assert_eq!(headers["content-type"], "application/connect+json");
    let Request::Start(request) = decode_request("Start", &body, LIMIT).unwrap() else {
        panic!("expected Start")
    };
    assert_eq!(request.stdin_enabled(), fixture.spec.stdin);
    assert_eq!(
        request.pty.as_ref().map(
            |pty| pty
                .size
                .as_ref()
                .map_or_else(PtySize::default, |size| PtySize::from_wire(
                    size.cols, size.rows
                ))
        ),
        fixture.spec.pty
    );
    assert!(request.tag.is_none());
    let config = request.process.unwrap();
    // The fixture accepts exactly its predetermined command, once. The codec
    // decodes requests but cannot itself authorize or execute arbitrary input.
    assert_eq!(config.cmd, fixture.spec.program);
    assert_eq!(config.args, fixture.spec.args);
    assert_eq!(config.envs, fixture.spec.env);
    assert_eq!(config.cwd.as_deref(), fixture.spec.cwd.to_str());
    if fixture.started.swap(true, Ordering::SeqCst) {
        return StatusCode::CONFLICT.into_response();
    }
    let process = CommandProcess::prepare(
        Path::new(WORKER),
        fixture.directory.path(),
        1,
        1,
        &fixture.spec,
        ProcessLimits {
            chunk_bytes: 4096,
            launch_bytes: LIMIT,
            handshake_timeout: Duration::from_secs(5),
        },
    )
    .await
    .unwrap();
    if fixture.owned_body {
        return owned::response(process, &fixture).await;
    }
    let mut process = Owned(process);
    assert_eq!(
        process.0.release().await.unwrap(),
        LaunchObservation::NoExecErrorObserved
    );
    let state = Stream {
        process,
        stage: Stage::Start,
        signal: fixture.signal,
        truncate: fixture.truncate,
        completed: Arc::clone(&fixture.completed),
    };
    let stream = futures_util::stream::unfold(Some(state), next_frame);
    (
        [("content-type", "application/connect+json")],
        Body::from_stream(stream),
    )
        .into_response()
}

async fn next_frame(state: Option<Stream>) -> Option<(io::Result<Vec<u8>>, Option<Stream>)> {
    let mut state = state?;
    let frame = if matches!(state.stage, Stage::Start) {
        state.stage = Stage::Output;
        encode_event(Event::Start(state.process.0.identity().pid), LIMIT).unwrap()
    } else if matches!(state.stage, Stage::Finish) {
        if state.truncate {
            return None;
        }
        return Some((
            Ok::<_, io::Error>(encode_stream_end(None, LIMIT).unwrap()),
            None,
        ));
    } else {
        match state
            .process
            .0
            .next_event()
            .await
            .unwrap()
            .expect("missing End")
        {
            CommandEvent::Output { channel, bytes } => {
                let frame = encode_event(Event::Output(channel, &bytes), LIMIT).unwrap();
                state
                    .process
                    .0
                    .consume_output(state.process.0.identity(), channel, bytes.len())
                    .unwrap();
                if state.signal {
                    state
                        .process
                        .0
                        .signal(state.process.0.identity(), ProcessSignal::Kill)
                        .unwrap();
                    state.signal = false;
                }
                frame
            }
            CommandEvent::End {
                code,
                signal,
                core_dumped,
            } => {
                let mut frame = sandproof_guest_service::command::CommandExit {
                    code,
                    signal,
                    core_dumped,
                }
                .encode_end(LIMIT)
                .unwrap();
                assert!(state.process.0.next_event().await.unwrap().is_none());
                state.completed.store(true, Ordering::SeqCst);
                state.stage = Stage::Finish;
                if state.truncate {
                    frame.pop();
                }
                frame
            }
        }
    };
    Some((Ok(frame), Some(state)))
}

async fn probe(asynchronous: bool, mode: &str, owned_body: bool) {
    let script = match mode {
        "binary" | "truncated" => r"printf '\000\377A'; printf 'err' >&2; exit 17",
        "large" => "dd if=/dev/zero bs=8192 count=257 2>/dev/null",
        "signal" => "printf ready; exec /bin/cat",
        "pty" => r"/usr/bin/stty raw -echo; printf '\000\377A'; printf 'err' >&2; exit 17",
        _ => panic!("unknown fixture"),
    };
    let completed = Arc::new(AtomicBool::new(false));
    let fixture = Arc::new(Fixture {
        directory: tempfile::tempdir().unwrap(),
        spec: LaunchSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: "/tmp".into(),
            env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            stdin: mode == "signal",
            pty: (mode == "pty").then_some(PtySize { cols: 91, rows: 37 }),
            credentials: None,
        },
        signal: mode == "signal",
        truncate: mode == "truncated",
        started: AtomicBool::new(false),
        completed: Arc::clone(&completed),
        owned_body,
        body_requests: Arc::new(tokio::sync::Semaphore::new(1)),
    });
    let app = Router::new()
        .route("/process.Process/Start", post(start))
        .route("/process.Process/List", post(list))
        .layer(DefaultBodyLimit::max(LIMIT + 5))
        .with_state(Arc::clone(&fixture));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut sdk = Command::new("uv")
        .current_dir(root)
        .args([
            "run",
            "--locked",
            "--project",
            "tests/compat",
            "python",
            "tests/compat/guest_protocol_sdk.py",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut settings = serde_json::json!({"url": format!("http://{address}"), "asynchronous": asynchronous, "mode": mode, "request": {"process": {"cmd": fixture.spec.program, "args": fixture.spec.args, "cwd": fixture.spec.cwd, "envs": fixture.spec.env}, "stdin": fixture.spec.stdin}});
    if let Some(size) = fixture.spec.pty {
        settings["request"]["pty"] = serde_json::json!({"size":size});
    }
    sdk.stdin
        .take()
        .unwrap()
        .write_all(settings.to_string().as_bytes())
        .await
        .unwrap();
    let status = timeout(Duration::from_mins(1), sdk.wait()).await;
    let _ = stop.send(());
    timeout(Duration::from_secs(5), server)
        .await
        .expect("fixture HTTP connections leaked")
        .unwrap()
        .unwrap();
    assert!(
        status.expect("SDK probe timed out").unwrap().success(),
        "{mode}, async={asynchronous}"
    );
    assert!(fixture.started.load(Ordering::SeqCst));
    if owned_body {
        owned::settled(&fixture).await;
    }
    assert!(
        completed.load(Ordering::SeqCst),
        "real command not reaped/drained"
    );
    assert_eq!(
        std::fs::read_dir(fixture.directory.path()).unwrap().count(),
        0,
        "private worker socket leaked"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pinned_sdk_streams_real_command_events() {
    for asynchronous in [false, true] {
        for mode in ["binary", "large", "signal", "truncated", "pty"] {
            probe(asynchronous, mode, false).await;
        }
        for mode in ["binary", "pty"] {
            probe(asynchronous, mode, true).await;
        }
    }
}
