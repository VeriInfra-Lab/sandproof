use super::*;
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact};
#[path = "../../../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use crate::{CrashPoint, NeverCrash};
use fixture::*;

fn config() -> InternalInboxConfig {
    InternalInboxConfig {
        node: node("host-a"),
        max_sandboxes: 2,
        operations_per_sandbox: 3,
        max_options_bytes: 128,
    }
}
fn open(path: &std::path::Path) -> ControllerJournal {
    super::super::tests_support::open(path, "0123456789abcdef").unwrap()
}
struct Stop(CrashPoint);
impl CrashInjector for Stop {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}
#[test]
fn exact_duplicate_survives_restart_and_conflicting_parameters_never_execute() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut j = open(&path);
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let r = request(1, 1, Intent::Create);
    let original = j.record_internal_intent(&r, &mut NeverCrash).unwrap();
    assert_eq!(original.fact, LifecycleFact::Recorded);
    assert!(j.world().unwrap().sandbox_records().is_empty());
    drop(j);
    let mut j = open(&path);
    assert_eq!(
        j.record_internal_intent(&r, &mut NeverCrash).unwrap(),
        original
    );
    let mut changed = r.clone();
    changed.create.timeout_ms += 1;
    assert!(matches!(
        j.record_internal_intent(&changed, &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert_eq!(j.internal_pending(&config().node).unwrap(), vec![r]);
}
#[test]
fn terminal_first_and_after_recording_create_fence_delayed_or_duplicate_create() {
    for first in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut j = open(&dir.path().join("db"));
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let create = request(1, 1, Intent::Create);
        let original = if first {
            None
        } else {
            Some(j.record_internal_intent(&create, &mut NeverCrash).unwrap())
        };
        let destroy = request(2, 2, Intent::Destroy);
        let result = j.record_internal_intent(&destroy, &mut NeverCrash).unwrap();
        let late = j.record_internal_intent(&create, &mut NeverCrash);
        if let Some(original) = original {
            assert_eq!(late.unwrap(), original);
        } else {
            assert!(matches!(late, Err(JournalError::Conflict)));
        }
        assert!(j.world().unwrap().sandbox_records().is_empty());
        assert_eq!(
            j.record_internal_intent(&destroy, &mut NeverCrash).unwrap(),
            result
        );
        assert_eq!(j.internal_pending(&config().node).unwrap(), vec![destroy]);
    }
}
#[test]
fn before_commit_rolls_back_after_commit_restart_finds_exact_request() {
    for point in [CrashPoint::BeforeCommit, CrashPoint::AfterCommit] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        let r = request(1, 1, Intent::Create);
        assert!(matches!(
            j.record_internal_intent(&r, &mut Stop(point)),
            Err(JournalError::InjectedCrash(_))
        ));
        drop(j);
        let mut j = open(&path);
        assert_eq!(
            j.internal_pending(&config().node).unwrap().len(),
            usize::from(point == CrashPoint::AfterCommit)
        );
        let result = j.record_internal_intent(&r, &mut NeverCrash).unwrap();
        assert_eq!(result.identity, r.identity);
        assert!(j.world().unwrap().sandbox_records().is_empty());
    }
}
#[test]
fn unknown_sql_commit_cannot_mint_a_new_operation_or_response() {
    for durable in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let mut j = open(&path);
        j.initialize_internal_inbox(config(), &mut NeverCrash)
            .unwrap();
        super::super::transaction::fail_next_commit(durable);
        let r = request(1, 1, Intent::Create);
        assert!(matches!(
            j.record_internal_intent(&r, &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        assert_eq!(
            j.internal_pending(&config().node).unwrap().len(),
            usize::from(durable)
        );
        assert_eq!(
            j.record_internal_intent(&r, &mut NeverCrash)
                .unwrap()
                .identity,
            r.identity
        );
    }
}
#[test]
fn ownership_node_window_and_noncanonical_options_fail_without_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = open(&dir.path().join("db"));
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    let r = request(1, 1, Intent::Create);
    j.record_internal_intent(&r, &mut NeverCrash).unwrap();
    for mutate in [0, 1, 2, 3] {
        let mut bad = r.clone();
        match mutate {
            0 => bad.identity.tenant_id = "other".try_into().unwrap(),
            1 => bad.identity.node.node_generation = 2.try_into().unwrap(),
            2 => bad.identity.request_window = 2.try_into().unwrap(),
            _ => bad.create.options.push(b' '),
        }
        assert!(matches!(
            j.record_internal_intent(&bad, &mut NeverCrash),
            Err(JournalError::Conflict)
        ));
    }
    assert_eq!(j.internal_pending(&config().node).unwrap(), vec![r]);
}
#[test]
fn normal_slots_full_leave_one_destroy_slot_and_future_sql_space_is_charged() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = open(&dir.path().join("db"));
    j.initialize_internal_inbox(config(), &mut NeverCrash)
        .unwrap();
    j.record_internal_intent(&request(1, 1, Intent::Create), &mut NeverCrash)
        .unwrap();
    j.record_internal_intent(
        &request(2, 2, Intent::Renew { end_at_ms: 400 }),
        &mut NeverCrash,
    )
    .unwrap();
    assert!(matches!(
        j.record_internal_intent(
            &request(3, 3, Intent::Renew { end_at_ms: 500 }),
            &mut NeverCrash
        ),
        Err(JournalError::MaintenanceCapacity)
    ));
    j.record_internal_intent(&request(3, 3, Intent::Destroy), &mut NeverCrash)
        .unwrap();
    let mut too_large = config();
    too_large.max_options_bytes = 65_536;
    let dir = tempfile::tempdir().unwrap();
    let mut j = open(&dir.path().join("db"));
    assert!(matches!(
        j.initialize_internal_inbox(too_large, &mut NeverCrash),
        Err(JournalError::MaintenanceCapacity)
    ));
}
