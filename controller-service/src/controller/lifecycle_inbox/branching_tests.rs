//! `SQLite` plus the original native Running admission, with protected guest
//! readiness/cleanup observations supplied by each test.
use super::{ControllerJournal, tests as fixture};
use crate::{
    HostBranchBatchKey, HostBranchBatchPhase, HostBranchClose, HostBranchLimits,
    InternalLocalBinding, JournalError, NeverCrash,
};
use sandproof_protocol::{
    DataKind, SandboxId,
    branching::{BranchAccessIdentity, BranchAccessSelector, BranchBatchIdentity, BranchTarget},
    lifecycle::{
        AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleAccessIdentity,
        LifecycleOperation, LifecycleWindowClose, TerminalLifecycleObservation, TerminalStatus,
    },
};
use std::path::Path;

const NOW: i64 = 25;
const LIMITS: HostBranchLimits = HostBranchLimits {
    max_batches: 3,
    max_branches_per_vm: 2,
};

fn running(
    path: &Path,
) -> (
    ControllerJournal,
    LifecycleAccessIdentity,
    AuthenticatedCenterContext,
) {
    let mut journal = fixture::initialize(path);
    let request = fixture::create("alpha", "session-a");
    let context = AuthenticatedCenterContext::from_verified_peer_registration(
        request.center.clone(),
        request.request.binding.node.clone(),
    );
    let (parent, _) = admit_running(&mut journal, &request, &context);
    (journal, parent, context)
}

fn admit_running(
    journal: &mut ControllerJournal,
    request: &AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
) -> (LifecycleAccessIdentity, InternalLocalBinding) {
    journal
        .record_lifecycle_intent(request, context, &mut NeverCrash)
        .unwrap();
    let local = journal
        .admit_lifecycle_create(request, context, fixture::admission(), &mut NeverCrash)
        .unwrap()
        .local
        .unwrap();
    crate::finish_admitted_create_timed(
        journal,
        local.sandbox,
        local.operation,
        &mut crate::controller::tests_support::worker,
        &mut || Ok(20),
    )
    .unwrap();
    let LifecycleOperation::Create {
        target_incarnation, ..
    } = request.request.operation
    else {
        unreachable!();
    };
    let parent = LifecycleAccessIdentity {
        center: request.center.clone(),
        node: request.request.binding.node.clone(),
        session: request.request.binding.session.clone(),
        operation: request.request.binding.key.clone(),
        accepted_version: request.accepted_version,
        incarnation: target_incarnation,
        protocol_version: request.request.binding.protocol_version,
    };
    (parent, local)
}

fn key(parent: &LifecycleAccessIdentity, batch: &str, sequence: u64) -> HostBranchBatchKey {
    let mut operation = parent.operation.clone();
    operation.sequence = sequence.try_into().unwrap();
    HostBranchBatchKey {
        operation,
        batch: BranchBatchIdentity {
            parent: parent.session.clone(),
            incarnation: parent.incarnation,
            checkpoint: "checkpoint".try_into().unwrap(),
            batch: batch.try_into().unwrap(),
        },
    }
}

fn access(
    parent: &LifecycleAccessIdentity,
    key: &HostBranchBatchKey,
    sandbox: &SandboxId,
) -> BranchAccessIdentity {
    BranchAccessIdentity {
        parent: parent.clone(),
        selector: BranchAccessSelector {
            batch: key.batch.clone(),
            operation: key.operation.clone(),
            branch: sandbox.clone(),
        },
    }
}

fn target(id: &str, port: u16) -> BranchTarget {
    BranchTarget {
        sandbox: id.try_into().unwrap(),
        port: port.try_into().unwrap(),
    }
}

#[test]
fn host_branch_reservation_replays_exact_private_plan_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (mut journal, parent, context) = running(&path);
    let key = key(&parent, "batch-a", 10);
    let planned = [target("a", 31_000), target("b", 31_001)];
    let first = access(&parent, &key, &planned[0].sandbox);
    assert!(matches!(
        journal.reserve_host_branch_batch(&parent, &context, &key, &planned, NOW),
        Err(JournalError::UnsupportedStore)
    ));
    journal.configure_host_branching(LIMITS).unwrap();
    assert_eq!(
        journal
            .reserve_host_branch_batch(&parent, &context, &key, &planned, NOW)
            .unwrap(),
        HostBranchBatchPhase::Pending
    );
    assert!(
        journal
            .host_branch_data_route(&first, &context, DataKind::Exec, NOW)
            .is_err()
    );
    let original = journal
        .pending_host_branch_guest_plan(&parent, &context, &key, NOW)
        .unwrap();
    assert_ne!(original[0].token(), original[1].token());
    drop(journal);

    let mut journal = fixture::open(&path);
    assert!(
        journal
            .pending_host_branch_guest_plan(&parent, &context, &key, NOW)
            .is_err()
    );
    fixture::refresh_epoch(&mut journal);
    journal.configure_host_branching(LIMITS).unwrap();
    let duplicate = journal
        .pending_host_branch_guest_plan(&parent, &context, &key, NOW)
        .unwrap();
    for (left, right) in original.iter().zip(&duplicate) {
        assert_eq!(left.target(), right.target());
        assert_eq!(left.token(), right.token());
    }
    assert!(
        journal
            .reserve_host_branch_batch(&parent, &context, &key, &planned[..1], NOW)
            .is_err()
    );
    assert!(
        journal
            .publish_host_branch_batch(&parent, &context, &key, &planned[..1], NOW)
            .is_err()
    );
    assert!(
        journal
            .publish_host_branch_batch(
                &parent,
                &context,
                &key,
                &[planned[1].clone(), planned[0].clone()],
                NOW
            )
            .is_err()
    );
    journal
        .publish_host_branch_batch(&parent, &context, &key, &planned, NOW)
        .unwrap();
    let ready = journal
        .host_branch_data_route(&first, &context, DataKind::Exec, NOW)
        .unwrap();
    assert_eq!(ready.port().get(), 31_000);
    assert_eq!(ready.token(), original[0].token());
    assert!(
        journal
            .host_branch_data_route(&first, &context, DataKind::FileRead, 60_021)
            .is_err()
    );
    let mut foreign = first.clone();
    foreign.selector.operation.owner = "bravo".try_into().unwrap();
    assert!(
        journal
            .host_branch_data_route(&foreign, &context, DataKind::FileRead, NOW)
            .is_err()
    );
    let mut stale = first.clone();
    stale.parent.accepted_version += 1;
    assert!(
        journal
            .host_branch_data_route(&stale, &context, DataKind::FileRead, NOW)
            .is_err()
    );
}

#[test]
fn fenced_branch_cannot_route_or_reuse_its_port_before_exact_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let (mut journal, parent, context) = running(&directory.path().join("db"));
    journal.configure_host_branching(LIMITS).unwrap();
    let original = key(&parent, "batch-a", 10);
    let successor = key(&parent, "batch-b", 11);
    let planned = [target("a", 31_000), target("b", 31_001)];
    let new = [target("c", 31_000)];
    journal
        .reserve_host_branch_batch(&parent, &context, &original, &planned, NOW)
        .unwrap();
    journal
        .publish_host_branch_batch(&parent, &context, &original, &planned, NOW)
        .unwrap();
    let branch_a = access(&parent, &original, &planned[0].sandbox);
    let branch_b = access(&parent, &original, &planned[1].sandbox);
    let old = journal
        .host_branch_data_route(&branch_a, &context, DataKind::Exec, NOW)
        .unwrap();
    let close_a = Some(&planned[0].sandbox);
    assert!(
        journal
            .change_host_branch_close(
                &parent,
                &context,
                &original,
                NOW,
                HostBranchClose::ConfirmGone(close_a)
            )
            .is_err()
    );
    journal
        .change_host_branch_close(
            &parent,
            &context,
            &original,
            NOW,
            HostBranchClose::Begin(close_a),
        )
        .unwrap();
    assert!(
        journal
            .host_branch_data_route(&branch_a, &context, DataKind::Exec, NOW)
            .is_err()
    );
    assert!(
        journal
            .host_branch_data_route(&branch_b, &context, DataKind::FileRead, NOW)
            .is_ok()
    );
    assert!(
        journal
            .reserve_host_branch_batch(&parent, &context, &successor, &new, NOW)
            .is_err()
    );
    journal
        .change_host_branch_close(
            &parent,
            &context,
            &original,
            NOW,
            HostBranchClose::ConfirmGone(close_a),
        )
        .unwrap();
    journal
        .reserve_host_branch_batch(&parent, &context, &successor, &new, NOW)
        .unwrap();
    journal
        .publish_host_branch_batch(&parent, &context, &successor, &new, NOW)
        .unwrap();
    let branch_c = access(&parent, &successor, &new[0].sandbox);
    let ready = journal
        .host_branch_data_route(&branch_c, &context, DataKind::FileWrite, NOW)
        .unwrap();
    assert_eq!(ready.port(), old.port());
    assert_ne!(ready.token(), old.token());
    assert!(
        journal
            .host_branch_data_route(&branch_a, &context, DataKind::Exec, NOW)
            .is_err()
    );
}

#[test]
fn pending_and_closing_guest_work_both_block_native_parent_unload() {
    use sandproof_protocol::lifecycle::ExpectedSession;

    let directory = tempfile::tempdir().unwrap();
    let (mut journal, parent, context) = running(&directory.path().join("db"));
    journal.configure_host_branching(LIMITS).unwrap();
    let original = key(&parent, "batch-a", 10);
    let planned = [target("a", 31_000)];
    journal
        .reserve_host_branch_batch(&parent, &context, &original, &planned, NOW)
        .unwrap();

    let mut unload = fixture::create("alpha", "session-a");
    unload.accepted_version = 2;
    unload.request.binding.expected = ExpectedSession::Version(1);
    unload.request.binding.key.sequence = 11.try_into().unwrap();
    unload.request.operation = LifecycleOperation::Unload {
        source_incarnation: parent.incarnation,
        capture: 1.try_into().unwrap(),
    };
    assert!(matches!(
        journal.record_lifecycle_intent(&unload, &context, &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    journal
        .change_host_branch_close(
            &parent,
            &context,
            &original,
            NOW,
            HostBranchClose::Begin(None),
        )
        .unwrap();
    assert!(matches!(
        journal.record_lifecycle_intent(&unload, &context, &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    journal
        .change_host_branch_close(
            &parent,
            &context,
            &original,
            NOW,
            HostBranchClose::ConfirmGone(None),
        )
        .unwrap();
    journal
        .record_lifecycle_intent(&unload, &context, &mut NeverCrash)
        .unwrap();
    let after_unload = key(&parent, "after-unload", 12);
    assert!(
        journal
            .reserve_host_branch_batch(&parent, &context, &after_unload, &planned, NOW)
            .is_err()
    );
}

#[test]
fn unknown_branch_reservation_commit_reconciles_only_the_original_plan() {
    for commit_did_persist in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let (mut journal, parent, context) = running(&path);
        journal.configure_host_branching(LIMITS).unwrap();
        let original = key(&parent, "batch-a", 10);
        let planned = [target("a", 31_000)];
        crate::controller::transaction::fail_next_commit(commit_did_persist);
        assert!(matches!(
            journal.reserve_host_branch_batch(&parent, &context, &original, &planned, NOW),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = fixture::open(&path);
        fixture::refresh_epoch(&mut journal);
        let recovered = journal.pending_host_branch_guest_plan(&parent, &context, &original, NOW);
        assert_eq!(recovered.is_ok(), commit_did_persist);
        let existing_token = recovered.ok().map(|rows| rows[0].token().to_owned());
        assert_eq!(
            journal
                .reserve_host_branch_batch(&parent, &context, &original, &planned, NOW)
                .unwrap(),
            HostBranchBatchPhase::Pending
        );
        let after = journal
            .pending_host_branch_guest_plan(&parent, &context, &original, NOW)
            .unwrap();
        if let Some(existing) = existing_token {
            assert_eq!(after[0].token(), existing);
        }
    }
}

#[test]
fn host_branch_quota_retires_with_exact_terminal_parent_window_and_rejects_old_replay() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let (mut journal, original, context) = running(&path);
    journal
        .configure_host_branching(HostBranchLimits {
            max_batches: 2,
            ..LIMITS
        })
        .unwrap();
    let first = key(&original, "first", 10);
    let first_targets = [target("first", 31_000)];
    journal
        .reserve_host_branch_batch(&original, &context, &first, &first_targets, NOW)
        .unwrap();
    let create = fixture::create("alpha", "session-a");
    let local = journal
        .lifecycle_work_state(&create, &context)
        .unwrap()
        .local
        .unwrap();

    let (survivor, _) = admit_running(
        &mut journal,
        &fixture::create("bravo", "session-b"),
        &context,
    );
    let other = key(&survivor, "surviving", 10);
    let other_targets = [target("other", 31_000)];
    journal
        .reserve_host_branch_batch(&survivor, &context, &other, &other_targets, NOW)
        .unwrap();
    let other_token = journal
        .pending_host_branch_guest_plan(&survivor, &context, &other, NOW)
        .unwrap()[0]
        .token()
        .to_owned();
    let successor = key(&survivor, "next", 11);
    let successor_targets = [target("next", 31_001)];
    let must_retain_quota = |journal: &mut ControllerJournal| {
        assert!(matches!(
            journal.reserve_host_branch_batch(
                &survivor,
                &context,
                &successor,
                &successor_targets,
                NOW
            ),
            Err(JournalError::MaintenanceCapacity)
        ));
    };
    must_retain_quota(&mut journal);

    let close =
        confirm_terminal_branch_parent(&mut journal, &create, local, &context, must_retain_quota);
    crate::controller::transaction::fail_next_commit(false);
    assert!(matches!(
        journal.close_lifecycle_window(&close, &context, &mut NeverCrash),
        Err(JournalError::CommitUncertain(_))
    ));
    drop(journal);
    let mut journal = fixture::open(&path);
    fixture::refresh_epoch(&mut journal);
    must_retain_quota(&mut journal);

    crate::controller::transaction::fail_next_commit(true);
    assert!(matches!(
        journal.close_lifecycle_window(&close, &context, &mut NeverCrash),
        Err(JournalError::CommitUncertain(_))
    ));
    drop(journal);
    let mut journal = fixture::open(&path);
    fixture::refresh_epoch(&mut journal);
    let ack = journal
        .close_lifecycle_window(&close, &context, &mut NeverCrash)
        .unwrap();
    assert_eq!(ack.terminal, close.terminal);
    assert_eq!(ack.retired_through, close.terminal.request.retention.window);
    assert_eq!(
        journal
            .pending_host_branch_guest_plan(&survivor, &context, &other, NOW)
            .unwrap()[0]
            .token(),
        other_token,
    );
    assert!(matches!(
        journal.reserve_host_branch_batch(&original, &context, &first, &first_targets, NOW),
        Err(JournalError::Conflict)
    ));
    assert_eq!(
        journal
            .reserve_host_branch_batch(&survivor, &context, &successor, &successor_targets, NOW)
            .unwrap(),
        HostBranchBatchPhase::Pending,
    );
}

fn confirm_terminal_branch_parent(
    journal: &mut ControllerJournal,
    create: &AcceptedLifecycleIntent,
    local: InternalLocalBinding,
    context: &AuthenticatedCenterContext,
    must_retain_quota: impl Fn(&mut ControllerJournal),
) -> LifecycleWindowClose {
    let close = LifecycleWindowClose {
        terminal: TerminalLifecycleObservation {
            request: fixture::destroy(create),
            status: TerminalStatus::Cleaned,
        },
    };
    journal
        .record_lifecycle_intent(&close.terminal.request, context, &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        journal.close_lifecycle_window(&close, context, &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    must_retain_quota(journal);

    crate::finish_accepted_kill(
        journal,
        local.sandbox,
        sandbox_controller_core::OperationId(2),
        &mut crate::controller::tests_support::worker,
    )
    .unwrap();
    let position = journal.host_slot_context(local.sandbox).unwrap().unwrap();
    assert!(
        journal
            .release_host_slot(
                position,
                |_| Err(crate::WorkerTransportError::ReplyLost),
                &mut NeverCrash
            )
            .is_err()
    );
    must_retain_quota(journal);
    journal
        .release_host_slot(position, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    assert!(matches!(
        journal.close_lifecycle_window(&close, context, &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));
    must_retain_quota(journal);
    journal
        .confirm_lifecycle_terminal(&close.terminal, context, &mut NeverCrash)
        .unwrap();

    close
}
