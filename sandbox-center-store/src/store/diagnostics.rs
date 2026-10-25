//! Bounded failure-only metadata; never a storage decision or retry.
use std::{
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

/// Fixed synchronous call provenance only. No request identity is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticFlow {
    Unscoped,
    LifecycleQuery,
    LifecycleUnload,
    Background,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticStep {
    Unscoped,
    Lookup,
    Expiry,
    Projection,
    Refresh,
    Reconcile,
    Retire,
    Recovery,
    Material,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticContext {
    pub sequence: u64,
    pub flow: DiagnosticFlow,
    pub step: DiagnosticStep,
}
/// Compact fixed codes: q=query, u=unload, b=background; l=lookup, e=expiry,
/// p=projection, f=refresh, c=reconcile, t=retire, r=recovery, m=material.
/// A dash is explicitly uncorrelated (including exhausted sequence numbers).
impl std::fmt::Display for DiagnosticContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.sequence == 0 || self.flow == DiagnosticFlow::Unscoped {
            return f.write_str("-");
        }
        let flow = match self.flow {
            DiagnosticFlow::LifecycleQuery => 'q',
            DiagnosticFlow::LifecycleUnload => 'u',
            DiagnosticFlow::Background => 'b',
            DiagnosticFlow::Unscoped => '-',
        };
        let step = match self.step {
            DiagnosticStep::Unscoped => '-',
            DiagnosticStep::Lookup => 'l',
            DiagnosticStep::Expiry => 'e',
            DiagnosticStep::Projection => 'p',
            DiagnosticStep::Refresh => 'f',
            DiagnosticStep::Reconcile => 'c',
            DiagnosticStep::Retire => 't',
            DiagnosticStep::Recovery => 'r',
            DiagnosticStep::Material => 'm',
        };
        write!(f, "{flow}{}/{step}", self.sequence)
    }
}
const UNSCOPED: DiagnosticContext = DiagnosticContext {
    sequence: 0,
    flow: DiagnosticFlow::Unscoped,
    step: DiagnosticStep::Unscoped,
};
std::thread_local! { static CONTEXT: std::cell::Cell<DiagnosticContext> = const { std::cell::Cell::new(UNSCOPED) }; }
static NEXT_SCOPE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn next_context(
    sequence: &std::sync::atomic::AtomicU64,
    flow: DiagnosticFlow,
    step: DiagnosticStep,
) -> DiagnosticContext {
    match sequence.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1)) {
        Ok(previous) => DiagnosticContext {
            sequence: previous + 1,
            flow,
            step,
        },
        Err(_) => UNSCOPED,
    }
}
/// Restores the caller even on error/unwind. Deliberately cannot cross threads
/// or an async suspension; original query/worker scopes are synchronous.
pub struct DiagnosticScope {
    previous: DiagnosticContext,
    thread_bound: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl DiagnosticScope {
    #[must_use]
    pub fn enter(flow: DiagnosticFlow, step: DiagnosticStep) -> Self {
        let context = next_context(&NEXT_SCOPE, flow, step);
        Self::replace(context)
    }
    #[must_use]
    pub fn step(step: DiagnosticStep) -> Self {
        Self::replace(DiagnosticContext {
            step,
            ..diagnostic_context()
        })
    }
    fn replace(context: DiagnosticContext) -> Self {
        Self {
            previous: CONTEXT.replace(context),
            thread_bound: std::marker::PhantomData,
        }
    }
}
impl Drop for DiagnosticScope {
    fn drop(&mut self) {
        CONTEXT.set(self.previous);
    }
}
#[must_use]
pub fn diagnostic_context() -> DiagnosticContext {
    CONTEXT.get()
}

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
    let context = diagnostic_context();
    let _ = writeln!(
        output,
        "center_store_query unix_ms={unix_ms:?} pid={pid} c={context} phase={phase:?} failure={failure:?} elapsed_us={} remaining_at_start_us={} remaining={}",
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

#[cfg(test)]
mod context_tests {
    use super::*;
    #[test]
    fn cas_timeout_keeps_the_same_scope_and_fixed_query_stage() {
        let _query = DiagnosticScope::enter(DiagnosticFlow::LifecycleQuery, DiagnosticStep::Lookup);
        let sequence = diagnostic_context().sequence;
        let _expiry = DiagnosticScope::step(DiagnosticStep::Expiry);
        let mut bytes = vec![];
        record(
            Phase::Cas,
            Failure::TransportTimeout,
            Duration::ZERO,
            Duration::ZERO,
            &AtomicUsize::new(0),
            &mut bytes,
        );
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains(&format!(
            "c=q{sequence}/e phase=Cas failure=TransportTimeout"
        )));
    }
    #[test]
    fn exhausted_sequence_is_explicitly_uncorrelated_and_never_reuses_last_id() {
        let sequence = std::sync::atomic::AtomicU64::new(u64::MAX - 1);
        let last = next_context(
            &sequence,
            DiagnosticFlow::LifecycleQuery,
            DiagnosticStep::Lookup,
        );
        assert_eq!(last.sequence, u64::MAX);
        assert_eq!(last.to_string(), "q18446744073709551615/l");
        assert_eq!(last.to_string().len(), 23);
        assert_eq!(
            next_context(
                &sequence,
                DiagnosticFlow::Background,
                DiagnosticStep::Refresh
            ),
            UNSCOPED
        );
        assert_eq!(
            next_context(
                &sequence,
                DiagnosticFlow::LifecycleQuery,
                DiagnosticStep::Lookup
            ),
            UNSCOPED
        );
        assert_eq!(sequence.load(Ordering::Relaxed), u64::MAX);
    }
    #[test]
    fn synchronous_scope_restores_nested_context_and_is_thread_local() {
        assert_eq!(diagnostic_context(), UNSCOPED);
        let outer = DiagnosticScope::enter(DiagnosticFlow::LifecycleQuery, DiagnosticStep::Lookup);
        let query = diagnostic_context();
        assert!(query.sequence > 0);
        {
            let _step = DiagnosticScope::step(DiagnosticStep::Expiry);
            assert_eq!(diagnostic_context().sequence, query.sequence);
            assert_eq!(diagnostic_context().step, DiagnosticStep::Expiry);
            assert_eq!(
                std::thread::spawn(diagnostic_context).join().unwrap(),
                UNSCOPED
            );
        }
        assert_eq!(diagnostic_context(), query);
        let _ = std::panic::catch_unwind(|| {
            let _nested =
                DiagnosticScope::enter(DiagnosticFlow::Background, DiagnosticStep::Recovery);
            assert_ne!(diagnostic_context().sequence, query.sequence);
            panic!("controlled unwind");
        });
        assert_eq!(diagnostic_context(), query);
        drop(outer);
        assert_eq!(diagnostic_context(), UNSCOPED);
    }
}
