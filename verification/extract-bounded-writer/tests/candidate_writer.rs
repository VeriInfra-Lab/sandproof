//! Only runs in the prepared candidate. No production source is overwritten.
#![cfg(feature = "candidate")]

use http_body_util::BodyExt;
use sandproof_bounded_writer_extract::candidate::{finish_entry, response_entry, write_entry};
use sandproof_controller_service::{
    JournalError,
    template_registry::evidence::{BoundedBuffer, BoundedWriter},
};
use sandproof_internal::ProofHttpJsonResponseBuilder;
use std::{
    future::Future,
    io,
    task::{Context, Poll, Waker},
};

struct RefuseReserve {
    bytes: Vec<u8>,
}

impl BoundedBuffer for RefuseReserve {
    type ReserveError = ();

    fn new_buffer() -> Self {
        Self { bytes: Vec::new() }
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn capacity(&self) -> usize {
        self.bytes.len()
    }

    fn try_reserve_exact(&mut self, _: usize) -> Result<(), Self::ReserveError> {
        // A controlled result for the candidate's library port. It does not
        // claim that the private original Vec allocator failed in this test.
        Err(())
    }

    fn extend_from_slice(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn capacity_error() -> io::Error {
        io::Error::other("bounded evidence exceeded")
    }

    fn allocation_error() -> io::Error {
        io::Error::other("bounded evidence allocation failed")
    }
}

#[test]
fn controlled_reserve_failure_preserves_prefix_without_setting_exceeded() {
    let mut writer = BoundedWriter {
        bytes: RefuseReserve { bytes: vec![1] },
        limit: 16,
        exceeded: false,
    };
    let error = write_entry(&mut writer, &[2, 3]).unwrap_err();
    assert_eq!(error.to_string(), "bounded evidence allocation failed");
    assert_eq!(writer.bytes.bytes, [1]);
    assert!(!writer.exceeded);
    assert!(matches!(
        finish_entry(writer, Err(serde_json::Error::io(error))),
        Err(JournalError::Json(_))
    ));
}

#[test]
fn a_prior_size_fault_remains_set_after_a_reserve_failure() {
    let mut writer = BoundedWriter {
        bytes: RefuseReserve { bytes: vec![1] },
        limit: 16,
        exceeded: true,
    };
    let error = write_entry(&mut writer, &[2]).unwrap_err();
    assert_eq!(writer.bytes.bytes, [1]);
    assert!(writer.exceeded);
    assert!(matches!(
        finish_entry(writer, Err(serde_json::Error::io(error))),
        Err(JournalError::MaintenanceCapacity)
    ));
}

#[test]
fn a_small_write_can_succeed_after_a_size_fault_without_clearing_it() {
    let mut writer = BoundedWriter {
        bytes: vec![1],
        limit: 2,
        exceeded: true,
    };
    assert_eq!(write_entry(&mut writer, &[2]).unwrap(), 1);
    assert_eq!(writer.bytes, [1, 2]);
    assert!(writer.exceeded);
    assert!(matches!(
        finish_entry(writer, Ok(())),
        Err(JournalError::MaintenanceCapacity)
    ));
}

#[test]
fn zero_length_write_is_not_rejected_just_because_exceeded_is_set() {
    let mut writer = BoundedWriter {
        bytes: Vec::<u8>::new(),
        limit: 0,
        exceeded: true,
    };
    assert_eq!(write_entry(&mut writer, &[]).unwrap(), 0);
    assert!(writer.bytes.is_empty());
    assert!(writer.exceeded);
}

#[test]
fn swallowed_allocation_failure_is_not_automatically_a_complete_serialization() {
    let mut writer = BoundedWriter {
        bytes: RefuseReserve { bytes: vec![b'['] },
        limit: 16,
        exceeded: false,
    };
    assert!(write_entry(&mut writer, b"1]").is_err());
    // This intentionally exposes the actual finish semantics if an arbitrary
    // callback lies about success. Full JSON needs the fixed derive contract.
    assert_eq!(finish_entry(writer, Ok(())).unwrap(), b"[");
}

#[test]
fn size_refusal_does_not_append_and_keeps_the_original_error_kind() {
    let mut writer = BoundedWriter {
        bytes: vec![1],
        limit: 1,
        exceeded: false,
    };
    let error = write_entry(&mut writer, &[2]).unwrap_err();
    assert_eq!(error.to_string(), "bounded evidence exceeded");
    assert_eq!(writer.bytes, [1]);
    assert!(writer.exceeded);
}

#[test]
fn actual_http_error_builder_returns_empty_500() {
    let response =
        response_entry::<ProofHttpJsonResponseBuilder>(Err(JournalError::MaintenanceCapacity));
    assert_eq!(response.status().as_u16(), 500);
    assert_eq!(response.headers()["connection"], "close");
    assert!(response.headers().get("content-type").is_none());
    let mut body = std::pin::pin!(response.into_body().collect());
    let mut context = Context::from_waker(Waker::noop());
    let Poll::Ready(Ok(collected)) = body.as_mut().poll(&mut context) else {
        panic!("Full response body must be ready");
    };
    assert!(collected.to_bytes().is_empty());
}

#[test]
fn actual_http_success_builder_keeps_the_exact_given_bytes() {
    let expected = br#"{"ok":1}"#.to_vec();
    let response = response_entry::<ProofHttpJsonResponseBuilder>(Ok(expected.clone()));
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.headers()["connection"], "close");
    assert_eq!(response.headers()["content-type"], "application/json");
    let mut body = std::pin::pin!(response.into_body().collect());
    let mut context = Context::from_waker(Waker::noop());
    let Poll::Ready(Ok(collected)) = body.as_mut().poll(&mut context) else {
        panic!("Full response body must be ready");
    };
    assert_eq!(collected.to_bytes().as_ref(), expected.as_slice());
}
