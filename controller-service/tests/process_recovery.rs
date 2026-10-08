mod support;
use support::*;

use sandbox_controller_core as core;
use sandproof_controller_service::{
    CrashInjector, CrashPoint, JournalError, NeverCrash, finish_accepted_kill_v1,
    finish_admitted_create_v1,
};
use tempfile::TempDir;

// Keep process spawning in a separate test executable: a fork/exec window can
// transiently inherit another thread's open file locks before close-on-exec.
// These tests must not share a process with unrelated live database leases.

struct ExitAt(CrashPoint);
impl CrashInjector for ExitAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            // Terminate the process without Rust destructors/SQLite rollback.
            // Unlike abort/SIGABRT, this does not leave a platform core dump.
            std::process::exit(91);
        }
        Ok(())
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "each named process-exit window must set up its exact durable and host-visible state"
)]
fn crash_child(stage: &str) {
    let path = std::env::var_os("SANDPROOF_REDUCER_CRASH_DB").unwrap();
    let mut journal = open_complete(&path, NAMESPACE).unwrap();
    if stage == "after_prepare_worker" {
        apply(&mut journal, create());
        assert_eq!(dispatch_all(&mut journal), 1); // install fence
        let plan = journal
            .world()
            .unwrap()
            .operation_plan(core::OperationKey {
                sandbox: SANDBOX,
                operation: CREATE_OP,
            })
            .unwrap()
            .clone();
        apply(
            &mut journal,
            core::ControllerInput::Scheduler(core::SchedulerInput::DispatchPrepare(
                SANDBOX,
                plan.vm.unwrap(),
                plan.slot.unwrap(),
            )),
        );
        journal
            .dispatch_next(
                &mut |action| {
                    assert!(matches!(
                        action.command.command(),
                        core::Command::PrepareVm { .. }
                    ));
                    std::fs::write(
                        std::path::Path::new(&path).with_extension("host-marker"),
                        b"vm started",
                    )
                    .unwrap();
                    worker(action)
                },
                &mut ExitAt(CrashPoint::AfterWorkerCall),
            )
            .unwrap();
        panic!("child missed its process-exit boundary");
    }
    if stage == "after_stop_worker" {
        let core::ControllerInput::Request(core::Request::Create(request)) = create() else {
            unreachable!()
        };
        admit_and_finish(&mut journal, request, &mut worker).unwrap();
        apply(&mut journal, kill(1));
        assert_eq!(dispatch_all(&mut journal), 1); // terminal fence
        let world = journal.world().unwrap();
        let record = world.sandbox_record(SANDBOX).unwrap();
        let plan = world
            .operation_plan(core::OperationKey {
                sandbox: SANDBOX,
                operation: core::OperationId(101),
            })
            .unwrap();
        let vm = plan.stop_vms[0];
        apply(
            &mut journal,
            core::ControllerInput::Scheduler(core::SchedulerInput::StopVm(
                SANDBOX,
                record.generation,
                vm,
            )),
        );
        journal
            .dispatch_next(
                &mut |action| {
                    assert!(matches!(
                        action.command.command(),
                        core::Command::StopVm { .. }
                    ));
                    std::fs::write(
                        std::path::Path::new(&path).with_extension("host-marker"),
                        b"vm stopped",
                    )
                    .unwrap();
                    worker(action)
                },
                &mut ExitAt(CrashPoint::AfterWorkerCall),
            )
            .unwrap();
        panic!("child missed its process-exit boundary");
    }
    let point = match stage {
        "before_commit" => CrashPoint::BeforeCommit,
        "after_commit" => CrashPoint::AfterCommit,
        "after_claim" => CrashPoint::AfterDispatchClaim,
        "after_worker" => CrashPoint::AfterWorkerCall,
        _ => panic!("unknown child scenario"),
    };
    if matches!(point, CrashPoint::BeforeCommit | CrashPoint::AfterCommit) {
        let core::ControllerInput::Request(core::Request::Create(request)) = create() else {
            unreachable!()
        };
        admit(&mut journal, request, &mut ExitAt(point)).unwrap();
    } else {
        apply(&mut journal, create());
        journal
            .dispatch_next(
                &mut |action| {
                    let marker = std::path::Path::new(&path).with_extension("host-marker");
                    std::fs::write(marker, b"fake host action completed").unwrap();
                    worker(action)
                },
                &mut ExitAt(point),
            )
            .unwrap();
    }
    panic!("child missed its process-exit boundary");
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one process-exit matrix asserts distinct commit, launch and stop recovery outcomes"
)]
fn process_exit_recovers_commits_and_never_repeats_an_uncertain_host_action() {
    if let Ok(stage) = std::env::var("SANDPROOF_REDUCER_CRASH_CHILD") {
        crash_child(&stage);
        return;
    }
    for stage in [
        "before_commit",
        "after_commit",
        "after_claim",
        "after_worker",
        "after_prepare_worker",
        "after_stop_worker",
    ] {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("controller.db");
        let initial = open_complete(&path, NAMESPACE).unwrap().world().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process_exit_recovers_commits_and_never_repeats_an_uncertain_host_action",
                "--nocapture",
            ])
            .env("SANDPROOF_REDUCER_CRASH_CHILD", stage)
            .env("SANDPROOF_REDUCER_CRASH_DB", &path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(91), "{stage}: {output:?}");
        let mut journal = open_complete(&path, NAMESPACE).unwrap();
        if stage == "before_commit" {
            assert_eq!(journal.world().unwrap(), initial);
            assert_eq!(dispatch_all(&mut journal), 0);
        } else if stage == "after_commit" {
            assert!(journal.world().unwrap().sandbox_record(SANDBOX).is_some());
            assert_eq!(dispatch_all(&mut journal), 1);
        } else if stage == "after_prepare_worker" {
            assert_eq!(dispatch_all(&mut journal), 0);
            assert!(path.with_extension("host-marker").exists());
            let pending = journal.uncertain_actions().unwrap();
            assert_eq!(pending.len(), 1);
            assert!(matches!(
                pending[0].command.command(),
                core::Command::PrepareVm { .. }
            ));
            assert_eq!(
                journal
                    .reconcile_uncertain(&mut |action| Some(success(action)))
                    .unwrap(),
                1
            );
            let mut repeated_prepare = false;
            let running =
                finish_admitted_create_v1(&mut journal, SANDBOX, CREATE_OP, &mut |action| {
                    if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                        repeated_prepare = true;
                    }
                    worker(action)
                })
                .unwrap();
            assert!(!repeated_prepare, "recovery must not launch the VM twice");
            assert_eq!(
                running.sandbox_record(SANDBOX).unwrap().phase,
                core::Phase::Running
            );
        } else if stage == "after_stop_worker" {
            assert_eq!(dispatch_all(&mut journal), 0);
            assert!(path.with_extension("host-marker").exists());
            let pending = journal.uncertain_actions().unwrap();
            assert_eq!(pending.len(), 1);
            assert!(matches!(
                pending[0].command.command(),
                core::Command::StopVm { .. }
            ));
            assert_eq!(
                journal
                    .reconcile_uncertain(&mut |action| Some(success(action)))
                    .unwrap(),
                1
            );
            let mut repeated_stop = false;
            let destroyed = finish_accepted_kill_v1(
                &mut journal,
                SANDBOX,
                core::OperationId(101),
                &mut |action| {
                    if matches!(action.command.command(), core::Command::StopVm { .. }) {
                        repeated_stop = true;
                    }
                    worker(action)
                },
            )
            .unwrap();
            assert!(!repeated_stop, "recovery must not stop another VM");
            assert_eq!(
                destroyed.sandbox_record(SANDBOX).unwrap().phase,
                core::Phase::Destroyed
            );
        } else {
            assert_eq!(dispatch_all(&mut journal), 0);
            let pending = journal.uncertain_actions().unwrap();
            assert_eq!(pending.len(), 1);
            assert_eq!(
                path.with_extension("host-marker").exists(),
                stage == "after_worker"
            );
            assert!(matches!(
                journal.apply(kill(1), &mut NeverCrash),
                Err(JournalError::UnresolvedAction)
            ));
            if stage == "after_worker" {
                journal
                    .complete(&success(&pending[0]), &mut NeverCrash)
                    .unwrap();
                assert_eq!(dispatch_all(&mut journal), 0);
            }
        }
    }
}
