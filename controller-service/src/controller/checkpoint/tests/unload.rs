//! Original journal/reducer/outbox/receipt/recovery component checks. Host
//! observations are synthetic; this does not enable a physical unload route.
use super::*;
use crate::DispatchResult;

fn policy() -> MaintenanceConfig {
    maintenance_config()
}
fn open_unload(path: &std::path::Path) -> ControllerJournal {
    ControllerJournal::open_complete(path, NS, host_config(4), policy(), 1000).unwrap()
}
fn accepted_pause(journal: &mut ControllerJournal, id: u64) {
    // Exercise the original lower-level acceptance transaction while the
    // public route remains closed pending source/bundle admission work.
    // The private reserved path accepts the original Pause above. A second
    // Pause while already pausing is rejected by the original phase guard;
    // the public entry points remain UnsupportedInput as checked above.
    let namespace = journal.namespace.clone();
    let tx =
        WorldTransaction::begin(&mut journal.journal.connection, &journal.committed_world).unwrap();
    super::super::super::storage::apply(
        &tx,
        &namespace,
        core::ControllerInput::Request(core::Request::Pause(core::PauseRequest {
            actor: core::Actor::Tenant(core::TenantId(id)),
            sandbox: core::SandboxId(id),
            operation: core::OperationId(2),
        })),
    )
    .unwrap();
    tx.commit().unwrap();
}
fn reopen(journal: ControllerJournal, path: &std::path::Path) -> ControllerJournal {
    let expected = journal.world().unwrap();
    drop(journal);
    let journal = open_unload(path);
    assert_eq!(journal.world().unwrap(), expected);
    journal
}
fn dispatch_all(journal: &mut ControllerJournal, id: u64) {
    for _ in 0..16 {
        if matches!(
            journal
                .dispatch_next_for(core::SandboxId(id), &mut success, &mut NeverCrash)
                .unwrap(),
            DispatchResult::NoReadyAction
        ) {
            return;
        }
    }
    panic!("finite original outbox did not drain");
}
fn reserve_and_seal(journal: &mut ControllerJournal, id: u64) -> core::RestoreSource {
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::ReservePauseSnapshot(
                core::OperationKey {
                    sandbox: core::SandboxId(id),
                    operation: core::OperationId(2),
                },
            )),
            &mut NeverCrash,
        )
        .unwrap();
    dispatch_all(journal, id);
    let source = journal
        .world()
        .unwrap()
        .operation_plan(core::OperationKey {
            sandbox: core::SandboxId(id),
            operation: core::OperationId(2),
        })
        .unwrap()
        .target_snapshot
        .unwrap();
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::SealSnapshot(
                core::SandboxId(id),
                source.snapshot,
                source.manifest,
            )),
            &mut NeverCrash,
        )
        .unwrap();
    dispatch_all(journal, id);
    source
}

#[test]
fn original_sqlite_pause_history_checkpoint_prune_reopen_and_query_are_preserved() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open_unload(&path);
    create(&mut journal, 1);
    create(&mut journal, 2);
    let other = journal
        .world()
        .unwrap()
        .sandbox_record(core::SandboxId(2))
        .unwrap()
        .clone();
    accepted_pause(&mut journal, 1);
    let mut journal = reopen(journal, &path);
    let source = reserve_and_seal(&mut journal, 1);
    checkpoint_and_prune(&mut journal);
    let mut journal = reopen(journal, &path);
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::CommitSnapshot(
                core::SandboxId(1),
                source.snapshot,
            )),
            &mut NeverCrash,
        )
        .unwrap();
    dispatch_all(&mut journal, 1);
    journal
        .apply(
            core::ControllerInput::Publication(core::PublicationInput::Paused(
                core::SandboxId(1),
                source,
            )),
            &mut NeverCrash,
        )
        .unwrap();
    checkpoint_and_prune(&mut journal);
    let journal = reopen(journal, &path);
    let world = journal.world().unwrap();
    let row = world.sandbox_record(core::SandboxId(1)).unwrap();
    assert_eq!(row.phase, core::Phase::Paused);
    assert_eq!(row.durable_snapshot, Some(source));
    assert_eq!(world.sandbox_record(core::SandboxId(2)), Some(&other));
    journal
        .check(core::ControllerInput::Request(core::Request::Query(
            core::QueryRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                sandbox: core::SandboxId(1),
                operation: core::OperationId(2),
            },
        )))
        .unwrap();
    // The actual VM/worker drain and running-position release remain open.
    assert!(
        journal
            .host_slot_context(core::SandboxId(1))
            .unwrap()
            .is_some()
    );
    // Keep the paused session while unrelated owners use and retire their
    // original positions. Its old input prefix must not pin those histories.
    let mut journal = journal;
    for id in 3..=5 {
        create(&mut journal, id);
        destroy(&mut journal, id);
        release(&mut journal, id);
        let now = 3000 + id * 200;
        journal
            .retire_reclaimed(core::SandboxId(id), now, |_| Ok(()), &mut NeverCrash)
            .unwrap();
        journal.expire_retained(now + 100, &mut NeverCrash).unwrap();
        checkpoint_and_prune(&mut journal);
        assert_eq!(count(&journal, "controller_inputs"), 0);
        journal = reopen(journal, &path);
        let world = journal.world().unwrap();
        assert_eq!(
            world
                .sandbox_record(core::SandboxId(1))
                .unwrap()
                .durable_snapshot,
            Some(source)
        );
        assert_eq!(world.sandbox_record(core::SandboxId(2)), Some(&other));
    }
    drop(journal);
    let mut wrong = maintenance_config();
    wrong.retention_ms += 1;
    assert!(ControllerJournal::open_complete(&path, NS, host_config(4), wrong, 1000).is_err());
}

#[test]
fn registration_reply_loss_reopens_the_original_committed_or_uncommitted_outcome() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open_unload(&path);
        create(&mut journal, 1);
        accepted_pause(&mut journal, 1);
        let source = reserve_and_seal(&mut journal, 1);
        journal
            .apply(
                core::ControllerInput::Scheduler(core::SchedulerInput::CommitSnapshot(
                    core::SandboxId(1),
                    source.snapshot,
                )),
                &mut NeverCrash,
            )
            .unwrap();
        let action = journal
            .claim_next(Some(core::SandboxId(1)), &mut NeverCrash)
            .unwrap()
            .unwrap();
        assert!(matches!(
            action.command.command(),
            core::Command::CommitSnapshot { .. }
        ));
        let receipt = success(&action).unwrap();
        super::super::super::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.complete(&receipt, &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let journal = open_unload(&path);
        let world = journal.world().unwrap();
        let plan = world
            .operation_plan(core::OperationKey {
                sandbox: core::SandboxId(1),
                operation: core::OperationId(2),
            })
            .unwrap();
        assert_eq!(
            plan.capture.unwrap().commit(),
            if durable {
                core::capture::CaptureCommit::Registered
            } else {
                core::capture::CaptureCommit::Unknown
            }
        );
        assert_eq!(
            world
                .effect_ids()
                .iter()
                .any(|id| matches!(id.command(), core::Command::StopVm { .. })),
            durable
        );
        let vm = world
            .sandbox_record(core::SandboxId(1))
            .unwrap()
            .current_vm
            .unwrap();
        assert_eq!(world.vm_record(vm).unwrap().status, core::VmStatus::Frozen);
        assert!(
            journal
                .host_slot_context(core::SandboxId(1))
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn recovery_profile_does_not_open_unreserved_public_pause_or_resume() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open_unload(&path);
    create(&mut journal, 1);
    let before = journal.world().unwrap();
    assert!(matches!(
        journal.apply(
            core::ControllerInput::Request(core::Request::Pause(core::PauseRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                sandbox: core::SandboxId(1),
                operation: core::OperationId(2),
            })),
            &mut NeverCrash
        ),
        Err(JournalError::UnsupportedInput)
    ));
    assert_eq!(journal.world().unwrap(), before);
    assert!(matches!(
        journal.apply(
            core::ControllerInput::Request(core::Request::Resume(core::ResumeRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                sandbox: core::SandboxId(1),
                operation: core::OperationId(3),
            })),
            &mut NeverCrash
        ),
        Err(JournalError::UnsupportedInput)
    ));
    assert_eq!(journal.world().unwrap(), before);
    accepted_pause(&mut journal, 1);
    let before = journal.world().unwrap();
    let namespace = journal.namespace.clone();
    let tx =
        WorldTransaction::begin(&mut journal.journal.connection, &journal.committed_world).unwrap();
    assert!(matches!(
        super::super::super::storage::apply(
            &tx,
            &namespace,
            core::ControllerInput::Request(core::Request::Pause(core::PauseRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                sandbox: core::SandboxId(1),
                operation: core::OperationId(3),
            }))
        ),
        Err(JournalError::ControllerRejected(core::controller::ControllerReject::Request(
            core::request::Reject::WrongPhase
        )))
    ));
    drop(tx);
    assert_eq!(journal.world().unwrap(), before);
}

#[test]
fn old_config_bytes_stay_old_and_envelope_cannot_lie_about_its_core_version() {
    assert!(
        serde_json::to_value(maintenance_config())
            .unwrap()
            .get("checkpoint_profile")
            .is_none()
    );
    for (envelope_version, core_version) in [(1, 2), (2, 1), (3, 2)] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open_unload(&path);
        create(&mut journal, 1);
        checkpoint_and_prune(&mut journal);
        drop(journal);
        let conn = Connection::open(&path).unwrap();
        let bytes: Vec<u8> = conn
            .query_row(
                "SELECT payload FROM controller_checkpoint WHERE id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut root: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(root["version"], 2);
        assert_eq!(root["core"]["version"], 2);
        root["version"] = envelope_version.into();
        root["core"]["version"] = core_version.into();
        let bytes = serde_json::to_vec(&root).unwrap();
        conn.execute(
            "UPDATE controller_checkpoint SET payload=?1,sha256=?2 WHERE id=1",
            params![bytes, Sha256::digest(&bytes).as_slice()],
        )
        .unwrap();
        drop(conn);
        assert!(
            ControllerJournal::open_complete(&path, NS, host_config(4), policy(), 1000).is_err()
        );
    }
}
