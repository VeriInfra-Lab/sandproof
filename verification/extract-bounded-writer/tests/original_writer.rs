//! These call the original public encoder, including its private Writer.
//! Synthetic Serialize callbacks exercise error histories, not business logic.
use sandproof_bounded_writer_extract::encode_status;
use sandproof_controller_service::{JournalError, template_registry::evidence::encode_bounded};
use sandproof_internal::status::HostStatus;
use sandproof_protocol::{
    DeploymentId, NodeId,
    internal::{NodeIdentity, PositiveCounter, lifecycle::Resources},
};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use std::cell::Cell;

fn status(maximum: bool) -> HostStatus {
    HostStatus {
        node: NodeIdentity {
            deployment_id: DeploymentId::try_from("deployment-a").unwrap(),
            node_id: NodeId::try_from("node-a").unwrap(),
            node_generation: PositiveCounter::try_from(if maximum { u64::MAX } else { 1 }).unwrap(),
        },
        retention_slots: if maximum { u16::MAX } else { 1 },
        observed_at_ms: if maximum { u64::MAX } else { 0 },
        free: Resources {
            instances: 0,
            cpu: if maximum { u64::MAX } else { 0 },
            memory_bytes: if maximum { u64::MAX } else { 0 },
            disk_bytes: if maximum { u64::MAX } else { 0 },
        },
    }
}

#[test]
fn successful_status_encoding_never_exceeds_the_original_limit() {
    for value in [status(false), status(true)] {
        let expected = serde_json::to_vec(&value).unwrap();
        for limit in 0..=expected.len() + 1 {
            match encode_status(&value, limit) {
                Ok(bytes) => {
                    assert!(
                        bytes.len() <= limit,
                        "bounded encoder published too many bytes"
                    );
                    assert_eq!(bytes, expected);
                    assert!(limit >= expected.len());
                }
                Err(JournalError::MaintenanceCapacity) => assert!(limit < expected.len()),
                Err(error) => panic!("unexpected error at byte budget {limit}: {error:?}"),
            }
        }
    }
}

#[test]
fn exact_status_boundary_preserves_all_fields() {
    let value = status(true);
    let bytes = serde_json::to_vec(&value).unwrap();
    let actual = encode_status(&value, bytes.len()).unwrap();
    assert_eq!(actual, bytes);
    assert_eq!(
        serde_json::from_slice::<HostStatus>(&actual).unwrap(),
        value
    );
}

#[test]
fn zero_limit_refuses_before_the_first_status_byte() {
    assert!(matches!(
        encode_status(&status(false), 0),
        Err(JournalError::MaintenanceCapacity)
    ));
}

#[test]
fn empty_array_has_an_exact_two_byte_boundary() {
    let empty: Vec<u8> = Vec::new();
    assert!(matches!(
        encode_bounded(&empty, 0),
        Err(JournalError::MaintenanceCapacity)
    ));
    assert!(matches!(
        encode_bounded(&empty, 1),
        Err(JournalError::MaintenanceCapacity)
    ));
    assert_eq!(encode_bounded(&empty, 2).unwrap(), b"[]");
}

struct FailBeforeOutput;

impl Serialize for FailBeforeOutput {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("synthetic failure before output"))
    }
}

#[test]
fn serialization_failure_without_output_remains_json_error() {
    assert!(matches!(
        encode_bounded(&FailBeforeOutput, 32),
        Err(JournalError::Json(_))
    ));
}

struct FailAfterPrefix<'a>(&'a Cell<bool>);

impl Serialize for FailAfterPrefix<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        sequence.serialize_element(&1_u8)?;
        self.0.set(true);
        Err(serde::ser::Error::custom("synthetic failure after prefix"))
    }
}

#[test]
fn serialization_failure_after_a_successful_prefix_is_not_capacity() {
    let prefix_completed = Cell::new(false);
    let result = encode_bounded(&FailAfterPrefix(&prefix_completed), 64);
    assert!(prefix_completed.get());
    assert!(matches!(result, Err(JournalError::Json(_))));
}

struct ContinueAfterError<'a> {
    observed_error: &'a Cell<bool>,
    continued: &'a Cell<bool>,
    successful_end: &'a Cell<bool>,
    fail_at_end: bool,
}

impl Serialize for ContinueAfterError<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        let first = sequence.serialize_element(&"012345678901234567890123456789");
        self.observed_error.set(first.is_err());
        // Deliberately retain the real history after an ignored write failure.
        sequence.serialize_element(&0_u8)?;
        self.continued.set(true);
        if self.fail_at_end {
            return Err(serde::ser::Error::custom("synthetic later failure"));
        }
        let result = sequence.end();
        self.successful_end.set(result.is_ok());
        result
    }
}

#[test]
fn exceeded_is_sticky_even_when_the_serializer_later_returns_ok() {
    let observed_error = Cell::new(false);
    let continued = Cell::new(false);
    let successful_end = Cell::new(false);
    let value = ContinueAfterError {
        observed_error: &observed_error,
        continued: &continued,
        successful_end: &successful_end,
        fail_at_end: false,
    };
    let result = encode_bounded(&value, 8);
    assert!(observed_error.get());
    assert!(continued.get());
    assert!(successful_end.get());
    assert!(matches!(result, Err(JournalError::MaintenanceCapacity)));
}

#[test]
fn exceeded_has_priority_over_a_later_serialization_error() {
    let observed_error = Cell::new(false);
    let continued = Cell::new(false);
    let successful_end = Cell::new(false);
    let value = ContinueAfterError {
        observed_error: &observed_error,
        continued: &continued,
        successful_end: &successful_end,
        fail_at_end: true,
    };
    let result = encode_bounded(&value, 8);
    assert!(observed_error.get());
    assert!(continued.get());
    assert!(!successful_end.get());
    assert!(matches!(result, Err(JournalError::MaintenanceCapacity)));
}

#[test]
fn private_prefix_does_not_change_a_later_independent_encoding() {
    let _ = encode_bounded(&"too-long", 1);
    assert_eq!(encode_bounded(&7_u8, 1).unwrap(), b"7");
}

#[test]
fn fixed_library_capacity_overflow_is_a_reserve_error() {
    // This witnesses a fixed Vec error without trying to exhaust host memory.
    // It does not exercise the private Writer's physical-allocation branch.
    let mut bytes = vec![1_u8, 2, 3];
    let before = bytes.clone();
    assert!(bytes.try_reserve_exact(usize::MAX).is_err());
    assert_eq!(bytes, before);
}

#[test]
fn unicode_and_json_escapes_are_charged_as_output_bytes() {
    let text = "é中\n\"";
    let expected = serde_json::to_vec(&text).unwrap();
    assert!(expected.len() > text.chars().count());
    for limit in 0..=expected.len() + 1 {
        match encode_bounded(&text, limit) {
            Ok(bytes) => {
                assert!(bytes.len() <= limit);
                assert_eq!(bytes, expected);
            }
            Err(JournalError::MaintenanceCapacity) => assert!(limit < expected.len()),
            Err(error) => panic!("unexpected error at byte budget {limit}: {error:?}"),
        }
    }
}
