//! Candidate callback checks only; standard Serde and durable acceptance stay unproved.
use super::proof_entry_validation::{create_entry, inbox, session};
use super::{context, core};
use crate::JournalError;
use crate::controller::lifecycle_inbox::model::{Entry, proof_entry_kind_using};
use crate::controller::lifecycle_inbox::unload_resolution::{UnloadEncoding, UnloadResolution};
use sandproof_protocol::lifecycle::{CurrentReadyFacts, ExpectedSession, LifecycleOperation};

struct NeverEncode;
impl UnloadEncoding for NeverEncode {
    fn encode(_: &UnloadResolution) -> Result<Vec<u8>, serde_json::Error> {
        panic!("encoding must not run before a present unload resolution");
    }
}
struct ErrorEncode;
impl UnloadEncoding for ErrorEncode {
    fn encode(_: &UnloadResolution) -> Result<Vec<u8>, serde_json::Error> {
        Err(serde_json::from_slice::<serde_json::Value>(b"{").unwrap_err())
    }
}
struct Bytes<const N: usize>;
impl<const N: usize> UnloadEncoding for Bytes<N> {
    fn encode(_: &UnloadResolution) -> Result<Vec<u8>, serde_json::Error> {
        Ok(vec![0; N])
    }
}

fn unload() -> Entry {
    let mut value = create_entry();
    value.intent.request.binding.expected = ExpectedSession::Version(0);
    value.intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    value.local_operation = Some(core::OperationId::new(30));
    value.unload_resolution = Some(UnloadResolution::AfterCapture {
        facts: CurrentReadyFacts {
            started_at_ms: 0,
            observed_at_ms: 1,
            end_at_ms: 10,
            max_end_at_ms: 20,
            startup_deadline_ms: 3,
        },
    });
    value
}

#[test]
fn callback_order_and_error() {
    let value = create_entry();
    assert!(matches!(
        proof_entry_kind_using::<NeverEncode>(&inbox(&value), &session(&value), &value, &context()),
        Ok(0)
    ));
    let mut value = unload();
    value.intent.request.operation = LifecycleOperation::Destroy {};
    assert!(matches!(
        proof_entry_kind_using::<NeverEncode>(&inbox(&value), &session(&value), &value, &context()),
        Err(JournalError::CorruptStore)
    ));
    let mut value = unload();
    value.intent.request.binding.protocol_version = 3;
    assert!(matches!(
        proof_entry_kind_using::<ErrorEncode>(&inbox(&value), &session(&value), &value, &context()),
        Err(JournalError::Json(_))
    ));
}

#[test]
fn encoded_size_bound() {
    let value = unload();
    assert!(matches!(
        proof_entry_kind_using::<Bytes<4096>>(&inbox(&value), &session(&value), &value, &context()),
        Ok(1)
    ));
    assert!(matches!(
        proof_entry_kind_using::<Bytes<4097>>(&inbox(&value), &session(&value), &value, &context()),
        Err(JournalError::CorruptStore)
    ));
}
