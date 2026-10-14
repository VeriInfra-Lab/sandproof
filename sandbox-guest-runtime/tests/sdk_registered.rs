//! Fixed SDK -> restricted loopback fixture -> authenticated PID components.
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
    http::{HeaderMap, Method},
    response::{IntoResponse, Response},
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
    command_requests::{self, CommandRequests, StartPreparation, http::Rpc},
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
    session: Session<MetadataFixture>,
    accounts: AccountPool<CurrentAccount>,
    lookups: Arc<AtomicUsize>,
    calls: AtomicUsize,
    requests: CommandRequests,
    owners: CommandOwners,
    registry: PidRegistry,
    registrar: PidRegistrar,
    expected: Request,
    started: AtomicBool,
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
        let bytes = serde_json::to_vec(request).unwrap();
        let mut framed = vec![0];
        framed.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
        framed.extend(bytes);
        Self {
            directory,
            session,
            accounts,
            lookups,
            calls: AtomicUsize::new(0),
            requests: CommandRequests::new(command_requests::Limits {
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
            .unwrap(),
            owners: CommandOwners::new(OwnerLimits {
                records: 1,
                subscribers: 3,
                queued_chunks: 16,
                output_bytes: 8192,
                stdin_bytes: 8192,
            })
            .unwrap(),
            registry: PidRegistry::new(
                binding(),
                1,
                RegistryLimits {
                    records: 1,
                    metadata_bytes: 4096,
                    retention: Duration::from_secs(30),
                },
            )
            .unwrap(),
            registrar: PidRegistrar::new(claims),
            expected: decode_request("Start", &framed, LIMIT).unwrap(),
            started: AtomicBool::new(false),
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
    let rpc = match uri.path() {
        "/process.Process/Start" => Rpc::Start,
        "/process.Process/Connect" => Rpc::Connect,
        "/process.Process/List" => Rpc::List,
        "/process.Process/SendInput" => Rpc::SendInput,
        "/process.Process/CloseStdin" => Rpc::CloseStdin,
        "/process.Process/SendSignal" => Rpc::SendSignal,
        "/process.Process/Update" => Rpc::Update,
        _ => panic!("unexpected fixture method"),
    };
    let prepared = fixture
        .requests
        .admit()
        .unwrap()
        .authenticate(&fixture.session, headers)
        .await
        .unwrap()
        .prepare(&fixture.accounts, settings().limits)
        .await
        .unwrap();
    let stream = rpc.is_stream();
    selected_json(prepared.headers(), stream);
    let selected = prepared.select_json(&Method::POST, rpc).unwrap();
    let content_type = selected.content_type();
    let ready = selected.decode_identity(body).await.unwrap();
    fixture.calls.fetch_add(1, Ordering::SeqCst);
    let limits = CommandBodyLimits {
        message_bytes: 4096,
        keepalive: Duration::from_secs(5),
    };
    let reply = if rpc == Rpc::Start {
        assert_eq!(ready.request(), &fixture.expected);
        assert!(!fixture.started.swap(true, Ordering::SeqCst));
        let gates = fixture.directory.path().join("gates");
        let reply = ready
            .start_pid(
                &fixture.accounts,
                &settings(),
                StartPreparation {
                    owners: &fixture.owners,
                    registry: &fixture.registry,
                    registrar: &fixture.registrar,
                    binding: binding(),
                    session: 1,
                    command: 1,
                    worker: Path::new(WORKER),
                    private_directory: &gates,
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
                limits,
            )
            .await
            .unwrap();
        let listed: Value =
            serde_json::from_slice(&fixture.registry.list_json(4096).unwrap()).unwrap();
        let pid = u32::try_from(listed["processes"][0]["pid"].as_u64().unwrap()).unwrap();
        *fixture.command.lock().unwrap() = Some(Stop(fixture.registry.connectable(pid).unwrap()));
        reply
    } else {
        ready.execute_pid(&fixture.registry, limits).await.unwrap()
    };
    assert_eq!(reply.is_stream(), stream);
    ([("content-type", content_type)], reply.into_body()).into_response()
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
    let slots: Vec<_> = (0..REQUESTS)
        .map(|_| fixture.requests.admit().unwrap())
        .collect();
    drop(slots);
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
            1
        }
    );
}

async fn probe(asynchronous: bool, mode: &str) {
    let script = match mode {
        "pipe" | "term" | "kill" => {
            "printf once >> effect; printf '%s|ready' \"$VALUE\"; exec /bin/cat"
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
        "basic":asynchronous,"mode":mode,"request":request});
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
async fn pinned_sdk_uses_authenticated_original_pid_for_every_command_operation() {
    for asynchronous in [false, true] {
        for mode in ["pipe", "pty", "term", "kill"] {
            probe(asynchronous, mode).await;
        }
    }
}
