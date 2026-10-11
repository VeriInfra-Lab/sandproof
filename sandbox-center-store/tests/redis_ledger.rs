#![cfg(unix)]

use redis::{Connection, ConnectionAddr, IntoConnectionInfo, RedisConnectionInfo};
use sandproof_center_store::{
    ControlState, REDIS_VERSION, RedisEndpoint, RedisLedger, StoreConfig, StoreError, WriterLease,
};
use sandproof_protocol::DeploymentId;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const MEMORY: u64 = 64 << 20;
const KEY: &str = "sandproof:authority:fixture-deployment";
const PASSWORD: &str = "component-fixture-not-a-production-credential";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    frontier: u64,
    pending: String,
}
impl ControlState for State {
    const FORMAT_VERSION: u32 = 1;
    fn is_valid(&self) -> bool {
        self.frontier > 0
    }
}
fn state(value: u64) -> State {
    State {
        frontier: value,
        pending: "original-operation".into(),
    }
}
fn deployment() -> DeploymentId {
    "fixture-deployment".try_into().unwrap()
}
fn endpoint(address: SocketAddrV4) -> RedisEndpoint {
    RedisEndpoint::new(address, "default".into(), PASSWORD.into()).unwrap()
}
fn connect(address: SocketAddrV4) -> Connection {
    let info = ConnectionAddr::Tcp(address.ip().to_string(), address.port())
        .into_connection_info()
        .unwrap()
        .set_redis_settings(
            RedisConnectionInfo::default()
                .set_password(PASSWORD)
                .set_skip_set_lib_name(),
        );
    let connection = redis::Client::open(info)
        .unwrap()
        .get_connection_with_timeout(Duration::from_secs(1))
        .unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    connection
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    connection
}

struct Engine {
    root: tempfile::TempDir,
    child: Option<Child>,
    address: SocketAddrV4,
    binary: String,
}
impl Engine {
    fn new() -> Self {
        let binary = std::env::var("SANDPROOF_REDIS_SERVER").expect(
            "mandatory fixed Redis fixture missing: set SANDPROOF_REDIS_SERVER (never skip)",
        );
        assert!(std::path::Path::new(&binary).is_absolute());
        let version = Command::new(&binary).arg("--version").output().unwrap();
        assert!(version.status.success());
        assert!(
            String::from_utf8(version.stdout)
                .unwrap()
                .split_whitespace()
                .any(|field| field == format!("v={REDIS_VERSION}"))
        );
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(root.path().join("writer")).unwrap();
        fs::set_permissions(
            root.path().join("writer"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let config = format!(
            "bind 127.0.0.1\nport {port}\nprotected-mode yes\nrequirepass {PASSWORD}\nappendonly yes\nappendfsync always\nmaxmemory {MEMORY}\nmaxmemory-policy noeviction\nno-appendfsync-on-rewrite no\naof-load-truncated no\nauto-aof-rewrite-percentage 0\nsave \"\"\ndir {}\nlogfile {}\n",
            root.path().display(),
            root.path().join("redis.log").display()
        );
        fs::write(root.path().join("redis.conf"), config).unwrap();
        fs::set_permissions(
            root.path().join("redis.conf"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mut fixture = Self {
            root,
            child: None,
            address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
            binary,
        };
        fixture.start();
        fixture
    }
    fn spawn(&mut self) {
        assert!(self.child.is_none());
        self.child = Some(
            Command::new(&self.binary)
                .arg(self.root.path().join("redis.conf"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    fn start(&mut self) {
        self.spawn();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "fixture Redis exited before ready"
            );
            if let Ok(stream) =
                TcpStream::connect_timeout(&self.address.into(), Duration::from_millis(30))
            {
                drop(stream);
                let mut connection = connect(self.address);
                // Listening starts before AOF loading finishes. Poll actual
                // readiness under the fixture deadline, not just an open port.
                if matches!(redis::cmd("PING").query::<String>(&mut connection), Ok(reply) if reply == "PONG")
                {
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "fixture Redis readiness deadline"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
    fn lease(&self) -> WriterLease {
        WriterLease::acquire(&self.root.path().join("writer"), deployment()).unwrap()
    }
    fn config(limit: usize) -> StoreConfig {
        StoreConfig::new(deployment(), limit, MEMORY, Duration::from_secs(2)).unwrap()
    }
    fn initialize(
        &self,
        limit: usize,
    ) -> (RedisLedger<State>, sandproof_center_store::Observed<State>) {
        let (ledger, committed) = RedisLedger::initialize(
            endpoint(self.address),
            Self::config(limit),
            self.lease(),
            &state(1),
        )
        .unwrap();
        (ledger, committed.into_observed())
    }
    fn field(&self, name: &str, value: impl redis::ToRedisArgs) {
        let _: usize = redis::cmd("HSET")
            .arg(KEY)
            .arg(name)
            .arg(value)
            .query(&mut connect(self.address))
            .unwrap();
    }
    fn revision(&self) -> String {
        redis::cmd("HGET")
            .arg(KEY)
            .arg("revision")
            .query(&mut connect(self.address))
            .unwrap()
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn atomic_commit_conflict_and_reaffirm_keep_original_binding() {
    let fixture = Engine::new();
    let (mut ledger, first) = fixture.initialize(1024);
    let second = ledger.commit(&first, &state(2)).unwrap();
    assert_eq!(second.revision().get(), 2);
    assert!(matches!(
        ledger.commit(&first, &state(3)),
        Err(StoreError::Conflict)
    ));
    let observed = ledger.read().unwrap();
    assert_eq!(observed.state(), &state(2));
    let recovered = ledger.reaffirm(&observed).unwrap();
    assert_eq!(recovered.revision().get(), 3);
    assert_eq!(recovered.state(), observed.state());
    let keys: Vec<String> = redis::cmd("KEYS")
        .arg("*")
        .query(&mut connect(fixture.address))
        .unwrap();
    assert_eq!(keys, [KEY]);
    let fields: usize = redis::cmd("HLEN")
        .arg(KEY)
        .query(&mut connect(fixture.address))
        .unwrap();
    assert_eq!(fields, 4);
}

#[test]
fn ordinary_open_never_initializes_missing_or_overwrites_foreign_data() {
    let fixture = Engine::new();
    assert!(matches!(
        RedisLedger::<State>::open(
            endpoint(fixture.address),
            Engine::config(1024),
            fixture.lease()
        ),
        Err(StoreError::Missing)
    ));
    let _: () = redis::cmd("SET")
        .arg(KEY)
        .arg("foreign-fixture")
        .query(&mut connect(fixture.address))
        .unwrap();
    assert!(matches!(
        RedisLedger::initialize(
            endpoint(fixture.address),
            Engine::config(1024),
            fixture.lease(),
            &state(1)
        ),
        Err(StoreError::AlreadyInitialized)
    ));
    assert!(matches!(
        RedisLedger::<State>::open(
            endpoint(fixture.address),
            Engine::config(1024),
            fixture.lease()
        ),
        Err(StoreError::InvalidRecord)
    ));
    let unchanged: String = redis::cmd("GET")
        .arg(KEY)
        .query(&mut connect(fixture.address))
        .unwrap();
    assert_eq!(unchanged, "foreign-fixture");
}

#[test]
fn payload_capacity_and_same_revision_substitution_refuse_without_mutation() {
    let fixture = Engine::new();
    let length = serde_json::to_vec(&state(1)).unwrap().len();
    let (mut ledger, original) = fixture.initialize(length);
    let mut large = state(2);
    large.pending.push('x');
    assert!(matches!(
        ledger.commit(&original, &large),
        Err(StoreError::Capacity)
    ));
    assert!(matches!(
        ledger.commit(&original, &state(0)),
        Err(StoreError::InvalidState)
    ));
    assert_eq!(fixture.revision(), "1");
    fixture.field("payload", serde_json::to_vec(&state(9)).unwrap());
    assert!(matches!(
        ledger.commit(&original, &state(2)),
        Err(StoreError::InvalidRecord)
    ));
    assert_eq!(fixture.revision(), "1");
}

#[test]
fn malformed_metadata_payload_and_ttl_fail_closed() {
    let fixture = Engine::new();
    let (mut ledger, _) = fixture.initialize(1024);
    for (field, original, bad) in [
        ("deployment", "fixture-deployment", "foreign"),
        ("format", "1", "2"),
        ("revision", "1", "0"),
        ("revision", "1", "01"),
        ("revision", "1", "18446744073709551616"),
    ] {
        fixture.field(field, bad);
        assert!(matches!(ledger.read(), Err(StoreError::InvalidRecord)));
        fixture.field(field, original);
    }
    for payload in [
        "{",
        "null",
        r#"{"frontier":1,"frontier":2,"pending":"x"}"#,
        r#"{"frontier":1,"pending":"x","extra":0}"#,
        r#"{"frontier":0,"pending":"x"}"#,
    ] {
        fixture.field("payload", payload);
        assert!(matches!(
            ledger.read(),
            Err(StoreError::InvalidRecord | StoreError::InvalidState)
        ));
    }
    fixture.field("payload", "x".repeat(1025));
    assert!(matches!(ledger.read(), Err(StoreError::InvalidRecord)));
    fixture.field("payload", serde_json::to_vec(&state(1)).unwrap());
    fixture.field("extra", "unexpected");
    assert!(matches!(ledger.read(), Err(StoreError::InvalidRecord)));
    let _: usize = redis::cmd("HDEL")
        .arg(KEY)
        .arg("extra")
        .query(&mut connect(fixture.address))
        .unwrap();
    let _: bool = redis::cmd("PEXPIRE")
        .arg(KEY)
        .arg(60_000)
        .query(&mut connect(fixture.address))
        .unwrap();
    assert!(matches!(ledger.read(), Err(StoreError::InvalidRecord)));
}

#[test]
fn revision_above_lua_integer_precision_is_exact_and_exhaustion_never_wraps() {
    let fixture = Engine::new();
    let (mut ledger, _) = fixture.initialize(1024);
    fixture.field("revision", 9_007_199_254_740_993_u64);
    let observed = ledger.read().unwrap();
    assert_eq!(
        ledger
            .commit(&observed, &state(2))
            .unwrap()
            .revision()
            .get(),
        9_007_199_254_740_994
    );
    fixture.field("revision", u64::MAX);
    let observed = ledger.read().unwrap();
    assert!(matches!(
        ledger.commit(&observed, &state(3)),
        Err(StoreError::RevisionExhausted)
    ));
    assert_eq!(fixture.revision(), u64::MAX.to_string());
}

#[test]
fn every_required_persistence_setting_is_checked_before_write() {
    let fixture = Engine::new();
    let (mut ledger, original) = fixture.initialize(1024);
    for (name, good, bad) in [
        ("appendfsync", "always", "everysec"),
        ("maxmemory-policy", "noeviction", "allkeys-lru"),
        ("no-appendfsync-on-rewrite", "no", "yes"),
        ("aof-load-truncated", "no", "yes"),
        ("maxmemory", "67108864", "67108863"),
    ] {
        let _: () = redis::cmd("CONFIG")
            .arg("SET")
            .arg(name)
            .arg(bad)
            .query(&mut connect(fixture.address))
            .unwrap();
        assert!(matches!(
            ledger.commit(&original, &state(2)),
            Err(StoreError::Configuration)
        ));
        assert_eq!(fixture.revision(), "1");
        let _: () = redis::cmd("CONFIG")
            .arg("SET")
            .arg(name)
            .arg(good)
            .query(&mut connect(fixture.address))
            .unwrap();
    }
    let _: () = redis::cmd("CONFIG")
        .arg("SET")
        .arg("appendonly")
        .arg("no")
        .query(&mut connect(fixture.address))
        .unwrap();
    assert!(matches!(
        ledger.commit(&original, &state(2)),
        Err(StoreError::Configuration)
    ));
    assert_eq!(fixture.revision(), "1");
}

#[test]
fn engine_crash_recovers_same_state_and_requires_explicit_reaffirm() {
    let mut fixture = Engine::new();
    let (mut ledger, original) = fixture.initialize(1024);
    ledger.commit(&original, &state(2)).unwrap();
    drop(ledger);
    fixture.stop();
    fixture.start();
    let (mut recovered, observed) = RedisLedger::<State>::open(
        endpoint(fixture.address),
        Engine::config(1024),
        fixture.lease(),
    )
    .unwrap();
    assert_eq!(observed.state(), &state(2));
    assert_eq!(observed.revision().get(), 2);
    assert_eq!(recovered.reaffirm(&observed).unwrap().revision().get(), 3);
}

#[test]
fn truncated_aof_refuses_start_without_repairing_the_original_file() {
    let mut fixture = Engine::new();
    let (ledger, _) = fixture.initialize(1024);
    drop(ledger);
    fixture.stop();
    let path = fs::read_dir(fixture.root.path().join("appendonlydir"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".incr.aof"))
        .unwrap();
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"*3\r\n$3\r\nSET\r\n$7\r\nbroken").unwrap();
    file.sync_all().unwrap();
    let bytes = fs::read(&path).unwrap();
    fixture.spawn();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(exit) = fixture.child.as_mut().unwrap().try_wait().unwrap() {
            assert!(!exit.success());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    fixture.child = None; // reaped above; never kill an unrelated process
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(matches!(
        RedisLedger::<State>::open(
            endpoint(fixture.address),
            Engine::config(1024),
            fixture.lease()
        ),
        Err(StoreError::Unavailable)
    ));
}

// Controlled proxy changes only delivery of an actual Redis reply. It neither
// implements nor mocks the authority store. Frames/threads have fixed budgets.
struct ReplyLoss {
    address: SocketAddrV4,
    armed: Arc<AtomicU8>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl ReplyLoss {
    fn new(target: SocketAddrV4) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port());
        let armed = Arc::new(AtomicU8::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_arm = Arc::clone(&armed);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            for incoming in listener.incoming() {
                if worker_stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(client) = incoming else {
                    break;
                };
                let upstream =
                    TcpStream::connect_timeout(&target.into(), Duration::from_secs(1)).unwrap();
                for stream in [&client, &upstream] {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                }
                let mut client = BufReader::new(client);
                let mut upstream = BufReader::new(upstream);
                while let Ok(request) = frame(&mut client) {
                    let fault = if request.windows(4).any(|part| part == b"HSET") {
                        worker_arm.swap(0, Ordering::SeqCst)
                    } else {
                        0
                    };
                    if fault == 2 {
                        // Real noeviction refusal after the adapter's health
                        // check, not a fabricated Redis error response.
                        let _: () = redis::cmd("CONFIG")
                            .arg("SET")
                            .arg("maxmemory")
                            .arg(1)
                            .query(&mut connect(target))
                            .unwrap();
                    }
                    if upstream.get_mut().write_all(&request).is_err() {
                        break;
                    }
                    let Ok(reply) = frame(&mut upstream) else {
                        break;
                    };
                    if fault == 1 {
                        assert_eq!(reply, b":0\r\n");
                        break; // commit acknowledged by real engine, not by caller
                    }
                    if client.get_mut().write_all(&reply).is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            address,
            armed,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for ReplyLoss {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.address.into(), Duration::from_secs(1));
        self.worker.take().unwrap().join().unwrap();
    }
}
fn frame(reader: &mut BufReader<TcpStream>) -> std::io::Result<Vec<u8>> {
    fn read(
        reader: &mut BufReader<TcpStream>,
        output: &mut Vec<u8>,
        depth: usize,
    ) -> std::io::Result<()> {
        if depth > 8 {
            return Err(std::io::Error::other("fixture frame depth"));
        }
        let mut line = Vec::new();
        loop {
            let mut byte = [0];
            reader.read_exact(&mut byte)?;
            line.push(byte[0]);
            if line.len() > 4096 {
                return Err(std::io::Error::other("fixture header bound"));
            }
            if line.ends_with(b"\r\n") {
                break;
            }
        }
        let number = || -> std::io::Result<usize> {
            std::str::from_utf8(&line[1..line.len() - 2])
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|value| *value < 32768)
                .ok_or_else(|| std::io::Error::other("fixture frame bound"))
        };
        let prefix = line[0];
        let count = if matches!(prefix, b'$' | b'*') {
            Some(number()?)
        } else {
            None
        };
        output.extend_from_slice(&line);
        match prefix {
            b'$' => {
                let mut bytes = vec![0; count.unwrap() + 2];
                reader.read_exact(&mut bytes)?;
                output.extend(bytes);
            }
            b'*' => {
                for _ in 0..count.unwrap() {
                    read(reader, output, depth + 1)?;
                }
            }
            b'+' | b'-' | b':' => {}
            _ => return Err(std::io::Error::other("fixture RESP2")),
        }
        if output.len() > 65536 {
            return Err(std::io::Error::other("fixture aggregate bound"));
        }
        Ok(())
    }
    let mut output = Vec::new();
    read(reader, &mut output, 0)?;
    Ok(output)
}

#[test]
fn actual_commit_reply_loss_fences_until_original_state_is_durably_reaffirmed() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let (mut ledger, committed) = RedisLedger::initialize(
        endpoint(proxy.address),
        Engine::config(1024),
        fixture.lease(),
        &state(1),
    )
    .unwrap();
    let original = committed.into_observed();
    proxy.armed.store(1, Ordering::SeqCst);
    assert!(matches!(
        ledger.commit(&original, &state(2)),
        Err(StoreError::OutcomeUnknown)
    ));
    assert!(ledger.writes_fenced());
    assert_eq!(fixture.revision(), "2");
    assert!(matches!(
        ledger.commit(&original, &state(3)),
        Err(StoreError::WriteFenced)
    ));
    ledger.reconnect().unwrap();
    let observed = ledger.read().unwrap();
    assert_eq!(observed.state(), &state(2));
    assert!(ledger.writes_fenced());
    assert!(matches!(
        ledger.commit(&observed, &state(3)),
        Err(StoreError::WriteFenced)
    ));
    let confirmed = ledger.reaffirm(&observed).unwrap();
    assert_eq!(confirmed.revision().get(), 3);
    assert_eq!(confirmed.state(), &state(2));
    assert!(!ledger.writes_fenced());
    drop(ledger); // release connection before joining proxy
}

#[test]
fn actual_noeviction_write_refusal_keeps_original_record_and_requires_reconciliation() {
    let fixture = Engine::new();
    let proxy = ReplyLoss::new(fixture.address);
    let (mut ledger, committed) = RedisLedger::initialize(
        endpoint(proxy.address),
        Engine::config(1024),
        fixture.lease(),
        &state(1),
    )
    .unwrap();
    proxy.armed.store(2, Ordering::SeqCst);
    assert!(matches!(
        ledger.commit(&committed.into_observed(), &state(2)),
        Err(StoreError::OutcomeUnknown)
    ));
    assert!(ledger.writes_fenced());
    assert_eq!(fixture.revision(), "1");
    let _: () = redis::cmd("CONFIG")
        .arg("SET")
        .arg("maxmemory")
        .arg(MEMORY)
        .query(&mut connect(fixture.address))
        .unwrap();
    ledger.reconnect().unwrap();
    let observed = ledger.read().unwrap();
    assert_eq!(observed.state(), &state(1));
    assert_eq!(ledger.reaffirm(&observed).unwrap().revision().get(), 2);
    drop(ledger);
}

#[test]
fn bad_acl_or_writer_deployment_cannot_open_or_mutate_the_authority() {
    let fixture = Engine::new();
    let (ledger, _) = fixture.initialize(1024);
    drop(ledger);
    let bad = RedisEndpoint::new(
        fixture.address,
        "default".into(),
        "incorrect-fixture".into(),
    )
    .unwrap();
    assert!(matches!(
        RedisLedger::<State>::open(bad, Engine::config(1024), fixture.lease()),
        Err(StoreError::Unavailable)
    ));
    let foreign = WriterLease::acquire(
        &fixture.root.path().join("writer"),
        "foreign-deployment".try_into().unwrap(),
    )
    .unwrap();
    assert!(matches!(
        RedisLedger::<State>::open(endpoint(fixture.address), Engine::config(1024), foreign),
        Err(StoreError::Configuration)
    ));
    assert_eq!(fixture.revision(), "1");
}

#[test]
fn stalled_real_engine_has_finite_operation_deadline_and_no_automatic_reconnect() {
    let fixture = Engine::new();
    let (ledger, _) = fixture.initialize(1024);
    drop(ledger);
    let config = StoreConfig::new(deployment(), 1024, MEMORY, Duration::from_millis(50)).unwrap();
    let (mut ledger, _) =
        RedisLedger::<State>::open(endpoint(fixture.address), config, fixture.lease()).unwrap();
    let mut admin = connect(fixture.address);
    let _: () = redis::cmd("CLIENT")
        .arg("PAUSE")
        .arg(300)
        .arg("ALL")
        .query(&mut admin)
        .unwrap();
    let start = Instant::now();
    assert!(matches!(ledger.read(), Err(StoreError::Unavailable)));
    assert!(start.elapsed() < Duration::from_secs(1));
    let _: () = redis::cmd("CLIENT")
        .arg("UNPAUSE")
        .query(&mut admin)
        .unwrap();
    assert!(matches!(ledger.read(), Err(StoreError::Unavailable))); // old stream discarded
    ledger.reconnect().unwrap();
    assert_eq!(ledger.read().unwrap().state(), &state(1));
}
