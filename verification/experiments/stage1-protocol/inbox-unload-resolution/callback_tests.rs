//! Candidate encoding interface only; no standard Serde connection is implied.
use super::proof_unload_entry::before;
use crate::JournalError;
use crate::controller::lifecycle_inbox::model::proof_unload_entry_using;
use crate::controller::lifecycle_inbox::unload_resolution::{UnloadEncoding, UnloadResolution};
use sandproof_protocol::lifecycle::LifecycleOperation;

struct NeverEncode;
impl UnloadEncoding for NeverEncode {
    fn encode(_: &UnloadResolution) -> Result<Vec<u8>, serde_json::Error> {
        panic!("encoder must not run before a present unload resolution");
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

#[test]
fn callback_order_and_error() {
    let mut entry = before();
    entry.unload_resolution = None;
    assert!(proof_unload_entry_using::<NeverEncode>(&entry).is_ok());
    let mut entry = before();
    entry.intent.request.operation = LifecycleOperation::Destroy {};
    assert!(matches!(
        proof_unload_entry_using::<NeverEncode>(&entry),
        Err(JournalError::CorruptStore)
    ));
    let mut entry = before();
    entry.local_operation = Some(sandbox_controller_core::OperationId::new(30));
    assert!(matches!(
        proof_unload_entry_using::<ErrorEncode>(&entry),
        Err(JournalError::Json(_))
    ));
}

#[test]
fn encoded_size_bound() {
    let entry = before();
    assert!(proof_unload_entry_using::<Bytes<4096>>(&entry).is_ok());
    assert!(matches!(
        proof_unload_entry_using::<Bytes<4097>>(&entry),
        Err(JournalError::CorruptStore)
    ));
}
