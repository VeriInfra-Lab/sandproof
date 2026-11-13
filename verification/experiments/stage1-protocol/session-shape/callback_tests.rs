use crate::controller::lifecycle_inbox::model::{TerminalEncoding, proof_inbox_session_shape};
use crate::controller::lifecycle_inbox::native_retirement::{
    NativeRetirementReceipt, RetirementEncoding,
};
use crate::controller::lifecycle_inbox::source_work;
use sandbox_controller_core::maintenance::retirement::{
    QuiescenceCertificate, QuiescenceStatus, RetireRequest,
};
use sandproof_protocol::lifecycle::ContentDigest;
use std::cell::RefCell;

// Reuse the established native-retirement fixture and its complete request checks.
fn saved_session() -> Session {
    let initial = create("owner", "session");
    let mut destroy = initial.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2_u64.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    assert_eq!(initial.validate(&context()), Ok(()));
    assert_eq!(destroy.validate(&context()), Ok(()));
    let local = InternalLocalBinding {
        sandbox: core::SandboxId(10),
        owner: core::TenantId(20),
        operation: core::OperationId(30),
    };
    let retirement = RetireRequest {
        sandbox: local.sandbox,
        owner: local.owner,
        epoch: 5,
        operation: core::OperationId(31),
        retain_until: 100,
        quiescence: QuiescenceCertificate {
            sandbox: local.sandbox,
            owner: local.owner,
            epoch: 5,
            operation: core::OperationId(31),
            workspace: core::FsId(6),
            worker_drained: QuiescenceStatus::Confirmed,
            data_connections_drained: QuiescenceStatus::Confirmed,
            host_resources_reclaimed: QuiescenceStatus::Confirmed,
            external_pins_released: QuiescenceStatus::Confirmed,
        },
    };
    let saved: NativeRetirementReceipt = serde_json::from_value(serde_json::json!({
        "local": local,
        "request_binding": source_work::digest(&serde_json::to_vec(&destroy).unwrap()),
        "retirement": retirement,
    }))
    .unwrap();
    let entry = |intent, operation| Entry {
        intent,
        local_operation: Some(core::OperationId(operation)),
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: None,
    };
    Session {
        id: initial.request.binding.session.clone(),
        retention: initial.retention,
        owner: initial.request.binding.key.owner.clone(),
        current_version: 2,
        terminal: true,
        refusal: None,
        incarnation: Some(101_u64.try_into().unwrap()),
        local: Some(local),
        entries: vec![entry(initial, 30), entry(destroy, 31)],
        confirmed_terminal: None,
        confirmed_renewal: None,
        compacted_sdk_through: None,
        native_retirement: Some(saved),
    }
}

thread_local! {
    static CALLBACKS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

struct SavedProbe<const ERROR: bool>;
impl<const E: bool> RetirementEncoding for SavedProbe<E> {
    fn encode_intent(value: &AcceptedLifecycleIntent) -> Result<Vec<u8>, serde_json::Error> {
        CALLBACKS.with_borrow_mut(|calls| calls.push("intent"));
        if E {
            Err(serde_json::from_str::<serde_json::Value>("{").unwrap_err())
        } else {
            serde_json::to_vec(value)
        }
    }
    fn digest(bytes: &[u8]) -> ContentDigest {
        CALLBACKS.with_borrow_mut(|calls| calls.push("digest"));
        source_work::digest(bytes)
    }
    fn encode_receipt(_value: &NativeRetirementReceipt) -> Result<Vec<u8>, serde_json::Error> {
        CALLBACKS.with_borrow_mut(|calls| calls.push("receipt"));
        Ok(vec![])
    }
}

struct TerminalProbe<const ERROR: bool, const SIZE: usize>;
impl<const E: bool, const S: usize> TerminalEncoding for TerminalProbe<E, S> {
    fn encode(_value: &TerminalLifecycleObservation) -> Result<Vec<u8>, serde_json::Error> {
        CALLBACKS.with_borrow_mut(|calls| calls.push("terminal"));
        if E {
            Err(serde_json::from_str::<serde_json::Value>("{").unwrap_err())
        } else {
            Ok(vec![0; S])
        }
    }
}

fn probe<const R: bool, const E: bool, const S: usize>(
    inbox: &Inbox,
    session: &Session,
    expected: &[&str],
) -> Result<(), JournalError> {
    CALLBACKS.with_borrow_mut(Vec::clear);
    let before = (inbox.clone(), session.clone());
    let result = proof_inbox_session_shape::<SavedProbe<R>, TerminalProbe<E, S>>(inbox, 0, session);
    CALLBACKS.with_borrow(|calls| assert_eq!(calls, expected));
    assert_eq!((inbox.clone(), session.clone()), before);
    result
}

#[test]
fn terminal_size_accepts_boundary_and_rejects_successor() {
    let (inbox, session) = terminal();
    probe::<false, false, 8192>(&inbox, &session, &["terminal"]).unwrap();
    assert!(matches!(
        probe::<false, false, 8193>(&inbox, &session, &["terminal"]),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn terminal_error_only_after_earlier_guards() {
    let (inbox, session) = terminal();
    assert!(matches!(
        probe::<false, true, 0>(&inbox, &session, &["terminal"]),
        Err(JournalError::Json(_))
    ));
    let mut wrong = session.clone();
    wrong.retention.slot = 0;
    assert!(matches!(
        probe::<false, true, 0>(&inbox, &wrong, &[]),
        Err(JournalError::CorruptStore)
    ));
    wrong = session;
    wrong.confirmed_terminal.as_mut().unwrap().status = TerminalStatus::Pending;
    assert!(matches!(
        probe::<false, true, 0>(&inbox, &wrong, &[]),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn receipt_error_precedes_session_guards() {
    let (inbox, _) = normal();
    let mut session = saved_session();
    session.retention.slot = 0;
    assert!(matches!(
        probe::<true, true, 0>(&inbox, &session, &["intent"]),
        Err(JournalError::Json(_))
    ));
}

#[test]
fn receipt_and_terminal_callback_order_is_preserved() {
    let (mut inbox, _) = normal();
    let mut session = saved_session();
    local(&mut inbox, &mut session);
    session.confirmed_terminal = Some(TerminalLifecycleObservation {
        request: session.entries.last().unwrap().intent.clone(),
        status: TerminalStatus::Cleaned,
    });
    probe::<false, false, 0>(
        &inbox,
        &session,
        &["intent", "digest", "receipt", "terminal"],
    )
    .unwrap();
}
