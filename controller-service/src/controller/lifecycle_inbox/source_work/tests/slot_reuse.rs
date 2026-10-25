//! Original SQLite transitions; host completion/release facts remain controlled.
use super::*;
use sandbox_controller_core::maintenance::slots::{HostSlotId, HostSlotPhase};

fn occupy_released_position(path: &Path) -> AcceptedLifecycleIntent {
    let app = v3::application(
        path,
        Arc::new(Mutex::new(State {
            epoch: TestEpoch::Matching,
            source: Some(SourceProcessIdentity {
                pid: 1234,
                start_time_ticks: 5678,
                boot_id: "fixture-boot".into(),
            }),
            ..State::default()
        })),
        false,
    );
    app.establish_lifecycle_host_epoch().unwrap();
    let mut other = v3::request();
    other.request.binding.session = "other-owner-session".try_into().unwrap();
    other.request.binding.key.owner = "2".try_into().unwrap();
    other.retention.slot = 2;
    app.record_lifecycle_intent(&other, &v3::context()).unwrap();
    app.advance_lifecycle_create_with_verification(
        &other,
        &v3::context(),
        Some(v3::fixture::template()),
        v3::fixture::compute(),
        None,
    )
    .unwrap();
    other
}

fn destroy_other(journal: &mut ControllerJournal, other: &AcceptedLifecycleIntent) {
    let mut destroy = other.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    let local = journal
        .record_lifecycle_intent(&destroy, &v3::context(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    finish_restored_component(journal, local.sandbox);
    let context = journal.host_slot_context(local.sandbox).unwrap().unwrap();
    assert_eq!(context.position, HostSlotId(0));
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
}

fn capture_second(journal: &mut ControllerJournal, first: &AcceptedLifecycleIntent) {
    let request = second_unload(first);
    journal
        .record_lifecycle_intent(&request, &v3::context(), &mut NeverCrash)
        .unwrap();
    let source = journal
        .prepare_lifecycle_source_work::<FullEvidence>(
            &request,
            &v3::context(),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    journal
        .save_source_observation::<FullEvidence>(
            &source.work,
            &v3::context(),
            observation(source.work.requirement()),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    let captured = journal
        .admit_lifecycle_full_capture::<FullEvidence>(
            &source.work,
            &v3::context(),
            &full_request(&source),
            10,
            &mut NeverCrash,
        )
        .unwrap();
    finish_full_component(journal, &captured);
}

fn assert_release_required(
    journal: &mut ControllerJournal,
    resume: &AcceptedLifecycleIntent,
    position: HostSlotPhase,
) {
    assert!(journal.host_slot_inventory().unwrap().iter().any(|(ctx, phase)| {
        ctx.position == HostSlotId(1) && *phase == position
    }));
    let before = journal.world().unwrap();
    assert!(matches!(
        journal.admit_lifecycle_resume(
            resume,
            &v3::context(),
            10,
            v3::limits(),
            v3::fixture::compute(),
            &mut NeverCrash,
        ),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(journal.world().unwrap(), before);
}

#[test]
fn two_owner_two_cycle_resume_keeps_its_released_position_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (journal, source) = unloaded_resume_source(&path);
    let sandbox = source.work.anchor.local.sandbox;
    assert_eq!(
        journal.host_slot_context(sandbox).unwrap().unwrap().position,
        HostSlotId(0)
    );
    drop(journal);
    let other = occupy_released_position(&path);
    let mut journal = v3::open(&path);
    bootstrap(&mut journal);
    let first = record_real_resume(&mut journal);
    admit_real_resume(&mut journal, &first);
    let moved = journal.host_slot_context(sandbox).unwrap().unwrap();
    assert_eq!(moved.position, HostSlotId(1), "occupied original slot cannot be reused");
    journal
        .register_data_credential(sandbox, 2, [99; 32], &mut NeverCrash)
        .unwrap();
    finish_restored_component(&mut journal, sandbox);
    destroy_other(&mut journal, &other);
    capture_second(&mut journal, &first);
    let mut second = second_unload(&first);
    second.accepted_version = 5;
    second.request.binding.expected = ExpectedSession::Version(4);
    second.request.binding.key.sequence = 5.try_into().unwrap();
    second.request.operation = LifecycleOperation::Resume {
        source_incarnation: 102.try_into().unwrap(),
        capture: 2.try_into().unwrap(),
        target_incarnation: 103.try_into().unwrap(),
        manifest_digest: ContentDigest::from_sha256([88; 32]),
    };
    journal
        .record_lifecycle_intent(&second, &v3::context(), &mut NeverCrash)
        .unwrap();
    assert_release_required(&mut journal, &second, HostSlotPhase::Occupied);
    assert!(
        journal
            .release_host_slot(
                moved,
                |_| Err(crate::WorkerTransportError::ReplyLost),
                &mut NeverCrash,
            )
            .is_err()
    );
    assert_release_required(&mut journal, &second, HostSlotPhase::Releasing);
    journal
        .release_host_slot(moved, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    drop(journal);
    let mut journal = v3::open(&path);
    bootstrap(&mut journal);
    admit_real_resume(&mut journal, &second);
    let rebound = journal.host_slot_context(sandbox).unwrap().unwrap();
    assert_eq!(rebound.position, moved.position, "retain one position identity per sandbox");
    assert_eq!(rebound.expected_previous, Some(moved.binding.vm));
    assert_eq!(rebound.binding.generation, 3);
    assert!(rebound.binding.vm > moved.binding.vm);
    assert_eq!(journal.host_slot_contexts().unwrap().len(), 2);
    drop(journal);
    let mut journal = v3::open(&path);
    assert_eq!(journal.host_slot_context(sandbox).unwrap(), Some(rebound));
    bootstrap(&mut journal);
    assert_eq!(admit_real_resume(&mut journal, &second).local.unwrap().sandbox, sandbox);
    journal
        .register_data_credential(sandbox, 3, [100; 32], &mut NeverCrash)
        .unwrap();
    finish_restored_component(&mut journal, sandbox);
    assert_eq!(journal.world().unwrap().sandbox_record(sandbox).unwrap().phase, core::Phase::Running);
}
