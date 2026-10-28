use crate::controller::lifecycle_inbox::native_retirement::{
    RetirementEncoding, proof_native_retirement_saved,
};
use sandproof_protocol::lifecycle::ContentDigest;
use std::cell::RefCell;

thread_local! {
    static ENCODING_CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

struct OriginalEncoding;

impl RetirementEncoding for OriginalEncoding {
    fn encode_intent(value: &AcceptedLifecycleIntent) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }

    fn digest(bytes: &[u8]) -> ContentDigest {
        source_work::digest(bytes)
    }

    fn encode_receipt(value: &NativeRetirementReceipt) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }
}

struct Probe<const INTENT_ERROR: bool, const RECEIPT_ERROR: bool, const SIZE: usize>;

impl<const I: bool, const R: bool, const S: usize> RetirementEncoding for Probe<I, R, S> {
    fn encode_intent(value: &AcceptedLifecycleIntent) -> Result<Vec<u8>, serde_json::Error> {
        ENCODING_CALLS.with_borrow_mut(|calls| calls.push("intent"));
        if I {
            Err(serde_json::from_str::<serde_json::Value>("{").unwrap_err())
        } else {
            serde_json::to_vec(value)
        }
    }

    fn digest(bytes: &[u8]) -> ContentDigest {
        ENCODING_CALLS.with_borrow_mut(|calls| calls.push("digest"));
        source_work::digest(bytes)
    }

    fn encode_receipt(_value: &NativeRetirementReceipt) -> Result<Vec<u8>, serde_json::Error> {
        ENCODING_CALLS.with_borrow_mut(|calls| calls.push("receipt"));
        if R {
            Err(serde_json::from_str::<serde_json::Value>("{").unwrap_err())
        } else {
            Ok(vec![0; S])
        }
    }
}

fn probe<const I: bool, const R: bool, const S: usize>(
    value: &Session,
    expected_calls: &[&str],
) -> Result<(), JournalError> {
    ENCODING_CALLS.with_borrow_mut(Vec::clear);
    let before = value.clone();
    let result = proof_native_retirement_saved::<Probe<I, R, S>>(value);
    ENCODING_CALLS.with_borrow(|calls| assert_eq!(calls, expected_calls));
    assert_eq!(*value, before);
    result
}

#[test]
fn generic_original_encoding_accepts_normal() {
    proof_native_retirement_saved::<OriginalEncoding>(&normal()).unwrap();
}

#[test]
fn absent_and_prefix_refusals_skip_callbacks() {
    let mut absent = normal();
    absent.native_retirement = None;
    absent.entries.clear();
    probe::<true, true, 8193>(&absent, &[]).unwrap();
    let changes: [fn(&mut Session); 9] = [
        |s| s.entries.clear(),
        |s| s.terminal = false,
        |s| s.local = None,
        |s| s.local.as_mut().unwrap().sandbox = core::SandboxId(99),
        |s| s.local.as_mut().unwrap().owner = core::TenantId(99),
        |s| s.local.as_mut().unwrap().operation = core::OperationId(99),
        |s| {
            s.entries.last_mut().unwrap().intent.request.operation = LifecycleOperation::Renew {
                incarnation: 101_u64.try_into().unwrap(),
                end_at_ms: 100,
                extend_only: false,
            }
        },
        |s| s.current_version = 3,
        |s| s.entries.last_mut().unwrap().local_operation = None,
    ];
    for change in changes {
        let mut value = normal();
        change(&mut value);
        assert!(matches!(
            probe::<true, true, 8193>(&value, &[]),
            Err(JournalError::CorruptStore)
        ));
    }
}

#[test]
fn intent_error_precedes_receipt_fields() {
    let mut value = normal();
    change_saved(&mut value, "/retirement/owner", serde_json::json!(99));
    assert!(matches!(
        probe::<true, true, 8193>(&value, &["intent"]),
        Err(JournalError::Json(_))
    ));
}

#[test]
fn binding_and_fields_skip_receipt_encoding() {
    let mut changed_intent = normal();
    changed_intent
        .entries
        .last_mut()
        .unwrap()
        .intent
        .center
        .center = "other".try_into().unwrap();
    assert!(matches!(
        probe::<false, true, 8193>(&changed_intent, &["intent", "digest"]),
        Err(JournalError::CorruptStore)
    ));
    for pointer in [
        "/retirement/sandbox",
        "/retirement/owner",
        "/retirement/quiescence/sandbox",
        "/retirement/quiescence/owner",
        "/retirement/quiescence/epoch",
        "/retirement/quiescence/operation",
    ] {
        let mut value = normal();
        change_saved(&mut value, pointer, serde_json::json!(99));
        assert!(matches!(
            probe::<false, true, 8193>(&value, &["intent", "digest"]),
            Err(JournalError::CorruptStore)
        ));
    }
    for field in [
        "worker_drained",
        "data_connections_drained",
        "host_resources_reclaimed",
        "external_pins_released",
    ] {
        let mut value = normal();
        change_saved(
            &mut value,
            &format!("/retirement/quiescence/{field}"),
            serde_json::json!("Unconfirmed"),
        );
        assert!(matches!(
            probe::<false, true, 8193>(&value, &["intent", "digest"]),
            Err(JournalError::CorruptStore)
        ));
    }
}

#[test]
fn receipt_error_after_all_fields() {
    assert!(matches!(
        probe::<false, true, 8193>(&normal(), &["intent", "digest", "receipt"]),
        Err(JournalError::Json(_))
    ));
}

#[test]
fn receipt_size_accepts_boundary_and_rejects_successor() {
    probe::<false, false, 8192>(&normal(), &["intent", "digest", "receipt"]).unwrap();
    assert!(matches!(
        probe::<false, false, 8193>(&normal(), &["intent", "digest", "receipt"]),
        Err(JournalError::CorruptStore)
    ));
}
