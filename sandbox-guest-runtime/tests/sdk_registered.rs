//! Fixed SDK -> restricted loopback fixture -> shared command HTTP dispatch.
//! One preselected command per fixture; not a production HTTP router or Go oracle.
#![cfg(all(target_os = "linux", feature = "sdk-contract-tests"))]

use std::{
    fs::File,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{OriginalUri, State},
    http::{HeaderMap, Method, Request as HttpRequest},
    response::Response,
    routing::post,
};
use sandproof_guest_runtime::{
    Credentials, ProcessLimits, ProcessSignal,
    accounts::{Account, AccountError},
    pid_claims::{Binding, MAX_PID, PidClaims},
    protocol::{Request, decode_request},
};
use sandproof_guest_service::{
    AccountPool, AccountSource, ContextLimits, ExecutionSettings, InitLimits, Metadata,
    MetadataSource, ServiceError, Session,
    command::{
        AdmissionLimits, CommandBodyLimits, CommandOwners, ManagedCommand, OwnerLimits,
        PidRegistrar, PidRegistry, RegistryError, RegistryLimits,
    },
    command_http::{
        CommandHttp, Prepared, Processes, Ready as CommandReady, RequestPools, ResponseLimits,
    },
    command_requests::{self, CommandRequests},
    file_upload::HeaderLimits,
    upload_body::HttpBodyLimits,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use tokio::{
    io::AsyncWriteExt,
    net::TcpListener,
    process::Command,
    sync::oneshot,
    time::{Instant, timeout},
};

const WORKER: &str = env!("CARGO_BIN_EXE_sandproof-guest-worker");
const LIMIT: usize = 16384;
const REQUESTS: usize = 4;
const INVALID_KEEPALIVE: [&str; 6] = [
    "0",
    "-1",
    "9223372037",
    "-9223372037",
    "20211507185753197",
    "9223372036854775807",
];

struct MetadataFixture;
impl MetadataSource for MetadataFixture {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "template".into(),
            access_token_sha512: Sha512::digest(b"token").into(),
        })
    }
}
struct CurrentAccount {
    home: String,
    lookups: Arc<AtomicUsize>,
}
impl AccountSource for CurrentAccount {
    fn lookup(&self, name: &str) -> Result<Account, AccountError> {
        assert_eq!(name, "fixture");
        self.lookups.fetch_add(1, Ordering::SeqCst);
        Ok(Account {
            name: name.into(),
            home: self.home.clone(),
            credentials: Credentials {
                uid: rustix::process::geteuid().as_raw(),
                gid: rustix::process::getegid().as_raw(),
                groups: rustix::process::getgroups()
                    .unwrap()
                    .iter()
                    .map(|group| group.as_raw())
                    .collect(),
            },
        })
    }
}
struct Stop(ManagedCommand);
impl Drop for Stop {
    fn drop(&mut self) {
        // Fixture failure cleanup uses only the registry's original pidfd.
        let _ = self.0.signal(self.0.identity(), ProcessSignal::Kill);
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    http: CommandHttp<MetadataFixture, CurrentAccount>,
    lookups: Arc<AtomicUsize>,
    calls: AtomicUsize,
    keepalive_rejections: AtomicUsize,
    requests: CommandRequests,
    controls: CommandRequests,
    owners: CommandOwners,
    registry: PidRegistry,
    expected: Request,
    started: AtomicBool,
    duplicate_starts: AtomicUsize,
    command: Mutex<Option<Stop>>,
}
fn binding() -> Binding {
    Binding::new([0x31; 32]).unwrap()
}
fn settings() -> ExecutionSettings {
    ExecutionSettings {
        daemon_path: "/usr/bin:/bin".into(),
        daemon_workdir: "/".into(),
        limits: ContextLimits {
            bytes: 4096,
            env_entries: 32,
            arguments: 32,
            username_bytes: 64,
            authorization_bytes: 256,
        },
    }
}
fn request_pool() -> CommandRequests {
    CommandRequests::new(command_requests::Limits {
        in_flight: REQUESTS,
        timeout: Duration::from_secs(15),
        headers: HeaderLimits {
            bytes: 4096,
            fields: 32,
        },
        body: HttpBodyLimits {
            total_bytes: LIMIT as u64,
            frame_bytes: LIMIT,
            frames: 32,
            trailer_bytes: 64,
            trailer_fields: 2,
        },
    })
    .unwrap()
}
fn response_limits() -> ResponseLimits {
    ResponseLimits {
        message: CommandBodyLimits {
            message_bytes: 4096,
            keepalive: Duration::from_secs(5),
        },
        http: command_requests::http::reply::Limits {
            wire_bytes: 8192,
            chunk_bytes: 2048,
        },
        account_headers: 4096,
    }
}
fn expected(request: &Value) -> Request {
    let bytes = serde_json::to_vec(request).unwrap();
    let mut framed = vec![0];
    framed.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
    framed.extend(bytes);
    decode_request("Start", &framed, LIMIT).unwrap()
}
impl Fixture {
    async fn new(request: &Value) -> Self {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(directory.path().join("gates")).unwrap();
        let claims = PidClaims::create(
            File::open(directory.path()).unwrap(),
            rustix::process::geteuid().as_raw(),
            binding(),
            MAX_PID,
        )
        .unwrap();
        let session = Session::new(
            MetadataFixture,
            InitLimits {
                body_bytes: 4096,
                token_bytes: 128,
                state_bytes: 2048,
                env_entries: 32,
                in_flight: 1,
                timeout: Duration::from_secs(5),
            },
        )
        .unwrap();
        session.initialize(br#"{"accessToken":"token","defaultUser":"fixture","defaultWorkdir":".","envVars":{"VALUE":"default"}}"#).await.unwrap();
        let lookups = Arc::new(AtomicUsize::new(0));
        let accounts = AccountPool::new(
            CurrentAccount {
                home: directory.path().to_str().unwrap().into(),
                lookups: lookups.clone(),
            },
            REQUESTS,
            Duration::from_secs(5),
        )
        .unwrap();
        let requests = request_pool();
        let controls = request_pool();
        let owners = CommandOwners::new(OwnerLimits {
            records: 1,
            subscribers: 3,
            queued_chunks: 16,
            output_bytes: 8192,
            input_chunk_bytes: 8192,
        })
        .unwrap();
        let registry = PidRegistry::new(
            binding(),
            1,
            RegistryLimits {
                records: 1,
                metadata_bytes: 4096,
                retention: Duration::from_secs(30),
            },
        )
        .unwrap();
        let http = CommandHttp::new(
            session,
            RequestPools {
                ordinary: requests.clone(),
                control: controls.clone(),
            },
            accounts,
            settings(),
            Processes {
                owners: owners.clone(),
                registry: registry.clone(),
                registrar: PidRegistrar::new(claims),
                binding: binding(),
                session: 1,
                worker: WORKER.into(),
                private_directory: directory.path().join("gates"),
                process_limits: ProcessLimits {
                    chunk_bytes: 1024,
                    launch_bytes: LIMIT,
                    handshake_timeout: Duration::from_secs(5),
                },
                admission_limits: AdmissionLimits {
                    ready_timeout: Duration::from_secs(5),
                    cleanup_timeout: Duration::from_secs(5),
                    retry_delay: Duration::from_secs(1),
                },
            },
            response_limits(),
        )
        .unwrap();
        Self {
            directory,
            http,
            lookups,
            calls: AtomicUsize::new(0),
            keepalive_rejections: AtomicUsize::new(0),
            requests,
            controls,
            owners,
            registry,
            expected: expected(request),
            started: AtomicBool::new(false),
            duplicate_starts: AtomicUsize::new(0),
            command: Mutex::new(None),
        }
    }
}

async fn dispatch(
    State(fixture): State<Arc<Fixture>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let start = uri.path() == "/process.Process/Start";
    let stream = start || uri.path() == "/process.Process/Connect";
    selected_json(&headers, stream);
    let invalid_keepalive = headers.get("keepalive-ping-interval").map(|value| {
        assert!(stream);
        assert!(
            INVALID_KEEPALIVE
                .iter()
                .any(|expected| value.as_bytes() == expected.as_bytes())
        );
        std::fs::read(fixture.directory.path().join("pid-claims.bin")).unwrap()
    });
    // This restricted Axum transport is not the production listener. Dropping
    // an unread HTTP/1 request after an early header rejection can reset the
    // socket while the original SDK is still sending its body, hiding the
    // intended Connect error behind BrokenPipe. Receive only this fixture's
    // finite request before calling the ORIGINAL preparation code; do not
    // decode it, relax the rejection, replay it or change production limits.
    let body = if invalid_keepalive.is_some() {
        Body::from(
            timeout(Duration::from_secs(5), axum::body::to_bytes(body, LIMIT))
                .await
                .expect("fixture request body deadline")
                .expect("fixture request body limit or transport failure"),
        )
    } else {
        body
    };
    let mut request = HttpRequest::new(body);
    *request.method_mut() = Method::POST;
    *request.uri_mut() = uri;
    *request.headers_mut() = headers;
    let prepared = fixture.http.prepare(request).await.unwrap();
    fixture.calls.fetch_add(1, Ordering::SeqCst);
    if let Some(before) = invalid_keepalive {
        let Prepared::Response(response) = prepared else {
            panic!("invalid keepalive was not rejected before decoding");
        };
        assert!(!fixture.started.load(Ordering::SeqCst));
        assert!(!fixture.directory.path().join("effect").exists());
        assert_eq!(fixture.registry.available_slots(), 1);
        assert!(fixture.owners.reserve().is_ok());
        assert_eq!(
            std::fs::read_dir(fixture.directory.path().join("gates"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            std::fs::read(fixture.directory.path().join("pid-claims.bin")).unwrap(),
            before
        );
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"],
            "application/connect+json"
        );
        fixture.keepalive_rejections.fetch_add(1, Ordering::SeqCst);
        return response;
    }
    let Prepared::Ready(ready) = prepared else {
        panic!("unexpected fixture preflight rejection");
    };
    if start {
        assert_eq!(ready.request(), &fixture.expected);
        if fixture.started.swap(true, Ordering::SeqCst) {
            return duplicate_start(&fixture, *ready).await;
        }
    }
    let response = ready.execute().await.unwrap();
    if start {
        let listed: Value =
            serde_json::from_slice(&fixture.registry.list_json(4096).unwrap()).unwrap();
        let pid = u32::try_from(listed["processes"][0]["pid"].as_u64().unwrap()).unwrap();
        *fixture.command.lock().unwrap() = Some(Stop(fixture.registry.connectable(pid).unwrap()));
    }
    response
}

async fn duplicate_start(
    fixture: &Fixture,
    ready: CommandReady<MetadataFixture, CurrentAccount>,
) -> Response {
    let Request::Start(expected) = &fixture.expected else {
        panic!("fixture must hold a Start request");
    };
    let tag = expected
        .tag
        .as_deref()
        .expect("only tagged fixtures may repeat Start");
    assert_eq!(fixture.duplicate_starts.fetch_add(1, Ordering::SeqCst), 0);
    let command = fixture.registry.live_tag(tag).unwrap().identity();
    let claims = fixture.directory.path().join("pid-claims.bin");
    let before = std::fs::read(&claims).unwrap();
    let gates = || {
        std::fs::read_dir(fixture.directory.path().join("gates"))
            .unwrap()
            .count()
    };
    let gate_count = gates();
    let response = ready.execute().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(std::fs::read(claims).unwrap(), before);
    assert_eq!(gates(), gate_count);
    assert_eq!(fixture.registry.live_tag(tag).unwrap().identity(), command);
    assert!(fixture.owners.reserve().is_err());
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    response
}

fn selected_json(headers: &HeaderMap, stream: bool) {
    // Only the pinned client's selected JSON/identity shape is accepted here.
    // This is deliberately not a general HTTP negotiation or error mapper.
    assert_eq!(
        headers["content-type"],
        if stream {
            "application/connect+json"
        } else {
            "application/json"
        }
    );
    assert_eq!(headers["connect-timeout-ms"], "10000");
    for name in ["content-encoding", "connect-content-encoding"] {
        // The fixed client materializes None as IdentityCompression, and may
        // send the explicit name. Both absent and identity mean raw bytes.
        let value = headers.get(name).map_or(b"".as_slice(), |v| v.as_bytes());
        assert!(
            matches!(value, b"" | b"identity"),
            "unexpected fixture encoding: {name}"
        );
    }
}

async fn settled(fixture: &Fixture, basic: bool, mode: &str) {
    let tagged = matches!(&fixture.expected, Request::Start(request) if request.tag.is_some());
    assert_eq!(
        fixture.duplicate_starts.load(Ordering::SeqCst),
        usize::from(tagged)
    );
    assert_eq!(
        fixture.keepalive_rejections.load(Ordering::SeqCst),
        2 * INVALID_KEEPALIVE.len()
    );
    let command = fixture.command.lock().unwrap().take().unwrap();
    let exit = timeout(Duration::from_secs(5), command.0.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        exit.signal,
        match mode {
            "term" => Some(15),
            "kill" => Some(9),
            _ => None,
        }
    );
    if exit.signal.is_none() {
        assert_eq!(exit.code, 0);
    }
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert_eq!(fixture.registry.list_json(2).unwrap(), b"{}");
    let identity = command.0.identity();
    let finished = command.0.status().completed_at.unwrap();
    // Actual wait/drain and HTTP shutdown precede this accelerated TTL check.
    tokio::time::pause();
    tokio::time::advance(
        (finished + Duration::from_secs(30) + Duration::from_millis(1))
            .saturating_duration_since(Instant::now()),
    )
    .await;
    assert!(matches!(
        fixture.registry.connectable(identity.pid),
        Err(RegistryError::NotFound)
    ));
    if let Request::Start(request) = &fixture.expected
        && let Some(tag) = &request.tag
    {
        assert!(matches!(
            fixture.registry.connectable_tag(tag),
            Err(RegistryError::NotFound)
        ));
    }
    tokio::time::resume();
    drop(command);
    timeout(Duration::from_secs(5), async {
        loop {
            if fixture.registry.available_slots() == 1
                && let Ok(slot) = fixture.owners.reserve()
            {
                drop(slot);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("original command/registry capacity leaked");
    for pool in [&fixture.requests, &fixture.controls] {
        let slots: Vec<_> = (0..REQUESTS).map(|_| pool.admit().unwrap()).collect();
        drop(slots);
    }
    assert_eq!(
        std::fs::read_dir(fixture.directory.path().join("gates"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        fixture.lookups.load(Ordering::SeqCst),
        if basic {
            fixture.calls.load(Ordering::SeqCst)
        } else {
            1 + usize::from(tagged)
        }
    );
}

async fn probe(asynchronous: bool, mode: &str, tag: Option<&str>) {
    let script = match mode {
        "pipe" => "printf once >> effect; printf '%s|ready' \"$VALUE\"; exec /bin/cat",
        "term" | "kill" => {
            "printf once >> effect; mkfifo hold; printf '%s|ready' \"$VALUE\"; read value < hold"
        }
        "pty" => {
            "printf once >> effect; stty -echo; stty size; read value; stty size; printf 'got:%s' \"$value\""
        }
        _ => panic!("unknown fixture"),
    };
    let mut request = json!({"process":{"cmd":"/bin/sh", "args":["-c",script], "envs":{"VALUE":"request"}},"stdin":true});
    if mode == "pty" {
        request["pty"] = json!({"size":{"cols":80,"rows":24}});
    }
    if let Some(tag) = tag {
        request["tag"] = json!(tag);
    }
    let fixture = Arc::new(Fixture::new(&request).await);
    let mut app = Router::new();
    for method in [
        "Start",
        "Connect",
        "List",
        "SendInput",
        "CloseStdin",
        "SendSignal",
        "Update",
    ] {
        app = app.route(&format!("/process.Process/{method}"), post(dispatch));
    }
    let app = app.with_state(fixture.clone());
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
            "tests/compat/guest_registered_sdk.py",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let settings = json!({"url":format!("http://{address}"),"asynchronous":asynchronous,
        "basic":asynchronous,"mode":mode,"request":request,"keepalive_values":INVALID_KEEPALIVE});
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
        .expect("fixture HTTP connection leaked")
        .unwrap()
        .unwrap();
    assert!(
        status.expect("SDK probe timed out").unwrap().success(),
        "{mode}, async={asynchronous}"
    );
    assert!(fixture.started.load(Ordering::SeqCst));
    settled(&fixture, asynchronous, mode).await;
}

#[tokio::test]
async fn keepalive_rejection_receives_the_bounded_wire_body_without_decoding_or_starting() {
    let fixture = Arc::new(Fixture::new(&json!({"process":{"cmd":"/bin/true"}})).await);
    let claims = fixture.directory.path().join("pid-claims.bin");
    let before = std::fs::read(&claims).unwrap();
    let (release, released) = oneshot::channel();
    let polled = Arc::new(AtomicBool::new(false));
    let observed = polled.clone();
    let body = Body::from_stream(futures_util::stream::once(async move {
        observed.store(true, Ordering::SeqCst);
        released.await.unwrap();
        // Deliberately not a protocol frame: the ORIGINAL header rejection
        // must still win over body decoding after transport receipt finishes.
        Ok::<_, std::io::Error>(b"not a Connect frame".as_slice())
    }));
    let headers = HeaderMap::from_iter([
        (
            "content-type".parse().unwrap(),
            "application/connect+json".parse().unwrap(),
        ),
        (
            "connect-timeout-ms".parse().unwrap(),
            "10000".parse().unwrap(),
        ),
        (
            "keepalive-ping-interval".parse().unwrap(),
            "0".parse().unwrap(),
        ),
        ("x-access-token".parse().unwrap(), "token".parse().unwrap()),
    ]);
    let mut response = Box::pin(dispatch(
        State(fixture.clone()),
        OriginalUri("/process.Process/Start".parse().unwrap()),
        headers,
        body,
    ));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(std::future::Future::poll(response.as_mut(), &mut context).is_pending());
    assert!(polled.load(Ordering::SeqCst));
    assert!(!fixture.started.load(Ordering::SeqCst));
    release.send(()).unwrap();
    let response = response.await;
    assert_eq!(response.status(), 200);
    let wire = axum::body::to_bytes(response.into_body(), LIMIT)
        .await
        .unwrap();
    assert_eq!(wire[0], 2);
    assert_eq!(
        usize::try_from(u32::from_be_bytes(wire[1..5].try_into().unwrap())).unwrap(),
        wire.len() - 5
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&wire[5..]).unwrap(),
        json!({"error":{
        "code":"invalid_argument","message":"keepalive interval must be between 1 and 9223372036 seconds"}})
    );
    assert_eq!(std::fs::read(claims).unwrap(), before);
    assert!(!fixture.directory.path().join("effect").exists());
    assert_eq!(fixture.registry.available_slots(), 1);
    assert!(fixture.owners.reserve().is_ok());
    assert_eq!(fixture.keepalive_rejections.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pinned_sdk_uses_authenticated_original_pid_for_every_command_operation() {
    for asynchronous in [false, true] {
        for mode in ["pipe", "pty", "term", "kill"] {
            probe(asynchronous, mode, None).await;
        }
    }
}

#[tokio::test]
async fn pinned_sdk_uses_exact_tags_and_decodes_duplicate_and_missing_tag_failures() {
    for asynchronous in [false, true] {
        for (mode, tag) in [
            ("pipe", ""),
            ("pty", "123"),
            ("term", "a\"\\<>&中\n"),
            ("kill", "名\u{2028}"),
        ] {
            probe(asynchronous, mode, Some(tag)).await;
        }
    }
}
