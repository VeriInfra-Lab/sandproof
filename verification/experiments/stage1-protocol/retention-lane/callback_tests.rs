//! Generic candidate only: no standard Serde equivalence is inferred.
use super::proof_retention_lane::normal;
use super::*;
use crate::controller::lifecycle_inbox::model::{
    TerminalEncoding, proof_retention_lane_valid_using,
};
use sandproof_protocol::lifecycle::TerminalLifecycleObservation;

struct NeverEncode;
impl TerminalEncoding for NeverEncode {
    fn encode(_: &TerminalLifecycleObservation) -> Result<Vec<u8>, serde_json::Error> {
        panic!("encoder must not run after an earlier lane decision");
    }
}
struct ErrorEncode;
impl TerminalEncoding for ErrorEncode {
    fn encode(_: &TerminalLifecycleObservation) -> Result<Vec<u8>, serde_json::Error> {
        Err(serde_json::from_slice::<serde_json::Value>(b"{").unwrap_err())
    }
}
struct Bytes<const N: usize>;
impl<const N: usize> TerminalEncoding for Bytes<N> {
    fn encode(_: &TerminalLifecycleObservation) -> Result<Vec<u8>, serde_json::Error> {
        Ok(vec![0; N])
    }
}

#[test]
fn absent_skips_encoding() {
    let mut lane = normal();
    lane.retired_through = 0;
    lane.current_window = Some(1_u64.try_into().unwrap());
    lane.last_retired = None;
    assert!(proof_retention_lane_valid_using::<NeverEncode>(
        &lane,
        &context()
    ));
}
#[test]
fn zero_floor_skips_encoding() {
    let mut lane = normal();
    lane.retired_through = 0;
    lane.current_window = Some(1_u64.try_into().unwrap());
    assert!(!proof_retention_lane_valid_using::<NeverEncode>(
        &lane,
        &context()
    ));
}
#[test]
fn encoding_error_rejects() {
    assert!(!proof_retention_lane_valid_using::<ErrorEncode>(
        &normal(),
        &context()
    ));
}
#[test]
fn exact_size_limit() {
    let lane = normal();
    assert!(proof_retention_lane_valid_using::<Bytes<8192>>(
        &lane,
        &context()
    ));
    assert!(!proof_retention_lane_valid_using::<Bytes<8193>>(
        &lane,
        &context()
    ));
}
