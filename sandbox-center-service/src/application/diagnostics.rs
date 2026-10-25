//! Finite failure metadata from the existing original wait, never a decision.
use super::{AuthorityError, PortError};
use std::{
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
};

const LIMIT: usize = 64;
static EMITTED: AtomicUsize = AtomicUsize::new(0);
// A Create poll may exhaust its existing budget before Resume begins. Keep
// Resume's fixed budget separate without changing any storage/HTTP decision.
static RESUME_EMITTED: AtomicUsize = AtomicUsize::new(0);
static RESUME_PENDING: AtomicUsize = AtomicUsize::new(0);
static RESUME_NOT_RUNNING: AtomicUsize = AtomicUsize::new(0);
const LOADING_LIMIT: usize = 4;
#[derive(Clone, Copy, Debug)]
pub(crate) enum Stage {
    TemplateResolve,
    CreateAdmission,
    SendSnapshot,
    ApplyResponse,
    SessionProjection,
    ReplyDeadline,
    RetiredSnapshot,
    AuthorityStore,
    AuthoritySnapshotLock,
    AuthoritySnapshot,
    LifecycleUnloadResult,
    LifecycleUnloadReplayAbsent,
    CreateObservationTransport,
    CreateObservationDeadline,
    CreateObservationSuperseded,
    CreateObservationNotCurrent,
    CreateObservationNotRunning,
    CreateObservationExpired,
    CreateObservationPending,
    CreateObservationTimeRejected,
    ResumeObservationTransport,
    ResumeObservationDeadline,
    ResumeObservationSuperseded,
    ResumeObservationNotCurrent,
    ResumeObservationNotRunning,
    ResumeObservationExpired,
    ResumeObservationPending,
    ResumeObservationTimeRejected,
    LifecycleQueryLookup,
    LifecycleQueryExpiry,
    LifecycleQueryResult,
    BackgroundRefresh,
    BackgroundReconcile,
    BackgroundRetire,
    BackgroundRecovery,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Failure {
    Authority(AuthorityError),
    Port(PortError),
    Transport(sandproof_internal::TransportError),
    ObservationTime {
        before_ms: u64,
        after_ms: u64,
        observed_at_ms: u64,
        end_at_ms: u64,
        health_window_ms: u64,
        max_clock_skew_ms: u64,
    },
    ClockInterval {
        before_ms: u64,
        after_ms: u64,
    },
}

fn record(stage: Stage, failure: Failure, count: &AtomicUsize, output: &mut impl Write) {
    let Ok(previous) = count.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        (value < LIMIT).then(|| value + 1)
    }) else {
        return;
    };
    // All errors are closed enums without arbitrary text or request data.
    // Match their contents explicitly so no stored payload can be introduced.
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|value| value.as_millis());
    let pid = std::process::id();
    let context = sandproof_center_store::diagnostic_context();
    let _ = match failure {
        Failure::Authority(error) => writeln!(
            output,
            "center_wait unix_ms={unix_ms:?} pid={pid} c={context} stage={stage:?} authority={error:?} remaining={}",
            LIMIT - previous - 1
        ),
        Failure::Port(error) => writeln!(
            output,
            "center_wait unix_ms={unix_ms:?} pid={pid} c={context} stage={stage:?} port={error:?} remaining={}",
            LIMIT - previous - 1
        ),
        Failure::Transport(error) => writeln!(
            output,
            "center_wait unix_ms={unix_ms:?} pid={pid} c={context} stage={stage:?} transport={error:?} remaining={}",
            LIMIT - previous - 1
        ),
        Failure::ObservationTime {
            before_ms,
            after_ms,
            observed_at_ms,
            end_at_ms,
            health_window_ms,
            max_clock_skew_ms,
        } => writeln!(
            output,
            "center_wait unix_ms={unix_ms:?} pid={pid} c={context} stage={stage:?} before_ms={before_ms} after_ms={after_ms} observed_at_ms={observed_at_ms} end_at_ms={end_at_ms} health_window_ms={health_window_ms} max_clock_skew_ms={max_clock_skew_ms} remaining={}",
            LIMIT - previous - 1
        ),
        Failure::ClockInterval { before_ms, after_ms } => writeln!(
            output,
            "center_wait unix_ms={unix_ms:?} pid={pid} c={context} stage={stage:?} before_ms={before_ms} after_ms={after_ms} remaining={}",
            LIMIT - previous - 1
        ),
    };
}
fn resume_stage(stage: Stage) -> bool {
    matches!(
        stage,
        Stage::ResumeObservationTransport
            | Stage::ResumeObservationDeadline
            | Stage::ResumeObservationSuperseded
            | Stage::ResumeObservationNotCurrent
            | Stage::ResumeObservationNotRunning
            | Stage::ResumeObservationExpired
            | Stage::ResumeObservationPending
            | Stage::ResumeObservationTimeRejected
    )
}
fn counter(stage: Stage) -> &'static AtomicUsize {
    if resume_stage(stage) {
        &RESUME_EMITTED
    } else {
        &EMITTED
    }
}
fn reserve_loading(count: &AtomicUsize) -> bool {
    count
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            (value < LOADING_LIMIT).then(|| value + 1)
        })
        .is_ok()
}
pub(crate) fn emit(stage: Stage, failure: Failure) {
    // Normal loading must not consume every Resume diagnostic before the
    // candidate becomes Running. These attempts still use the SAME total 64.
    let permitted = match stage {
        Stage::ResumeObservationPending => reserve_loading(&RESUME_PENDING),
        Stage::ResumeObservationNotRunning => reserve_loading(&RESUME_NOT_RUNNING),
        _ => true,
    };
    if !permitted {
        return;
    }
    record(stage, failure, counter(stage), &mut std::io::stderr().lock());
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_loading_reserves_space_within_the_same_resume_total_budget() {
        let pending = AtomicUsize::new(0);
        let not_running = AtomicUsize::new(0);
        let total = AtomicUsize::new(0);
        let mut bytes = vec![];
        for _ in 0..1000 {
            for (stage, count) in [
                (Stage::ResumeObservationPending, &pending),
                (Stage::ResumeObservationNotRunning, &not_running),
            ] {
                if reserve_loading(count) {
                    record(stage, Failure::Port(PortError::Busy), &total, &mut bytes);
                }
            }
        }
        assert_eq!(pending.load(Ordering::Relaxed), LOADING_LIMIT);
        assert_eq!(not_running.load(Ordering::Relaxed), LOADING_LIMIT);
        assert_eq!(total.load(Ordering::Relaxed), 2 * LOADING_LIMIT);
        for _ in 0..1000 {
            record(
                Stage::ResumeObservationTransport,
                Failure::Transport(sandproof_internal::TransportError::Rejected),
                &total,
                &mut bytes,
            );
        }
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), LIMIT);
        assert_eq!(text.matches("stage=ResumeObservationPending ").count(), 4);
        assert_eq!(text.matches("stage=ResumeObservationNotRunning ").count(), 4);
        assert_eq!(text.matches("stage=ResumeObservationTransport ").count(), LIMIT - 8);
        assert_eq!(total.load(Ordering::Relaxed), LIMIT);
        assert!(text.ends_with("remaining=0\n"));
    }

    #[test]
    fn resume_diagnostics_keep_an_independent_finite_budget_after_create_polling() {
        for stage in [
            Stage::ResumeObservationTransport,
            Stage::ResumeObservationDeadline,
            Stage::ResumeObservationSuperseded,
            Stage::ResumeObservationNotCurrent,
            Stage::ResumeObservationNotRunning,
            Stage::ResumeObservationExpired,
            Stage::ResumeObservationPending,
            Stage::ResumeObservationTimeRejected,
        ] {
            assert!(std::ptr::eq(counter(stage), &RESUME_EMITTED));
            assert!(!std::ptr::eq(
                counter(stage),
                counter(Stage::CreateObservationPending)
            ));
        }
        assert!(std::ptr::eq(counter(Stage::AuthorityStore), &EMITTED));
    }

    #[test]
    fn resume_time_rejection_records_bounded_numbers_and_original_query_context() {
        use sandproof_center_store::{DiagnosticFlow, DiagnosticScope, DiagnosticStep};
        let _scope = DiagnosticScope::enter(
            DiagnosticFlow::LifecycleQuery,
            DiagnosticStep::Projection,
        );
        let context = sandproof_center_store::diagnostic_context();
        let count = AtomicUsize::new(0);
        let mut bytes = vec![];
        for _ in 0..1000 {
            record(
                Stage::ResumeObservationTimeRejected,
                Failure::ObservationTime {
                    before_ms: u64::MAX,
                    after_ms: u64::MAX,
                    observed_at_ms: u64::MAX,
                    end_at_ms: u64::MAX,
                    health_window_ms: u64::MAX,
                    max_clock_skew_ms: u64::MAX,
                },
                &count,
                &mut bytes,
            );
        }
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), LIMIT);
        assert!(text.lines().all(|line| line.len() < 512
            && line.contains(&format!("c=q{}/p", context.sequence))));
        for field in [
            "before_ms",
            "after_ms",
            "observed_at_ms",
            "end_at_ms",
            "health_window_ms",
            "max_clock_skew_ms",
        ] {
            assert!(text.contains(&format!("{field}={}", u64::MAX)));
        }
        assert!(
            !text.contains("credential") && !text.contains("session") && !text.contains("payload")
        );
        assert!(text.ends_with("remaining=0\n"));
    }

    #[test]
    fn resume_transport_and_clock_rejections_keep_closed_causes() {
        let count = AtomicUsize::new(0);
        let mut bytes = vec![];
        record(
            Stage::ResumeObservationTransport,
            Failure::Transport(sandproof_internal::TransportError::InvalidResponse),
            &count,
            &mut bytes,
        );
        record(
            Stage::ResumeObservationTimeRejected,
            Failure::ClockInterval {
                before_ms: 200,
                after_ms: 100,
            },
            &count,
            &mut bytes,
        );
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("stage=ResumeObservationTransport transport=InvalidResponse"));
        assert!(text.contains("before_ms=200 after_ms=100"));
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn unload_transport_failure_preserves_closed_cause_and_request_scope() {
        use sandproof_center_store::{DiagnosticFlow, DiagnosticScope, DiagnosticStep};
        let _scope =
            DiagnosticScope::enter(DiagnosticFlow::LifecycleUnload, DiagnosticStep::Lookup);
        let sequence = sandproof_center_store::diagnostic_context().sequence;
        let count = AtomicUsize::new(0);
        let mut bytes = vec![];
        for error in [
            sandproof_internal::TransportError::Overloaded,
            sandproof_internal::TransportError::InvalidResponse,
        ] {
            record(
                Stage::CreateObservationTransport,
                Failure::Transport(error),
                &count,
                &mut bytes,
            );
        }
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(
            text.lines()
                .all(|line| line.contains(&format!("c=u{sequence}/l")) && line.len() < 220)
        );
        assert!(text.contains("transport=Overloaded"));
        assert!(text.contains("transport=InvalidResponse"));
    }
    #[test]
    fn failure_is_bound_to_the_synchronous_query_stage_without_identity() {
        use sandproof_center_store::{DiagnosticFlow, DiagnosticScope, DiagnosticStep};
        let _query = DiagnosticScope::enter(DiagnosticFlow::LifecycleQuery, DiagnosticStep::Lookup);
        let context = sandproof_center_store::diagnostic_context();
        let _expiry = DiagnosticScope::step(DiagnosticStep::Expiry);
        let mut bytes = vec![];
        record(
            Stage::LifecycleQueryExpiry,
            Failure::Port(PortError::OutcomeUnknown),
            &AtomicUsize::new(0),
            &mut bytes,
        );
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains(&format!("c=q{}/e", context.sequence)));
        assert_eq!(line.lines().count(), 1);
        assert!(line.trim_end().len() < 220);
    }
    #[test]
    fn lifecycle_query_causes_use_only_closed_errors_and_the_same_finite_budget() {
        use sandproof_center_store::StoreError;
        let count = AtomicUsize::new(0);
        let mut bytes = vec![];
        let causes = [
            (
                Stage::AuthorityStore,
                Failure::Authority(AuthorityError::Store(StoreError::InvalidState)),
            ),
            (
                Stage::AuthoritySnapshotLock,
                Failure::Authority(AuthorityError::RecoveryRequired),
            ),
            (
                Stage::AuthoritySnapshot,
                Failure::Authority(AuthorityError::Store(StoreError::Configuration)),
            ),
            (
                Stage::LifecycleQueryLookup,
                Failure::Port(PortError::OutcomeUnknown),
            ),
            (
                Stage::LifecycleQueryExpiry,
                Failure::Authority(AuthorityError::Store(StoreError::Unavailable)),
            ),
            (
                Stage::LifecycleQueryResult,
                Failure::Port(PortError::OutcomeUnknown),
            ),
        ];
        for (stage, failure) in causes.into_iter().cycle().take(1000) {
            record(stage, failure, &count, &mut bytes);
        }
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), LIMIT);
        assert!(text.contains("stage=AuthorityStore authority=Store(InvalidState)"));
        assert_eq!(count.load(Ordering::Relaxed), LIMIT);
        assert!(text.contains("stage=AuthoritySnapshot authority=Store(Configuration)"));
        assert!(text.contains("stage=LifecycleQueryExpiry authority=Store(Unavailable)"));
        assert!(text.lines().all(|line| line.len() < 220));
        assert!(!text.contains("payload") && !text.contains("credential") && !text.contains('/'));
    }

    #[test]
    fn authority_cause_is_preserved_without_retrying_failed_output() {
        let count = AtomicUsize::new(0);
        let mut broken = std::io::Cursor::new([0_u8; 0]);
        record(
            Stage::SendSnapshot,
            Failure::Port(PortError::OutcomeUnknown),
            &count,
            &mut broken,
        );
        let mut bytes = vec![];
        for _ in 0..1000 {
            record(
                Stage::ApplyResponse,
                Failure::Authority(AuthorityError::RecoveryRequired),
                &count,
                &mut bytes,
            );
        }
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), LIMIT - 1);
        assert!(
            text.lines()
                .all(|line| line.contains("authority=RecoveryRequired") && line.len() < 160)
        );
        assert!(text.ends_with("remaining=0\n"));
        assert_eq!(count.load(Ordering::Relaxed), LIMIT);
    }
}
