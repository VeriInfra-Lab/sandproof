//! Before any client or lease exists, replace only a positively failed bind.
use super::{Engine, FixtureIdentityError, STARTUP};
use sandproof_center_store::REDIS_VERSION;
use std::{
    fmt,
    net::{SocketAddrV4, TcpListener},
    process::{Command, ExitStatus},
    time::{Duration, Instant},
};

pub(super) const READY_TIMEOUT: Duration = Duration::from_secs(5);
const INITIAL_ENDPOINT_ATTEMPTS: usize = 3;

#[derive(Debug)]
pub(super) enum StartupFailure {
    Exited {
        pid: u32,
        status: ExitStatus,
        diagnostic: String,
    },
    Identity {
        error: FixtureIdentityError,
        diagnostic: String,
    },
    Deadline(&'static str),
}

impl StartupFailure {
    fn confirmed_bind_conflict(&self, address: SocketAddrV4) -> bool {
        let Self::Exited {
            pid,
            status,
            diagnostic,
        } = self
        else {
            return false;
        };
        let process = format!("{pid}:");
        let failure = format!(
            "# Warning: Could not create server TCP listening socket {address}: bind: Address already in use"
        );
        status.code() == Some(1)
            && diagnostic
                .lines()
                .any(|line| line.starts_with(&process) && line.ends_with(&failure))
    }
}

impl fmt::Display for StartupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited {
                status, diagnostic, ..
            } => write!(
                formatter,
                "fixture Redis exited before ready ({status}): {diagnostic}"
            ),
            Self::Identity { error, diagnostic } => write!(
                formatter,
                "fixture Redis identity not verified ({error:?}): {diagnostic}"
            ),
            Self::Deadline(message) => formatter.write_str(message),
        }
    }
}

impl Engine {
    // A supplied prior deadline can only shorten the original five seconds.
    // The reservation callback lets tests retain the actual listener before
    // spawn; normal construction simply drops it at this same boundary.
    pub(super) fn initial(
        mut release: impl FnMut(&Self, TcpListener, usize),
        mut deadline: Option<Instant>,
    ) -> Result<Self, StartupFailure> {
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
        let _startup = STARTUP.lock().expect("fixture startup lock");
        for attempt in 0..INITIAL_ENDPOINT_ATTEMPTS {
            if deadline.is_some_and(|limit| Instant::now() >= limit) {
                return Err(StartupFailure::Deadline("fixture Redis readiness deadline"));
            }
            let (mut fixture, reservation) = Self::prepare(binary.clone());
            release(&fixture, reservation, attempt);
            if deadline.is_some_and(|limit| Instant::now() >= limit) {
                return Err(StartupFailure::Deadline("fixture Redis readiness deadline"));
            }
            fixture.spawn();
            let original_limit = Instant::now() + READY_TIMEOUT;
            let limit = deadline.map_or(original_limit, |prior| prior.min(original_limit));
            deadline = Some(limit);
            match fixture.wait_ready(limit) {
                Ok(()) => return Ok(fixture),
                Err(error)
                    if error.confirmed_bind_conflict(fixture.address)
                        && attempt + 1 < INITIAL_ENDPOINT_ATTEMPTS
                        && Instant::now() < limit =>
                {
                    // try_wait has positively observed this same child exit.
                    // Reap it before discarding only its private failed root.
                    fixture.child.take().unwrap().wait().unwrap();
                    eprintln!("initial fixture endpoint allocation {attempt} failed: {error}");
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("last initial allocation returns its original failure")
    }
}

#[cfg(test)]
mod tests {
    use super::super::{KEY, connect};
    use super::*;
    use std::{fs::OpenOptions, io::Write, panic::AssertUnwindSafe};

    #[test]
    fn occupied_initial_reservation_reallocates_before_exposure_and_verifies_child() {
        let mut held = None;
        let mut originals = Vec::new();
        let mut attempts = 0;
        let mut fixture = Engine::initial(
            |current, listener, attempt| {
                attempts += 1;
                originals.push((current.address, current.root.path().to_path_buf()));
                if attempt == 0 {
                    held = Some(listener);
                } else {
                    drop(listener);
                }
            },
            None,
        )
        .unwrap();
        assert!((2..=INITIAL_ENDPOINT_ATTEMPTS).contains(&attempts));
        assert_ne!(fixture.address, originals[0].0);
        assert!(!originals[0].1.exists());
        assert!(held.is_some());
        assert_eq!(
            fixture.verify_ready_identity(&mut connect(fixture.address)),
            Ok(())
        );
        fixture.field("allocation_witness", "ready");
        let value: String = redis::cmd("HGET")
            .arg(KEY)
            .arg("allocation_witness")
            .query(&mut connect(fixture.address))
            .unwrap();
        assert_eq!(value, "ready");
    }

    #[test]
    fn confirmed_bind_exhaustion_is_bounded_and_does_not_poison_allocator() {
        let mut held = Vec::new();
        let mut addresses = Vec::new();
        let result = Engine::initial(
            |current, listener, _| {
                addresses.push(current.address);
                held.push(listener);
            },
            None,
        );
        let Err(error) = result else {
            panic!("all original endpoints are still reserved")
        };
        assert_eq!(addresses.len(), INITIAL_ENDPOINT_ATTEMPTS);
        assert!(error.confirmed_bind_conflict(*addresses.last().unwrap()));
        drop(held);
        let mut healthy = Engine::new();
        assert_eq!(
            healthy.verify_ready_identity(&mut connect(healthy.address)),
            Ok(())
        );
    }

    #[test]
    fn unrelated_startup_exit_is_not_an_address_allocation_retry() {
        let mut attempts = 0;
        let result = Engine::initial(
            |current, listener, _| {
                attempts += 1;
                writeln!(
                    OpenOptions::new()
                        .append(true)
                        .open(current.root.path().join("redis.conf"))
                        .unwrap(),
                    "invalid-fixture-directive yes"
                )
                .unwrap();
                drop(listener);
            },
            None,
        );
        let Err(error) = result else {
            panic!("invalid configuration must fail")
        };
        assert_eq!(attempts, 1);
        assert!(matches!(error, StartupFailure::Exited { .. }));
        let mut healthy = Engine::new();
        assert_eq!(
            healthy.verify_ready_identity(&mut connect(healthy.address)),
            Ok(())
        );
    }

    #[test]
    fn spent_initial_deadline_allocates_no_endpoint_or_child() {
        let mut attempts = 0;
        let result = Engine::initial(|_, _, _| attempts += 1, Some(Instant::now()));
        assert!(matches!(result, Err(StartupFailure::Deadline(_))));
        assert_eq!(attempts, 0);
    }

    #[test]
    fn used_engine_restart_keeps_original_address_and_aof_after_conflict() {
        let mut fixture = Engine::new();
        fixture.field("restart_witness", "retained");
        let original = (fixture.address, fixture.root.path().to_path_buf());
        fixture.stop();
        let occupied = TcpListener::bind(original.0).unwrap();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| fixture.start()));
        assert!(result.is_err());
        assert_eq!(fixture.address, original.0);
        assert_eq!(fixture.root.path(), original.1);
        drop(occupied);
        fixture.stop();
        fixture.start();
        assert_eq!(fixture.address, original.0);
        let value: String = redis::cmd("HGET")
            .arg(KEY)
            .arg("restart_witness")
            .query(&mut connect(fixture.address))
            .unwrap();
        assert_eq!(value, "retained");
    }
}
