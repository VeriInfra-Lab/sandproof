//! Finite failure metadata from the existing original wait, never a decision.
use super::{AuthorityError, PortError};
use std::{
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
};

const LIMIT: usize = 64;
static EMITTED: AtomicUsize = AtomicUsize::new(0);
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
}

fn record(stage: Stage, failure: Failure, count: &AtomicUsize, output: &mut impl Write) {
    let Ok(previous) = count.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        (value < LIMIT).then(|| value + 1)
    }) else {
        return;
    };
    // Both errors are closed enums without arbitrary text or request data.
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
            "center_wait unix_ms={unix_ms:?} pid={pid} scope={} flow={:?} step={:?} stage={stage:?} authority={error:?} remaining={}",
            context.sequence, context.flow, context.step, LIMIT - previous - 1
        ),
        Failure::Port(error) => writeln!(
            output,
            "center_wait unix_ms={unix_ms:?} pid={pid} scope={} flow={:?} step={:?} stage={stage:?} port={error:?} remaining={}",
            context.sequence, context.flow, context.step, LIMIT - previous - 1
        ),
    };
}
pub(crate) fn emit(stage: Stage, failure: Failure) {
    record(stage, failure, &EMITTED, &mut std::io::stderr().lock());
}
#[cfg(test)]
mod tests {
    use super::*;
    fn without_scope(line: &str) -> String {
        let (prefix, rest) = line.split_once(" scope=").unwrap();
        let (scope, suffix) = rest.split_once(" stage=").unwrap();
        assert!(scope.len() < 100);
        format!("{prefix} stage={suffix}")
    }
    #[test]
    fn failure_is_bound_to_the_synchronous_query_stage_without_identity() {
        use sandproof_center_store::{DiagnosticFlow, DiagnosticScope, DiagnosticStep};
        let _query = DiagnosticScope::enter(DiagnosticFlow::LifecycleQuery, DiagnosticStep::Lookup);
        let context = sandproof_center_store::diagnostic_context();
        let _expiry = DiagnosticScope::step(DiagnosticStep::Expiry);
        let mut bytes = vec![];
        record(Stage::LifecycleQueryExpiry, Failure::Port(PortError::OutcomeUnknown), &AtomicUsize::new(0), &mut bytes);
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains(&format!("scope={} flow=LifecycleQuery step=Expiry", context.sequence)));
        assert_eq!(line.lines().count(), 1);
        assert!(without_scope(line.trim_end()).len() < 220);
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
        assert!(text.lines().all(|line| without_scope(line).len() < 220));
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
                .all(|line| line.contains("authority=RecoveryRequired") && without_scope(line).len() < 160)
        );
        assert!(text.ends_with("remaining=0\n"));
        assert_eq!(count.load(Ordering::Relaxed), LIMIT);
    }
}
