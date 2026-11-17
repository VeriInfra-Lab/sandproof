//! One fixed real Redis engine fixture shared by storage and service suites.
//! Each instance has a private directory, endpoint, writer lock and AOF.
//! Missing approved material fails; these helpers never download or skip.
use redis::{Connection, ConnectionAddr, IntoConnectionInfo, RedisConnectionInfo};
use sandproof_center_store::{RedisEndpoint, StoreConfig, WriterLease};
use sandproof_protocol::DeploymentId;
use std::{
    fs,
    io::Read,
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

pub(crate) const MEMORY: u64 = 64 << 20;
pub(crate) const KEY: &str = "sandproof:authority:fixture-deployment";
// Keep this process's parallel fixtures from selecting a just-released port
// before the original Redis child binds it. Only startup holds this guard;
// test bodies and their concurrency scenarios remain independent.
static STARTUP: Mutex<()> = Mutex::new(());
const PASSWORD: &str = "component-fixture-not-a-production-credential";
#[path = "engine/initial.rs"]
mod initial;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FixtureIdentityError {
    MissingChild,
    ChildExited,
    ObservationUnavailable,
    UnknownProcess,
    WrongProcess { expected: u32, observed: u32 },
}

pub(crate) fn deployment() -> DeploymentId {
    "fixture-deployment".try_into().unwrap()
}
pub(crate) fn endpoint(address: SocketAddrV4) -> RedisEndpoint {
    RedisEndpoint::new(address, "default".into(), PASSWORD.into()).unwrap()
}
pub(crate) fn connect(address: SocketAddrV4) -> Connection {
    try_connect(address).expect("ready fixture Redis connection")
}
fn try_connect(address: SocketAddrV4) -> redis::RedisResult<Connection> {
    let info = ConnectionAddr::Tcp(address.ip().to_string(), address.port())
        .into_connection_info()
        .unwrap()
        .set_redis_settings(
            RedisConnectionInfo::default()
                .set_password(PASSWORD)
                .set_skip_set_lib_name(),
        );
    let connection =
        redis::Client::open(info)?.get_connection_with_timeout(Duration::from_secs(1))?;
    connection.set_read_timeout(Some(Duration::from_secs(1)))?;
    connection.set_write_timeout(Some(Duration::from_secs(1)))?;
    Ok(connection)
}

pub(crate) struct Engine {
    pub(crate) root: tempfile::TempDir,
    pub(crate) child: Option<Child>,
    pub(crate) address: SocketAddrV4,
    binary: String,
}
impl Engine {
    pub(crate) fn new() -> Self {
        Self::initial(|_, reservation, _| drop(reservation), None)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn prepare(binary: String) -> (Self, TcpListener) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        // Snapshot fixtures may require loop XFS, but this fsync-backed Redis
        // fixture does not. Keep its private AOF on the operator-selected disk
        // without changing the workspace TMPDIR or any request deadline.
        let root = if let Some(parent) = std::env::var_os("SANDPROOF_REDIS_TEST_TMPDIR") {
            let parent = std::path::PathBuf::from(parent);
            assert!(
                parent.is_absolute(),
                "Redis fixture parent must be absolute"
            );
            tempfile::tempdir_in(parent).expect("explicit Redis fixture parent must be usable")
        } else {
            tempfile::tempdir().unwrap()
        };
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
        let fixture = Self {
            root,
            child: None,
            address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
            binary,
        };
        (fixture, listener)
    }
    pub(crate) fn spawn(&mut self) {
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
    pub(crate) fn start(&mut self) {
        self.spawn();
        // An existing fixture always keeps its endpoint and original AOF.
        self.wait_ready(Instant::now() + initial::READY_TIMEOUT)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    fn wait_ready(&mut self, deadline: Instant) -> Result<(), initial::StartupFailure> {
        loop {
            if Instant::now() >= deadline {
                return Err(initial::StartupFailure::Deadline(
                    "fixture Redis readiness deadline",
                ));
            }
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                // Synthetic fixture logs only, bounded before TempDir cleanup.
                // Keep the actual startup cause; never retry an unknown engine
                // or mistake a different process's open port for readiness.
                let diagnostic = self.startup_diagnostic();
                return Err(initial::StartupFailure::Exited {
                    pid: self.child.as_ref().unwrap().id(),
                    status,
                    diagnostic,
                });
            }
            if let Ok(stream) =
                TcpStream::connect_timeout(&self.address.into(), Duration::from_millis(30))
            {
                drop(stream);
                let Ok(mut connection) = try_connect(self.address) else {
                    if Instant::now() >= deadline {
                        return Err(initial::StartupFailure::Deadline(
                            "fixture Redis authentication/readiness deadline",
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                // Listening starts before AOF loading finishes. Poll actual
                // readiness under the fixture deadline, not just an open port.
                if matches!(redis::cmd("PING").query::<String>(&mut connection), Ok(reply) if reply == "PONG")
                {
                    if let Err(error) = self.verify_ready_identity(&mut connection) {
                        let diagnostic = self.startup_diagnostic();
                        return Err(initial::StartupFailure::Identity { error, diagnostic });
                    }
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err(initial::StartupFailure::Deadline(
                    "fixture Redis readiness deadline",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn startup_diagnostic(&self) -> String {
        let mut diagnostic = String::new();
        let observed = fs::File::open(self.root.path().join("redis.log"))
            .and_then(|file| file.take(4096).read_to_string(&mut diagnostic));
        if let Err(error) = observed {
            return format!("fixture Redis startup log unavailable: {error}");
        }
        diagnostic
    }

    /// A PONG/fixture password does not select the original child process.
    /// Both process observations are independent of the server's INFO value.
    /// # Errors
    /// Unknown/mismatched identity or an exited child cannot become readiness.
    pub(crate) fn verify_ready_identity(
        &mut self,
        connection: &mut Connection,
    ) -> Result<(), FixtureIdentityError> {
        let child = self
            .child
            .as_mut()
            .ok_or(FixtureIdentityError::MissingChild)?;
        let expected = child.id();
        if child
            .try_wait()
            .map_err(|_| FixtureIdentityError::ObservationUnavailable)?
            .is_some()
        {
            return Err(FixtureIdentityError::ChildExited);
        }
        let info: String = redis::cmd("INFO")
            .arg("server")
            .query(connection)
            .map_err(|_| FixtureIdentityError::ObservationUnavailable)?;
        let mut observed = None;
        for line in info.lines() {
            if let Some(value) = line.strip_prefix("process_id:") {
                if observed.is_some() {
                    return Err(FixtureIdentityError::UnknownProcess);
                }
                let pid = value
                    .parse::<u32>()
                    .map_err(|_| FixtureIdentityError::UnknownProcess)?;
                if pid == 0 {
                    return Err(FixtureIdentityError::UnknownProcess);
                }
                observed = Some(pid);
            }
        }
        let observed = observed.ok_or(FixtureIdentityError::UnknownProcess)?;
        if observed != expected {
            return Err(FixtureIdentityError::WrongProcess { expected, observed });
        }
        if child
            .try_wait()
            .map_err(|_| FixtureIdentityError::ObservationUnavailable)?
            .is_some()
        {
            return Err(FixtureIdentityError::ChildExited);
        }
        Ok(())
    }
    pub(crate) fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }
    pub(crate) fn lease(&self) -> WriterLease {
        WriterLease::acquire(&self.root.path().join("writer"), deployment()).unwrap()
    }
    pub(crate) fn config(limit: usize) -> StoreConfig {
        StoreConfig::new(deployment(), limit, MEMORY, Duration::from_secs(2)).unwrap()
    }
    pub(crate) fn field(&self, name: &str, value: impl redis::ToRedisArgs) {
        let _: usize = redis::cmd("HSET")
            .arg(KEY)
            .arg(name)
            .arg(value)
            .query(&mut connect(self.address))
            .unwrap();
    }
    pub(crate) fn revision(&self) -> String {
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
