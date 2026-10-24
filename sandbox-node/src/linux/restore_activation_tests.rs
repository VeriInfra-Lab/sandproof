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
