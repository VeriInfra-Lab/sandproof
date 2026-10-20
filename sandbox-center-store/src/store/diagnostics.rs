//! Bounded failure-only metadata; never a storage decision or retry.
use std::{
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

const LIMIT: usize = 64;
static EMITTED: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug)]
pub(super) enum Phase {
    ServerHealth,
    SettingsHealth,
    PersistenceHealth,
    Read,
    Cas,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    BudgetExpired,
    MissingConnection,
    ReadTimeoutSetup,
    WriteTimeoutSetup,
    TransportTimeout,
    Transport {
        kind: std::io::ErrorKind,
        errno: Option<i32>,
    },
    Protocol(redis::ErrorKind),
    ConfigurationMismatch,
    UnhealthyPersistence,
}

pub(super) fn classify(error: &redis::RedisError) -> Failure {
    if error.is_timeout() {
        Failure::TransportTimeout
    } else if error.is_io_error() {
        let source = std::error::Error::source(error).and_then(|source| {
            source.downcast_ref::<std::io::Error>().or_else(|| {
                // The pinned Redis crate exposes its source through a public
                // Arc<dyn Error>; never format the arbitrary inner error.
                source
                    .downcast_ref::<std::sync::Arc<dyn std::error::Error + Send + Sync>>()
                    .and_then(|source| source.as_ref().downcast_ref::<std::io::Error>())
            })
        });
        Failure::Transport {
            kind: source.map_or(std::io::ErrorKind::Other, std::io::Error::kind),
            errno: source.and_then(std::io::Error::raw_os_error),
        }
    } else {
        Failure::Protocol(error.kind())
    }
}

fn record(
    phase: Phase,
    failure: Failure,
    elapsed: Duration,
    budget: Duration,
    count: &AtomicUsize,
    output: &mut impl Write,
) {
    let Ok(previous) = count.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        (value < LIMIT).then(|| value + 1)
    }) else {
        return;
    };
    // Consume failed writes too; fixed enums/numbers only, no Redis error text,
    // command arguments, payload, address, identity, credential or path.
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|value| value.as_millis());
    let pid = std::process::id();
    let _ = writeln!(
        output,
        "center_store_query unix_ms={unix_ms:?} pid={pid} phase={phase:?} failure={failure:?} elapsed_us={} remaining_at_start_us={} remaining={}",
        elapsed.as_micros(),
        budget.as_micros(),
        LIMIT - previous - 1
    );
}

pub(super) fn emit(phase: Phase, failure: Failure, elapsed: Duration, budget: Duration) {
    record(
        phase,
        failure,
        elapsed,
        budget,
        &EMITTED,
        &mut std::io::stderr().lock(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redis_failure_classification_discards_arbitrary_peer_text() {
        let secret = "credential-and-payload-must-never-appear";
        let cases = [
            (
                redis::RedisError::from(std::io::Error::new(std::io::ErrorKind::TimedOut, secret)),
                Failure::TransportTimeout,
            ),
            (
                redis::RedisError::from(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    secret,
                )),
                Failure::Transport {
                    kind: std::io::ErrorKind::ConnectionReset,
                    errno: None,
                },
            ),
            (
                redis::RedisError::from((
                    redis::ErrorKind::UnexpectedReturnType,
                    "private-detail",
                    secret.to_owned(),
                )),
                Failure::Protocol(redis::ErrorKind::UnexpectedReturnType),
            ),
        ];
        let mut bytes = vec![];
        let count = AtomicUsize::new(0);
        for (error, expected) in cases {
            assert_eq!(classify(&error), expected);
            record(
                Phase::Cas,
                classify(&error),
                Duration::ZERO,
                Duration::ZERO,
                &count,
                &mut bytes,
            );
        }
        let text = String::from_utf8(bytes).unwrap();
        assert!(!text.contains(secret) && !text.contains("private-detail"));
        assert_eq!(text.lines().count(), 3);
    }
    #[cfg(unix)]
    #[test]
    fn numeric_os_error_survives_the_public_redis_error_source_wrapper() {
        let error = redis::RedisError::from(std::io::Error::from_raw_os_error(13));
        assert_eq!(
            classify(&error),
            Failure::Transport {
                kind: std::io::ErrorKind::PermissionDenied,
                errno: Some(13),
            }
        );
    }

    #[test]
    fn failed_sink_consumes_the_finite_process_attempt_budget() {
        let count = AtomicUsize::new(0);
        let mut broken = std::io::Cursor::new([0_u8; 0]);
        record(
            Phase::Read,
            Failure::Protocol(redis::ErrorKind::UnexpectedReturnType),
            Duration::MAX,
            Duration::MAX,
            &count,
            &mut broken,
        );
        let mut bytes = vec![];
        for _ in 0..1000 {
            record(
                Phase::Cas,
                Failure::BudgetExpired,
                Duration::MAX,
                Duration::MAX,
                &count,
                &mut bytes,
            );
        }
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), LIMIT - 1);
        assert!(text.lines().all(|line| line.len() < 256));
        assert!(text.ends_with("remaining=0\n"));
        assert_eq!(count.load(Ordering::Relaxed), LIMIT);
    }
}
