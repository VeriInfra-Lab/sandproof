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
    Transport,
    Protocol,
}

pub(super) fn classify(error: &redis::RedisError) -> Failure {
    if error.is_timeout() {
        Failure::TransportTimeout
    } else if error.is_io_error() {
        Failure::Transport
    } else {
        Failure::Protocol
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
    let _ = writeln!(
        output,
        "center_store_query phase={phase:?} failure={failure:?} elapsed_us={} remaining_at_start_us={} remaining={}",
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
                Failure::Transport,
            ),
            (
                redis::RedisError::from((
                    redis::ErrorKind::UnexpectedReturnType,
                    "private-detail",
                    secret.to_owned(),
                )),
                Failure::Protocol,
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
    #[test]
    fn failed_sink_consumes_the_finite_process_attempt_budget() {
        let count = AtomicUsize::new(0);
        let mut broken = std::io::Cursor::new([0_u8; 0]);
        record(
            Phase::Read,
            Failure::Protocol,
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
