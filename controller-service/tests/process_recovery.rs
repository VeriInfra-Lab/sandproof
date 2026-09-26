mod support;
use support::*;

use sandbox_controller_core as core;
use sandproof_controller_service::{
    ControllerJournal, CrashInjector, CrashPoint, JournalError, NeverCrash,
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

fn crash_child(stage: &str) {
    let path = std::env::var_os("SANDPROOF_REDUCER_CRASH_DB").unwrap();
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    let point = match stage {
        "before_commit" => CrashPoint::BeforeCommit,
        "after_commit" => CrashPoint::AfterCommit,
        "after_claim" => CrashPoint::AfterDispatchClaim,
        "after_worker" => CrashPoint::AfterWorkerCall,
        _ => panic!("unknown child scenario"),
    };
    if matches!(point, CrashPoint::BeforeCommit | CrashPoint::AfterCommit) {
        journal.apply(create(), &mut ExitAt(point)).unwrap();
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
    ] {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("controller.db");
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
        let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
        if stage == "before_commit" {
            assert_eq!(journal.world().unwrap(), core::initial_world());
            assert_eq!(dispatch_all(&mut journal), 0);
        } else if stage == "after_commit" {
            assert!(journal.world().unwrap().sandbox_record(SANDBOX).is_some());
            assert_eq!(dispatch_all(&mut journal), 1);
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
