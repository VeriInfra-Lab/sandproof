//! One fixed real Redis engine fixture shared by storage and service suites.
//! Each instance has a private directory, endpoint, writer lock and AOF.
//! Missing approved material fails; these helpers never download or skip.
use redis::{Connection, ConnectionAddr, IntoConnectionInfo, RedisConnectionInfo};
use sandproof_center_store::{REDIS_VERSION, RedisEndpoint, StoreConfig, WriterLease};
use sandproof_protocol::DeploymentId;
use std::{
    fs,
    io::Read,
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(crate) const MEMORY: u64 = 64 << 20;
pub(crate) const KEY: &str = "sandproof:authority:fixture-deployment";
const PASSWORD: &str = "component-fixture-not-a-production-credential";

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
        // Reserve the selected endpoint during filesystem preparation. A
        // released ephemeral port is not an ownership claim on a later Redis.
        drop(listener);
        fixture.start();
        fixture
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
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                // Synthetic fixture logs only, bounded before TempDir cleanup.
                // Keep the actual startup cause; never retry an unknown engine
                // or mistake a different process's open port for readiness.
                let mut diagnostic = String::new();
                fs::File::open(self.root.path().join("redis.log"))
                    .unwrap()
                    .take(4096)
                    .read_to_string(&mut diagnostic)
                    .unwrap();
                panic!("fixture Redis exited before ready ({status}): {diagnostic}");
            }
            if let Ok(stream) =
                TcpStream::connect_timeout(&self.address.into(), Duration::from_millis(30))
            {
                drop(stream);
                let Ok(mut connection) = try_connect(self.address) else {
                    assert!(
                        Instant::now() < deadline,
                        "fixture Redis authentication/readiness deadline"
                    );
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
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
