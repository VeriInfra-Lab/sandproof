use super::*;
use crate::{NeverCrash, controller::lifecycle_inbox::tests as fixture};
use sandproof_protocol::lifecycle::{
    AuthenticatedCenterContext, LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
};

fn context() -> AuthenticatedCenterContext {
    let create = fixture::create("alpha", "session-a");
    AuthenticatedCenterContext::from_verified_peer_registration(
        create.center,
        create.request.binding.node,
    )
}
fn terminal(journal: &mut ControllerJournal) -> LifecycleWindowClose {
    let create = fixture::create("alpha", "session-a");
    journal
        .record_lifecycle_intent(&create, &context(), &mut NeverCrash)
        .unwrap();
    let destroy = fixture::destroy(&create);
    journal
        .record_lifecycle_intent(&destroy, &context(), &mut NeverCrash)
        .unwrap();
    LifecycleWindowClose {
        terminal: TerminalLifecycleObservation {
            request: destroy,
            status: TerminalStatus::Cleaned,
        },
    }
}

#[test]
fn terminal_intent_does_not_invent_confirmation_and_projection_does_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let close = terminal(&mut journal);
    let expected = journal.lifecycle_inbox_configuration().unwrap().unwrap();
    let changes = journal.journal.connection.total_changes();
    let view = snapshot(
        &journal.journal.connection,
        &journal.world().unwrap(),
        &expected,
    )
    .unwrap();
    assert_eq!(view.operations.len(), 2);
    assert!(
        view.operations
            .iter()
            .all(|operation| operation.terminal && operation.confirmed_terminal.is_none())
    );
    let current = view
        .operations
        .iter()
        .find(|operation| operation.current)
        .unwrap();
    assert_eq!(current.binding, close.terminal.request.request.binding);
    assert_eq!(
        serde_json::to_value(current).unwrap()["confirmed_terminal"],
        serde_json::Value::Null
    );
    assert_eq!(journal.journal.connection.total_changes(), changes);
    assert!(journal.world().unwrap().sandbox_records().is_empty());
}

#[test]
fn only_current_operation_exports_the_original_exact_confirmation_then_recent_retired_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let close = terminal(&mut journal);
    journal
        .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
        .unwrap();
    let expected = journal.lifecycle_inbox_configuration().unwrap().unwrap();
    let changes = journal.journal.connection.total_changes();
    let view = snapshot(
        &journal.journal.connection,
        &journal.world().unwrap(),
        &expected,
    )
    .unwrap();
    for operation in &view.operations {
        if operation.current {
            assert_eq!(operation.confirmed_terminal.as_ref(), Some(&close.terminal));
            assert_eq!(operation.binding, close.terminal.request.request.binding);
            assert_eq!(
                operation.accepted_version,
                close.terminal.request.accepted_version
            );
        } else {
            assert!(operation.confirmed_terminal.is_none());
        }
    }
    let json = serde_json::to_string(&view).unwrap();
    assert!(
        !json.contains("options") && !json.contains("env_vars") && !json.contains("credential")
    );
    assert_eq!(journal.journal.connection.total_changes(), changes);
    journal
        .close_lifecycle_window(&close, &context(), &mut NeverCrash)
        .unwrap();
    let retired = snapshot(
        &journal.journal.connection,
        &journal.world().unwrap(),
        &expected,
    )
    .unwrap();
    assert!(retired.operations.is_empty());
    assert_eq!(
        retired.retention_lanes[0].last_retired.as_ref(),
        Some(&close.terminal)
    );
}

#[test]
fn corrupt_cached_confirmation_cannot_be_exported_as_a_cleaned_result() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let mut journal = fixture::initialize(&dir.path().join("db"));
    let close = terminal(&mut journal);
    journal
        .confirm_lifecycle_terminal(&close.terminal, &context(), &mut NeverCrash)
        .unwrap();
    let expected = journal.lifecycle_inbox_configuration().unwrap().unwrap();
    let world = journal.world().unwrap();
    let mut inbox = codec::load(&journal.journal.connection).unwrap().unwrap();
    inbox.sessions[0]
        .confirmed_terminal
        .as_mut()
        .unwrap()
        .request
        .accepted_version = 99;
    let bytes = serde_json::to_vec(&inbox).unwrap();
    journal
        .journal
        .connection
        .execute(
            "UPDATE controller_lifecycle_inbox SET state_json=?1,sha256=?2 WHERE id=1",
            rusqlite::params![bytes, Sha256::digest(&bytes).as_slice()],
        )
        .unwrap();
    assert!(matches!(
        snapshot(&journal.journal.connection, &world, &expected),
        Err(JournalError::CorruptStore)
    ));
}
