//! #99: real durable lifecycle history, with only host side effects replaced.
//! No latency thresholds: the ordinary tests check semantics, the ignored test
//! records monotonic observations for an explicitly requested capacity profile.
use super::*;
use sandproof_api::{SandboxSession, managed::DataKind};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

const LIVE: usize = 2;
const PROBES: [&str; 6] = [
    "file_authorization",
    "command_admission",
    "get",
    "list",
    "idle_recovery",
    "idle_expiry",
];

fn elapsed_ns(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap()
}

fn measured<T>(call: impl FnOnce() -> T) -> (T, u64) {
    let start = Instant::now();
    let result = call();
    (result, elapsed_ns(start))
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct Rows {
    inputs: u64,
    sandboxes: u64,
    operations: u64,
    effects: u64,
    outbox: u64,
    receipts: u64,
}

fn rows(path: &Path) -> Rows {
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let count = |table: &str| {
        u64::try_from(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        )
        .unwrap()
    };
    Rows {
        inputs: count("controller_inputs"),
        sandboxes: count("sandboxes"),
        operations: count("operations"),
        effects: count("controller_effect_keys"),
        outbox: count("outbox"),
        receipts: count("controller_receipt_history"),
    }
}

fn file_bytes(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => panic!("cannot stat {}: {error}", path.display()),
    }
}

#[derive(Clone, Default)]
struct Calls {
    prepare: Arc<AtomicUsize>,
    stop: Arc<AtomicUsize>,
}

impl Calls {
    fn host(&self, live: &[SandboxSession]) -> FakeHost {
        FakeHost {
            prepare_calls: Some(Arc::clone(&self.prepare)),
            stop_calls: Some(Arc::clone(&self.stop)),
            secrets: live
                .iter()
                .map(|session| {
                    let id = session
                        .sandbox_id
                        .rsplit('-')
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap();
                    (id, (session.envd_access_token.clone(), BTreeMap::new()))
                })
                .collect(),
            ..FakeHost::default()
        }
    }

    fn snapshot(&self) -> [usize; 2] {
        [
            self.prepare.load(Ordering::SeqCst),
            self.stop.load(Ordering::SeqCst),
        ]
    }
}

fn policy() -> ManagedConfig {
    let mut policy = config();
    // Two stable live identities plus one transient create/kill during setup.
    policy.limits.max_active_global = 3;
    policy.limits.max_active_per_tenant = 3;
    policy.limits.max_disk_bytes_per_tenant = 3_000;
    policy
}

fn long_request() -> CreateRequest {
    CreateRequest {
        timeout: Some(86_400),
        ..request()
    }
}

fn probe(port: &ManagedPort<FakeHost>, live: &[SandboxSession], name: &str) {
    let session = &live[LIVE - 1];
    match name {
        "file_authorization" => {
            let target = port
                .authorize_data(&session.envd_access_token, DataKind::FileRead)
                .unwrap();
            assert_eq!(target.generation, 1);
        }
        "command_admission" => {
            // This is the production gateway's permit path, not API-key auth
            // or a claim to have started a command in an actual envd/VM.
            let permit = port
                .data_permit(&session.envd_access_token, DataKind::Exec)
                .unwrap();
            assert_eq!(permit.target.generation, 1);
            assert!(!*permit.revocation.borrow());
        }
        "get" => {
            let got = port.get("1", &session.sandbox_id).unwrap();
            assert_eq!(got.info.sandbox_id, session.sandbox_id);
            assert_eq!(got.envd_access_token, session.envd_access_token);
        }
        "list" => {
            let listed = port.list("1", ListQuery::default()).unwrap();
            let actual: BTreeSet<_> = listed.items.iter().map(|item| &item.sandbox_id).collect();
            let expected: BTreeSet<_> = live.iter().map(|item| &item.sandbox_id).collect();
            assert_eq!(actual, expected);
        }
        "idle_recovery" => {
            let report = port.recover_on_startup().unwrap();
            assert_eq!(report.workflows_completed, 0);
            assert_eq!(report.receipts_recovered, 0);
            assert_eq!(report.unresolved_sandboxes, 0);
        }
        "idle_expiry" => assert_eq!(port.expire_due().unwrap(), 0),
        _ => panic!("unknown probe: {name}"),
    }
}

#[derive(Serialize)]
struct Samples {
    raw_ns: Vec<u64>,
    count: usize,
    min_ns: u64,
    p50_ns: u64,
    p95_ns: u64,
    max_ns: u64,
}

impl Samples {
    fn new(raw_ns: Vec<u64>) -> Self {
        assert!(!raw_ns.is_empty());
        let mut sorted = raw_ns.clone();
        sorted.sort_unstable();
        let count = sorted.len();
        Self {
            raw_ns,
            count,
            min_ns: sorted[0],
            p50_ns: sorted[(count * 50).div_ceil(100) - 1],
            p95_ns: sorted[(count * 95).div_ceil(100) - 1],
            max_ns: sorted[count - 1],
        }
    }
}

#[derive(Serialize)]
struct Checkpoint {
    completed_create_kill_cycles: usize,
    active_at_measurement: usize,
    setup_peak_active: usize,
    setup_increment_ns: u64,
    setup_cumulative_ns: u64,
    journal: Rows,
    setup_database_bytes: u64,
    setup_wal_bytes: u64,
    database_bytes: u64,
    wal_bytes: u64,
    host_prepare_stop_calls: [usize; 2],
    reopen_ns: u64,
    first_recovery_after_reopen_ns: u64,
    first_command_admission_after_reopen_ns: u64,
    warm: BTreeMap<&'static str, Samples>,
}

struct Fixture {
    directory: TempDir,
    port: Option<ManagedPort<FakeHost>>,
    live: Vec<SandboxSession>,
    identities: Vec<sandproof_api::managed::DataTarget>,
    calls: Calls,
    completed: usize,
    setup_ns: u64,
}

impl Fixture {
    fn new() -> Self {
        let directory = TempDir::new().unwrap();
        let calls = Calls::default();
        let (port, setup_ns) = measured(|| {
            ManagedPort::new(
                ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
                calls.host(&[]),
                policy(),
            )
        });
        let (live, live_setup): (Vec<SandboxSession>, u64) = measured(|| {
            (0..LIVE)
                .map(|_| port.create("1", long_request()).unwrap())
                .collect()
        });
        let identities = live
            .iter()
            .map(|session: &SandboxSession| {
                port.data_permit(&session.envd_access_token, DataKind::Exec)
                    .unwrap()
                    .target
            })
            .collect();
        Self {
            directory,
            port: Some(port),
            live,
            identities,
            calls,
            completed: 0,
            setup_ns: setup_ns + live_setup,
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.path().join("controller.db")
    }

    fn checkpoint(&mut self, cycles: usize, samples: usize) -> Checkpoint {
        assert!(cycles >= self.completed);
        assert!(samples > 0);
        let port = self.port.as_ref().unwrap();
        let ((), setup_increment_ns) = measured(|| {
            for _ in self.completed..cycles {
                let retired = port.create("1", long_request()).unwrap();
                port.kill("1", &retired.sandbox_id).unwrap();
                assert_eq!(
                    port.authorize_data(&retired.envd_access_token, DataKind::Exec),
                    Err(PortError::NotFound)
                );
            }
        });
        self.completed = cycles;
        self.setup_ns += setup_increment_ns;
        let path = self.path();
        let before = rows(&path);
        let setup_database_bytes = file_bytes(&path);
        let setup_wal_bytes = file_bytes(&path.with_file_name("controller.db-wal"));
        assert_eq!(before.sandboxes, u64::try_from(LIVE + cycles).unwrap());
        assert_eq!(self.calls.snapshot(), [LIVE + cycles, cycles]);
        // Close/reopen the real journal, retaining only the two live host
        // bindings. This is a cold controller connection, not a cold OS cache.
        drop(self.port.take());
        let (journal, reopen_ns) = measured(|| ControllerJournal::open(&path, NAMESPACE).unwrap());
        let port = ManagedPort::new(journal, self.calls.host(&self.live), policy());
        let ((), first_recovery_after_reopen_ns) =
            measured(|| probe(&port, &self.live, "idle_recovery"));
        let ((), first_command_admission_after_reopen_ns) =
            measured(|| probe(&port, &self.live, "command_admission"));
        let mut warm = BTreeMap::new();
        for name in PROBES {
            probe(&port, &self.live, name); // One untimed warm-up for each path.
            warm.insert(
                name,
                Samples::new(
                    (0..samples)
                        .map(|_| measured(|| probe(&port, &self.live, name)).1)
                        .collect(),
                ),
            );
        }
        assert_eq!(
            rows(&path),
            before,
            "read/recovery probes appended durable work"
        );
        assert_eq!(self.calls.snapshot(), [LIVE + cycles, cycles]);
        for (session, identity) in self.live.iter().zip(&self.identities) {
            assert_eq!(
                port.data_permit(&session.envd_access_token, DataKind::Exec)
                    .unwrap()
                    .target,
                *identity
            );
        }
        self.port = Some(port);
        Checkpoint {
            completed_create_kill_cycles: cycles,
            active_at_measurement: LIVE,
            setup_peak_active: if cycles == 0 { LIVE } else { LIVE + 1 },
            setup_increment_ns,
            setup_cumulative_ns: self.setup_ns,
            journal: before,
            setup_database_bytes,
            setup_wal_bytes,
            database_bytes: file_bytes(&path),
            wal_bytes: file_bytes(&path.with_file_name("controller.db-wal")),
            host_prepare_stop_calls: self.calls.snapshot(),
            reopen_ns,
            first_recovery_after_reopen_ns,
            first_command_admission_after_reopen_ns,
            warm,
        }
    }

    fn finish(mut self) {
        let port = self.port.take().unwrap();
        for session in &self.live {
            port.kill("1", &session.sandbox_id).unwrap();
        }
        drop(port);
        let journal = ControllerJournal::open(self.path(), NAMESPACE).unwrap();
        let world = journal.world().unwrap();
        assert!(
            world
                .sandbox_records()
                .iter()
                .all(|(_, record)| record.phase == core::Phase::Destroyed)
        );
        assert!(journal.uncertain_actions().unwrap().is_empty());
        assert_eq!(
            self.calls.snapshot(),
            [LIVE + self.completed, LIVE + self.completed]
        );
    }
}

#[test]
fn real_history_smoke_preserves_fixed_live_authority() {
    let mut fixture = Fixture::new();
    let zero = fixture.checkpoint(0, 1);
    let grown = fixture.checkpoint(3, 1);
    assert!(grown.journal.inputs > zero.journal.inputs);
    assert!(grown.journal.operations > zero.journal.operations);
    assert!(grown.journal.receipts > zero.journal.receipts);
    fixture.finish();
}

#[derive(Serialize)]
struct Interference {
    requested_observation_window_ns: u64,
    actual_observation_window_ns: u64,
    slow_kill_total_ns: u64,
    completed_before_host_release: Vec<String>,
    other_request_total_ns: BTreeMap<String, u64>,
}

// Channels let a failed stop/probe release the host rather than hanging the
// test coordinator in an unpaired barrier. The only wait is externally owned.
pub(super) struct HostGate {
    entered: mpsc::Sender<bool>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl HostGate {
    pub(super) fn block(&self) {
        self.entered.send(true).unwrap();
        self.release.lock().unwrap().recv().unwrap();
    }
}

struct ReleaseOnDrop(mpsc::Sender<()>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn other_probe(
    port: &ManagedPort<FakeHost>,
    session: &SandboxSession,
    name: &str,
) -> Result<(), PortError> {
    match name {
        "file_authorization" => port
            .authorize_data(&session.envd_access_token, DataKind::FileRead)
            .map(|_| ()),
        "command_admission" => port
            .data_permit(&session.envd_access_token, DataKind::Exec)
            .map(|permit| {
                assert!(!*permit.revocation.borrow());
            }),
        "get" => port
            .get("2", &session.sandbox_id)
            .map(|got| assert_eq!(got.info.sandbox_id, session.sandbox_id)),
        "idle_recovery" => port
            .recover_on_startup()
            .map(|report| assert_eq!(report.unresolved_sandboxes, 0)),
        "due_expiry" => port.expire_due().map(|count| assert_eq!(count, 1)),
        _ => unreachable!(),
    }
}

fn finish_interference(
    port: &ManagedPort<FakeHost>,
    calls: &Calls,
    sessions: [&SandboxSession; 3],
    other_permit: &sandproof_api::managed::DataPermit,
) {
    let [first, other, due] = sessions;
    assert_eq!(calls.snapshot(), [3, 2]);
    assert_eq!(port.get("3", &due.sandbox_id), Err(PortError::NotFound));
    assert_eq!(port.get("1", &first.sandbox_id), Err(PortError::NotFound));
    assert_eq!(
        port.authorize_data(&first.envd_access_token, DataKind::Exec),
        Err(PortError::NotFound)
    );
    assert_eq!(
        port.data_permit(&other.envd_access_token, DataKind::Exec)
            .unwrap()
            .target,
        other_permit.target
    );
    assert!(!*other_permit.revocation.borrow());
    port.kill("2", &other.sandbox_id).unwrap();
    assert_eq!(calls.snapshot(), [3, 3]);
}

fn slow_stop_interference(window: Duration) -> Interference {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let gate = Arc::new(HostGate {
        entered: entered_tx.clone(),
        release: Mutex::new(release_rx),
    });
    let calls = Calls::default();
    let directory = TempDir::new().unwrap();
    let port = Arc::new(ManagedPort::new(
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap(),
        FakeHost {
            history_stop_gate: Some((core::SandboxId(1), gate)),
            ..calls.host(&[])
        },
        policy(),
    ));
    let first = port.create("1", long_request()).unwrap();
    assert_eq!(first.sandbox_id, format!("sb-{NAMESPACE}-1"));
    let other = port.create("2", long_request()).unwrap();
    let due = port.create("3", long_request()).unwrap();
    port.set_timeout("3", &due.sandbox_id, 0).unwrap();
    let first_permit = port
        .data_permit(&first.envd_access_token, DataKind::Exec)
        .unwrap();
    let other_permit = port
        .data_permit(&other.envd_access_token, DataKind::Exec)
        .unwrap();
    let (start_tx, start_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let mut before_release = Vec::new();
    let mut timings = BTreeMap::new();
    let (slow_kill_total_ns, actual_observation_window_ns) = std::thread::scope(|scope| {
        let release = ReleaseOnDrop(release_tx);
        let kill = scope.spawn(|| {
            let outcome =
                measured(|| catch_unwind(AssertUnwindSafe(|| port.kill("1", &first.sandbox_id))));
            let _ = entered_tx.send(false); // Failure before StopVm also wakes the coordinator.
            outcome
        });
        assert!(
            entered_rx.recv_timeout(Duration::from_secs(30)).unwrap(),
            "kill ended before the host gate"
        );
        // No port call on the coordinating thread while the host is blocked.
        assert!(*first_permit.revocation.borrow());
        assert!(!*other_permit.revocation.borrow());
        for name in [
            "file_authorization",
            "command_admission",
            "get",
            "idle_recovery",
            "due_expiry",
        ] {
            let port = Arc::clone(&port);
            let session = other.clone();
            let started = start_tx.clone();
            let done = done_tx.clone();
            scope.spawn(move || {
                let start = Instant::now();
                started.send(()).unwrap();
                let result = catch_unwind(AssertUnwindSafe(|| other_probe(&port, &session, name)));
                done.send((name.to_string(), elapsed_ns(start), result))
                    .unwrap();
            });
        }
        drop(start_tx);
        drop(done_tx);
        for _ in 0..5 {
            start_rx.recv_timeout(Duration::from_secs(30)).unwrap();
        }
        let observation_start = Instant::now();
        // An external observer bounds the injected host hold. It never waits
        // for a contending operation before releasing the gate. This is not
        // a scheduling claim that "started" proves Mutex::lock was reached.
        while let Some(remaining) = window.checked_sub(observation_start.elapsed()) {
            match done_rx.recv_timeout(remaining) {
                Ok((name, ns, result)) => {
                    result.unwrap().unwrap();
                    before_release.push(name.clone());
                    timings.insert(name, ns);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => panic!("probe workers disconnected"),
            }
        }
        let actual = elapsed_ns(observation_start);
        drop(release);
        for _ in timings.len()..5 {
            let (name, ns, result) = done_rx.recv().unwrap();
            result.unwrap().unwrap();
            timings.insert(name, ns);
        }
        let (kill_result, kill_ns) = kill.join().unwrap();
        kill_result.unwrap().unwrap();
        (kill_ns, actual)
    });
    finish_interference(&port, &calls, [&first, &other, &due], &other_permit);
    Interference {
        requested_observation_window_ns: u64::try_from(window.as_nanos()).unwrap(),
        actual_observation_window_ns,
        slow_kill_total_ns,
        completed_before_host_release: before_release,
        other_request_total_ns: timings,
    }
}

#[test]
fn blocked_host_preserves_other_identity_and_releases_all_callers() {
    let result = slow_stop_interference(Duration::ZERO);
    assert_eq!(result.other_request_total_ns.len(), 5);
}

fn command_output(program: &str, args: &[&str]) -> String {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
#[ignore = "opt-in capacity observations; no timing SLA or KVM claims"]
fn history_capacity_profile() {
    let points = std::env::var("SANDPROOF_HISTORY_POINTS").unwrap_or_else(|_| "0,4,16,64".into());
    let points: Vec<usize> = points
        .split(',')
        .map(|value| value.parse().unwrap())
        .collect();
    assert!(
        !points.is_empty() && points[0] == 0 && points.windows(2).all(|pair| pair[0] < pair[1])
    );
    assert!(*points.last().unwrap() + LIVE <= usize::try_from(policy().limits.max_vm_id).unwrap());
    let samples: usize = std::env::var("SANDPROOF_HISTORY_SAMPLES")
        .unwrap_or_else(|_| "7".into())
        .parse()
        .unwrap();
    assert!((1..=1_000).contains(&samples));
    let output = PathBuf::from(
        std::env::var("SANDPROOF_HISTORY_OUTPUT")
            .unwrap_or_else(|_| "../artifacts/history-capacity.json".into()),
    );
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    let mut report = serde_json::json!({
        "schema": 1,
        "status": "in_progress",
        "source_revision": std::env::var("SANDPROOF_SOURCE_REVISION").unwrap_or_else(|_| "unrecorded".into()),
        "source_tree": std::env::var("SANDPROOF_SOURCE_TREE").unwrap_or_else(|_| "unrecorded".into()),
        "environment": {
            "rustc": command_output("rustc", &["--version", "--verbose"]),
            "kernel": command_output("uname", &["-srmo"]),
            "sqlite": rusqlite::version(),
            "architecture": std::env::consts::ARCH,
            "available_parallelism": std::thread::available_parallelism().unwrap().get(),
            "debug_assertions": cfg!(debug_assertions),
            "database_directory": std::env::temp_dir(),
            "journal_mode": "WAL", "synchronous": "FULL"
        },
        "method": {
            "clock": "std::time::Instant; raw nanoseconds; nearest-rank quantiles",
            "fixed_active": LIVE,
            "setup": "real ManagedPort create/kill with real SQLite and existing core reducer; setup separately timed",
            "warm": "one untimed call per path, then serial samples; OS/SQLite caches and volatile committed-World cache warm",
            "reopen": "drop/reopen real journal and one startup recovery; same process and warm filesystem cache, not a machine reboot",
            "command_admission": "ManagedPort::data_permit(Exec), including revocation subscription; no envd command execution",
            "host": "existing protocol-conforming FakeHost; no KVM or physical resource evidence",
            "interference": "A barrier-held StopVm; B file/command/get and global recovery, plus real expiry of C made due by set_timeout(0); request time includes mutex contention, scheduler delay and replay",
            "scope": "component observations only; no supported-history limit, production SLA or capacity refusal threshold inferred"
        },
        "requested_points": points,
        "samples_per_warm_probe": samples,
        "checkpoints": [],
    });
    let write = |report: &serde_json::Value| {
        let mut temporary = tempfile::NamedTempFile::new_in(output.parent().unwrap()).unwrap();
        serde_json::to_writer_pretty(temporary.as_file_mut(), report).unwrap();
        temporary.as_file().sync_all().unwrap();
        temporary.persist(&output).unwrap();
    };
    write(&report);
    let mut fixture = Fixture::new();
    for point in points {
        report["pending_checkpoint"] = point.into();
        write(&report);
        let checkpoint = fixture.checkpoint(point, samples);
        eprintln!(
            "history checkpoint complete: cycles={point}, inputs={}, command_p50_ns={}",
            checkpoint.journal.inputs, checkpoint.warm["command_admission"].p50_ns
        );
        report["checkpoints"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(checkpoint).unwrap());
        write(&report);
    }
    fixture.finish();
    report["pending_checkpoint"] = serde_json::Value::Null;
    report["interference"] =
        serde_json::to_value(slow_stop_interference(Duration::from_millis(250))).unwrap();
    report["status"] = "complete".into();
    write(&report);
    eprintln!("capacity observations: {}", output.display());
}
