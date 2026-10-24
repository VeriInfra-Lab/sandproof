//! Channel-controlled retirement at the original application's boundaries.
//! All injected facts/retirement use the original pure reducers; these unit
//! ports do not attest Redis, authenticated host storage, or physical cleanup.
use super::*;
use std::{sync::mpsc, thread::JoinHandle};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Checkpoint {
    AfterDestroyAccepted,
    BeforeConfirmationCommit,
    BeforeTerminalApply,
    RetiredSnapshot,
}

pub(super) struct RetirementStep {
    checkpoint: Checkpoint,
    skip_reads: usize,
    gate: NetworkGate,
}

impl MemoryAuthority {
    pub(super) fn retire_at_checkpoint(
        &mut self,
        checkpoint: Checkpoint,
    ) -> Result<(), AuthorityError> {
        let Some(step) = self.retirement_step.as_mut() else {
            return Ok(());
        };
        if step.checkpoint != checkpoint {
            return Ok(());
        }
        if step.skip_reads > 0 {
            step.skip_reads -= 1;
            return Ok(());
        }
        let step = self.retirement_step.take().unwrap();
        step.gate.entered.send(()).unwrap();
        step.gate.release.recv().unwrap();
        if checkpoint == Checkpoint::RetiredSnapshot {
            return Ok(());
        }
        let current = self.state.sandboxes()[0].operations().last().unwrap();
        let terminal =
            current
                .terminal()
                .cloned()
                .unwrap_or_else(|| TerminalLifecycleObservation {
                    request: current.intent().clone(),
                    status: TerminalStatus::Cleaned,
                });
        let recorded_at = self.clock.0.load(Ordering::SeqCst);
        // An injected completed background result at an actual provider boundary.
        // All state changes still use the same production reducers.
        self.state = self
            .state
            .accept_lifecycle_terminal_observation(&terminal, recorded_at)?;
        self.state = self.state.confirm_terminal(&ConfirmedLifecycleResult {
            result: terminal.clone(),
        })?;
        let now = recorded_at.checked_add(RESULT_RETENTION_MS).unwrap();
        self.clock.0.store(now, Ordering::SeqCst);
        let (pending, close) = self.state.begin_close(&terminal, now)?;
        self.state = pending.apply_close_ack(
            &ClosedLifecycleWindow {
                retired_through: close.terminal.request.retention.window,
                terminal: close.terminal,
            },
            now,
        )?;
        Ok(())
    }
}

fn gate() -> (NetworkGate, mpsc::Receiver<()>, mpsc::SyncSender<()>) {
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    (
        NetworkGate {
            entered,
            release: resume,
        },
        observed,
        release,
    )
}

fn racing_fixture() -> Fixture {
    let mut f = fixture();
    Arc::get_mut(&mut f.app).unwrap().config.reply_timeout = Duration::from_secs(2);
    f
}

fn install_checkpoint(
    f: &Fixture,
    checkpoint: Checkpoint,
    skip_reads: usize,
) -> (mpsc::Receiver<()>, mpsc::SyncSender<()>) {
    let (gate, entered, release) = gate();
    f.app.authority.lock().unwrap().retirement_step = Some(RetirementStep {
        checkpoint,
        skip_reads,
        gate,
    });
    (entered, release)
}

fn start_kill(f: &Fixture, sandbox: String) -> JoinHandle<Result<(), PortError>> {
    let app = Arc::clone(&f.app);
    std::thread::spawn(move || app.kill("1", &sandbox))
}

fn observe(entered: &mpsc::Receiver<()>) {
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
}

fn current_destroy(f: &Fixture) -> AcceptedLifecycleIntent {
    f.app.authority.lock().unwrap().state.sandboxes()[0]
        .operations()
        .last()
        .unwrap()
        .intent()
        .clone()
}
fn save_cleaned(f: &Fixture, request: &AcceptedLifecycleIntent) -> TerminalLifecycleObservation {
    let terminal = TerminalLifecycleObservation {
        request: request.clone(),
        status: TerminalStatus::Cleaned,
    };
    f.app
        .authority
        .lock()
        .unwrap()
        .accept_lifecycle_terminal_observation(&terminal, f.clock.0.load(Ordering::SeqCst))
        .unwrap();
    f.host.terminals.lock().unwrap().push(terminal.clone());
    terminal
}
fn assert_retired(f: &Fixture, request: &AcceptedLifecycleIntent) {
    let state = f.app.snapshot().unwrap();
    assert!(state.sandboxes().is_empty());
    let terminal = state
        .lane(&request.request.binding.node, request.retention.slot)
        .unwrap()
        .last_retired()
        .unwrap();
    assert_eq!(terminal.status, TerminalStatus::Cleaned);
    assert!(terminal.matches_request(request));
    assert_eq!(
        f.app.kill(
            "1",
            request.request.binding.session.as_protocol_id().as_str()
        ),
        Err(PortError::NotFound)
    );
}

#[test]
fn accepted_destroy_retired_before_first_wait_snapshot_still_succeeds() {
    let f = racing_fixture();
    let session = f.app.create("1", command()).unwrap();
    let (entered, release) = install_checkpoint(&f, Checkpoint::AfterDestroyAccepted, 0);
    let killer = start_kill(&f, session.sandbox_id);
    observe(&entered);
    release.send(()).unwrap();
    assert_eq!(killer.join().unwrap(), Ok(()));
    // No destroy is resent to the host after its original history retired.
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 1);
    let state = f.app.snapshot().unwrap();
    assert!(state.sandboxes().is_empty());
    assert_eq!(
        state.retention_lanes()[0].last_retired().unwrap().status,
        TerminalStatus::Cleaned
    );
}

#[test]
fn retired_destroy_recovers_after_detached_reply_cannot_apply_to_removed_record() {
    for status in [TerminalStatus::Pending, TerminalStatus::Cleaned] {
        let f = racing_fixture();
        let session = f.app.create("1", command()).unwrap();
        let (network, entered, release) = gate();
        *f.host.gate.lock().unwrap() = Some(network);
        let killer = start_kill(&f, session.sandbox_id);
        observe(&entered);
        let request = current_destroy(&f);
        save_cleaned(&f, &request);
        f.app.reconcile_once().unwrap();
        advance_retention(&f);
        f.app.retire_terminal_history_once().unwrap();
        *f.host.late_terminal.lock().unwrap() = Some(TerminalLifecycleObservation {
            request: request.clone(),
            status,
        });
        release.send(()).unwrap();
        assert_eq!(killer.join().unwrap(), Ok(()));
        assert_retired(&f, &request);
        // Background confirmation re-records the same current Destroy.
        assert_eq!(f.host.calls.load(Ordering::SeqCst), 3);
        assert_eq!(f.app.snapshot().unwrap().identity_frontiers(), (1, 2));
    }
}

#[test]
fn retired_destroy_recovers_when_detached_terminal_apply_finds_original_retired() {
    let f = racing_fixture();
    let session = f.app.create("1", command()).unwrap();
    let (network, sent, send_release) = gate();
    *f.host.gate.lock().unwrap() = Some(network);
    let killer = start_kill(&f, session.sandbox_id);
    observe(&sent);
    let request = current_destroy(&f);
    save_cleaned(&f, &request);
    let (stale, stale_release) = install_checkpoint(&f, Checkpoint::BeforeTerminalApply, 0);
    *f.host.late_terminal.lock().unwrap() = Some(TerminalLifecycleObservation {
        request: request.clone(),
        status: TerminalStatus::Cleaned,
    });
    send_release.send(()).unwrap();
    observe(&stale);
    stale_release.send(()).unwrap();
    assert_eq!(killer.join().unwrap(), Ok(()));
    assert_retired(&f, &request);
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn retired_destroy_recovers_before_original_confirmation_commit() {
    let f = racing_fixture();
    let session = f.app.create("1", command()).unwrap();
    let (entered, release) = install_checkpoint(&f, Checkpoint::BeforeConfirmationCommit, 0);
    let killer = start_kill(&f, session.sandbox_id);
    observe(&entered);
    release.send(()).unwrap();
    assert_eq!(killer.join().unwrap(), Ok(()));
    let state = f.app.snapshot().unwrap();
    assert!(state.sandboxes().is_empty());
    assert_eq!(f.host.confirmation_budgets.lock().unwrap().len(), 1);
}

#[test]
fn retired_destroy_recovers_after_optional_confirmation_rpc_before_local_commit() {
    let f = racing_fixture();
    let session = f.app.create("1", command()).unwrap();
    let (network, entered, release) = gate();
    *f.host.confirmation_gate.lock().unwrap() = Some(network);
    let killer = start_kill(&f, session.sandbox_id);
    observe(&entered);
    let request = current_destroy(&f);
    // The background shares the same authority and can complete the exact
    // confirmation/close while the original confirmation response is detached.
    f.app.reconcile_once().unwrap();
    advance_retention(&f);
    f.app.retire_terminal_history_once().unwrap();
    release.send(()).unwrap();
    assert_eq!(killer.join().unwrap(), Ok(()));
    assert_retired(&f, &request);
    assert_eq!(f.host.confirmation_budgets.lock().unwrap().len(), 2);
}

fn retired_fixture() -> (Fixture, AcceptedLifecycleIntent, AcceptedLifecycleIntent) {
    let f = racing_fixture();
    let session = f.app.create("1", command()).unwrap();
    let create = f.host.last.lock().unwrap().clone().unwrap();
    f.app.kill("1", &session.sandbox_id).unwrap();
    let destroy = current_destroy(&f);
    advance_retention(&f);
    f.app.retire_terminal_history_once().unwrap();
    (f, create, destroy)
}

#[test]
fn retired_receipt_rejects_old_create_other_full_identity_and_future_window() {
    let (f, create, destroy) = retired_fixture();
    let mut wrong_node = destroy.clone();
    wrong_node.request.binding.node.node_id = "other-host".try_into().unwrap();
    let mut wrong_owner = destroy.clone();
    wrong_owner.request.binding.key.owner = "2".try_into().unwrap();
    let mut wrong_operation = destroy.clone();
    wrong_operation.request.binding.key.sequence = 1.try_into().unwrap();
    let mut wrong_version = destroy.clone();
    wrong_version.accepted_version = 1;
    let mut wrong_intent = destroy.clone();
    wrong_intent.request.operation = create.request.operation.clone();
    let mut future_window = destroy.clone();
    future_window.retention.window = destroy.retention.window.checked_next().unwrap();
    for request in [
        create,
        wrong_node,
        wrong_owner,
        wrong_operation,
        wrong_version,
        wrong_intent,
        future_window,
    ] {
        assert_eq!(
            f.app.wait_current_terminal(&request),
            Err(PortError::OutcomeUnknown)
        );
    }
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 2);
    assert_retired(&f, &destroy);
}

#[test]
fn successor_does_not_invalidate_exact_retired_receipt_but_its_later_retirement_does() {
    let (f, _, destroy) = retired_fixture();
    let successor = f.app.create("1", command()).unwrap();
    let successor_request = f.host.last.lock().unwrap().clone().unwrap();
    assert_eq!(successor_request.retention.slot, destroy.retention.slot);
    let before = f.app.get("1", &successor.sandbox_id).unwrap();
    assert_eq!(f.app.wait_current_terminal(&destroy), Ok(()));
    assert_eq!(f.app.get("1", &successor.sandbox_id).unwrap(), before);
    f.app.kill("1", &successor.sandbox_id).unwrap();
    advance_retention(&f);
    f.app.retire_terminal_history_once().unwrap();
    assert_eq!(
        f.app.wait_current_terminal(&destroy),
        Err(PortError::OutcomeUnknown)
    );
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 4);
}

#[test]
fn fallback_durable_snapshot_fault_cannot_reuse_the_retired_receipt() {
    let (f, _, destroy) = retired_fixture();
    // Skip send_current's snapshot, then gate the fallback's fresh read.
    let (entered, release) = install_checkpoint(&f, Checkpoint::RetiredSnapshot, 1);
    let app = Arc::clone(&f.app);
    let request = destroy.clone();
    let waiter = std::thread::spawn(move || app.wait_current_terminal(&request));
    observe(&entered);
    f.unreadable.store(true, Ordering::SeqCst);
    release.send(()).unwrap();
    assert_eq!(waiter.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert!(
        f.app.authority.lock().unwrap().state.retention_lanes()[0]
            .last_retired()
            .is_some()
    );
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn fallback_snapshot_crossing_original_reply_deadline_cannot_report_late_success() {
    let (f, _, destroy) = retired_fixture();
    let (entered, release) = install_checkpoint(&f, Checkpoint::RetiredSnapshot, 1);
    let app = Arc::clone(&f.app);
    let waiter = std::thread::spawn(move || app.wait_current_terminal(&destroy));
    observe(&entered);
    consume_reply_budget(f.app.config.reply_timeout);
    release.send(()).unwrap();
    assert_eq!(waiter.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 2);
}
