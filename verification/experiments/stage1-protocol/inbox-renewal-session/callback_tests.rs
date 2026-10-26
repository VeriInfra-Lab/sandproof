//! Candidate interface tests; these do not prove original Serde equivalence.
use super::proof_renewal_session::*;
use crate::JournalError;
use crate::controller::lifecycle_inbox::model::proof_renewal_session_using;
use crate::controller::lifecycle_inbox::renewal_confirmation::RenewalEncoding;
use sandproof_protocol::lifecycle::{RENEWAL_CONFIRMATION_BODY_BYTES, RenewalConfirmation};

struct NeverEncode;
impl RenewalEncoding for NeverEncode {
    fn encode(_: &RenewalConfirmation) -> Result<Vec<u8>, serde_json::Error> {
        panic!("encoder must not be called before confirmation shape acceptance")
    }
}

struct ErrorEncode;
impl RenewalEncoding for ErrorEncode {
    fn encode(_: &RenewalConfirmation) -> Result<Vec<u8>, serde_json::Error> {
        Err(serde_json::from_slice::<serde_json::Value>(b"{").unwrap_err())
    }
}

struct Bytes<const N: usize>;
impl<const N: usize> RenewalEncoding for Bytes<N> {
    fn encode(_: &RenewalConfirmation) -> Result<Vec<u8>, serde_json::Error> {
        Ok(vec![0; N])
    }
}

#[test]
fn callback_order_and_error() {
    let mut session = legal(2);
    session.confirmed_renewal = None;
    assert!(proof_renewal_session_using::<NeverEncode>(&session).is_ok());
    let mut session = legal(2);
    session
        .confirmed_renewal
        .as_mut()
        .unwrap()
        .effective_end_at_ms = 199;
    assert!(matches!(
        proof_renewal_session_using::<NeverEncode>(&session),
        Err(JournalError::CorruptStore)
    ));
    let mut session = legal(2);
    session.local = None;
    assert!(matches!(
        proof_renewal_session_using::<ErrorEncode>(&session),
        Err(JournalError::Json(_))
    ));
}

#[test]
fn encoded_size_bound() {
    let session = legal(2);
    assert!(
        proof_renewal_session_using::<Bytes<RENEWAL_CONFIRMATION_BODY_BYTES>>(&session).is_ok()
    );
    assert!(matches!(
        proof_renewal_session_using::<Bytes<{ RENEWAL_CONFIRMATION_BODY_BYTES + 1 }>>(&session),
        Err(JournalError::CorruptStore)
    ));
}
