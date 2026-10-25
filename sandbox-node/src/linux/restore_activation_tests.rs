//! Deterministic component faults; not Firecracker/guest E2E evidence.
use super::*;
use sandbox_runtime::RuntimeProcessIdentity;
use std::cell::Cell;

fn candidate(state: LoadedSnapshotState) -> LoadedSnapshotObservation {
    LoadedSnapshotObservation {
        process: RuntimeProcessIdentity {
            pid: 17,
            start_time_ticks: 23,
            boot_id: "00000000-0000-0000-0000-000000000001".into(),
        },
        state,
    }
}

#[test]
fn restore_exit_requires_two_confirmed_observations_of_the_same_process() {
    let original = candidate(LoadedSnapshotState::Exited);
    let calls = Cell::new(0);
    let observed = observe_confirmed_restore_exit(|| {
        calls.set(calls.get() + 1);
        Ok(original.clone())
    })
    .unwrap();
    assert_eq!(observed, Some(original.process));
    assert_eq!(calls.get(), 2);
}

#[test]
fn live_restore_candidate_never_produces_an_exit_fact() {
    for state in [LoadedSnapshotState::Paused, LoadedSnapshotState::Running] {
        let calls = Cell::new(0);
        assert!(
            observe_confirmed_restore_exit(|| {
                calls.set(calls.get() + 1);
                assert_eq!(calls.get(), 1, "a live first observation is not an exit");
                Ok(candidate(state))
            })
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn missing_or_timed_out_restore_observations_remain_errors() {
    use std::io::{Error, ErrorKind};
    for kind in [
        ErrorKind::NotFound,
        ErrorKind::TimedOut,
        ErrorKind::PermissionDenied,
    ] {
        for fail_at in [1, 2] {
            let calls = Cell::new(0);
            let result = observe_confirmed_restore_exit(|| {
                calls.set(calls.get() + 1);
                if calls.get() == fail_at {
                    Err(Error::new(kind, "original restore observation unavailable").into())
                } else {
                    Ok(candidate(LoadedSnapshotState::Exited))
                }
            });
            assert!(result.is_err(), "observation failure is never an exit fact");
        }
    }
}

#[test]
fn changed_pid_start_boot_or_state_never_adopts_a_restore_exit() {
    let original = candidate(LoadedSnapshotState::Exited);
    let mut changes = Vec::new();
    let mut changed = original.clone();
    changed.process.pid += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.process.start_time_ticks += 1;
    changes.push(changed);
    let mut changed = original.clone();
    changed.process.boot_id = "00000000-0000-0000-0000-000000000002".into();
    changes.push(changed);
    changes.push(candidate(LoadedSnapshotState::Paused));
    changes.push(candidate(LoadedSnapshotState::Running));
    for changed in changes {
        let calls = Cell::new(0);
        assert!(
            observe_confirmed_restore_exit(|| {
                calls.set(calls.get() + 1);
                Ok(if calls.get() == 1 {
                    original.clone()
                } else {
                    changed.clone()
                })
            })
            .is_err()
        );
    }
}

#[test]
fn invalid_restore_process_fields_never_produce_an_exit_fact() {
    for field in 0..3 {
        let mut original = candidate(LoadedSnapshotState::Exited);
        match field {
            0 => original.process.pid = 0,
            1 => original.process.start_time_ticks = 0,
            _ => original.process.boot_id.clear(),
        }
        let calls = Cell::new(0);
        assert!(
            observe_confirmed_restore_exit(|| {
                calls.set(calls.get() + 1);
                assert_eq!(
                    calls.get(),
                    1,
                    "invalid identity is rejected before confirmation"
                );
                Ok(original.clone())
            })
            .is_err()
        );
    }
}

#[test]
fn interrupted_after_resume_finishes_current_authorization_on_same_pid() {
    let state = Cell::new(LoadedSnapshotState::Paused);
    let current_token = Cell::new("old-token");
    let initializations = Cell::new(0);
    let original = candidate(state.get());
    let first = finish_permitted_activation(
        &original,
        || {
            state.set(LoadedSnapshotState::Running);
            Err("interrupted after physical activation before init".into())
        },
        || {
            initializations.set(initializations.get() + 1);
            Ok(())
        },
        || panic!("no successful receipt on an interrupted activation"),
    );
    assert!(first.is_err());
    assert_eq!(initializations.get(), 0);
    assert_eq!(current_token.get(), "old-token");
    let observed = candidate(state.get());
    let recovered = finish_permitted_activation(
        &observed,
        || Ok(candidate(state.get())),
        || {
            current_token.set("new-token");
            initializations.set(initializations.get() + 1);
            Ok(())
        },
        || {
            if current_token.get() != "new-token" {
                return Err("current token rejected".into());
            }
            Ok(candidate(state.get()))
        },
    )
    .unwrap();
    assert_eq!(recovered.process, original.process);
    assert_eq!(recovered.state, LoadedSnapshotState::Running);
    assert_eq!(initializations.get(), 1);
    assert_eq!(current_token.get(), "new-token");
}

#[test]
fn replaced_candidate_never_receives_original_initialization() {
    let before = candidate(LoadedSnapshotState::Running);
    let result = finish_permitted_activation(
        &before,
        || {
            let mut foreign = before.clone();
            foreign.process.start_time_ticks += 1;
            Ok(foreign)
        },
        || panic!("cannot initialize a replacement"),
        || panic!("cannot publish a replacement"),
    );
    assert!(result.is_err());
}

#[test]
fn failed_or_unobserved_authorization_keeps_activation_unknown() {
    let before = candidate(LoadedSnapshotState::Running);
    assert!(
        finish_permitted_activation(
            &before,
            || Ok(before.clone()),
            || Err("init response lost".into()),
            || panic!("lost init is not ready")
        )
        .is_err()
    );
    assert!(
        finish_permitted_activation(
            &before,
            || Ok(before.clone()),
            || Ok(()),
            || Err("current credential still rejected".into())
        )
        .is_err()
    );
}

#[test]
fn restored_file_policy_observes_the_same_activated_candidate() {
    let before = candidate(LoadedSnapshotState::Paused);
    let activated = finish_permitted_activation(
        &before,
        || Ok(candidate(LoadedSnapshotState::Running)),
        || Ok(()),
        || Ok(candidate(LoadedSnapshotState::Running)),
    )
    .unwrap();
    assert_eq!(activated.process, before.process);
    assert!(matches!(
        observe_running_file_policy(
            &activated,
            || Ok(ObservedResult::FilePolicy { disk_inode: 41 }),
            || Ok(activated.clone()),
        )
        .unwrap(),
        ObservedResult::FilePolicy { disk_inode: 41 }
    ));
}

#[test]
fn restored_file_policy_rejects_unactivated_exited_or_changed_candidates() {
    for state in [LoadedSnapshotState::Paused, LoadedSnapshotState::Exited] {
        assert!(
            observe_running_file_policy(
                &candidate(state),
                || panic!("an unactivated candidate must not reach the disk observer"),
                || panic!("an unactivated candidate must not be confirmed"),
            )
            .is_err()
        );
    }
    let before = candidate(LoadedSnapshotState::Running);
    let mut changed = before.clone();
    changed.process.start_time_ticks += 1;
    for after in [changed, candidate(LoadedSnapshotState::Paused)] {
        assert!(
            observe_running_file_policy(
                &before,
                || Ok(ObservedResult::FilePolicy { disk_inode: 41 }),
                || Ok(after),
            )
            .is_err()
        );
    }
}

#[test]
fn restored_file_policy_preserves_disk_and_observation_failures() {
    let before = candidate(LoadedSnapshotState::Running);
    assert!(
        observe_running_file_policy(
            &before,
            || Err("original private disk policy rejected".into()),
            || panic!("a rejected disk must not produce a confirmation"),
        )
        .is_err()
    );
    assert!(
        observe_running_file_policy(
            &before,
            || Ok(ObservedResult::FilePolicy { disk_inode: 41 }),
            || Err("original candidate observation lost".into()),
        )
        .is_err()
    );
}

#[test]
fn restore_execution_requires_paused_but_readonly_probe_can_observe_after_activation() {
    let paused = candidate(LoadedSnapshotState::Paused);
    assert_eq!(
        require_loaded_candidate(paused.clone(), false).unwrap(),
        paused
    );
    assert_eq!(
        require_loaded_candidate(paused.clone(), true).unwrap(),
        paused
    );
    let activated = finish_permitted_activation(
        &paused,
        || Ok(candidate(LoadedSnapshotState::Running)),
        || Ok(()),
        || Ok(candidate(LoadedSnapshotState::Running)),
    )
    .unwrap();
    assert_eq!(activated.process, paused.process);
    assert_eq!(
        require_loaded_candidate(activated.clone(), true).unwrap(),
        activated
    );
    assert!(require_loaded_candidate(activated, false).is_err());
    for observe_only in [false, true] {
        assert!(
            require_loaded_candidate(candidate(LoadedSnapshotState::Exited), observe_only).is_err()
        );
    }
}
