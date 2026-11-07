use super::*;
use crate::{NeverCrash, finish_admitted_create};
use sandproof_protocol::{ResourceEvidence, ResourceRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq)]
enum DecodeCall {
    Action(usize, Vec<u8>),
    Receipt(usize, Vec<u8>),
}

struct TrackedRowCodec {
    calls: std::rc::Rc<std::cell::RefCell<Vec<DecodeCall>>>,
    dropped_after: std::rc::Rc<std::cell::Cell<usize>>,
    next: usize,
}

impl OriginalRowCodec<Evidence> for TrackedRowCodec {
    fn decode_action(
        &mut self,
        bytes: &[u8],
    ) -> Result<WorkAction<core::EffectId>, serde_json::Error> {
        self.calls
            .borrow_mut()
            .push(DecodeCall::Action(self.next, bytes.to_vec()));
        self.next += 1;
        serde_json::from_slice(bytes)
    }

    fn decode_receipt(&mut self, bytes: &[u8]) -> Result<WorkReceipt<Evidence>, serde_json::Error> {
        self.calls
            .borrow_mut()
            .push(DecodeCall::Receipt(self.next, bytes.to_vec()));
        self.next += 1;
        serde_json::from_slice(bytes)
    }
}

impl Drop for TrackedRowCodec {
    fn drop(&mut self) {
        self.dropped_after.set(self.next);
    }
}

const NS: &str = "0123456789abcdef";
const SID: core::SandboxId = core::SandboxId(1);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Evidence {
    resource: ResourceRef,
    command: core::Command,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<SourceProcessIdentity>,
}
impl ResourceEvidence for Evidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}
impl HostEvidence for Evidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.command == *command
    }

    fn source_process_identity(
        &self,
        command: &core::Command,
        expected_resource: &ResourceRef,
    ) -> Option<SourceProcessIdentity> {
        if self.resource != *expected_resource || self.command != *command {
            return None;
        }
        match command {
            core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. } => {
                self.source.clone()
            }
            _ => None,
        }
    }
}

fn source() -> SourceProcessIdentity {
    SourceProcessIdentity {
        pid: 1234,
        start_time_ticks: 5678,
        boot_id: "original-host-boot".into(),
    }
}

fn running(
    path: &std::path::Path,
    prepared: Option<&SourceProcessIdentity>,
    activated: Option<&SourceProcessIdentity>,
) -> ControllerJournal {
    let mut journal = crate::controller::tests_support::open(path, NS).unwrap();
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox: SID,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            0,
            60,
            1000,
            crate::controller::tests_support::limits(),
            &mut NeverCrash,
        )
        .unwrap();
    finish_admitted_create(
        &mut journal,
        SID,
        core::OperationId(1),
        &mut |action: &WorkAction<core::EffectId>| {
            let source = match action.command.command() {
                core::Command::PrepareVm { .. } => prepared.cloned(),
                core::Command::ActivateVm { .. } => activated.cloned(),
                _ => None,
            };
            Ok(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::ObservedSuccess(Evidence {
                    resource: action.identity.resource.clone(),
                    command: action.command.command().clone(),
                    source,
                }),
            })
        },
    )
    .unwrap();
    journal
}

#[test]
fn explicit_row_codec_keeps_full_bytes_state_short_circuit_and_standard_result() {
    let dir = tempfile::tempdir().unwrap();
    let expected = source();
    let journal = running(
        &dir.path().join("controller.db"),
        Some(&expected),
        Some(&expected),
    );
    let world = journal.world().unwrap();
    let transaction = journal.journal.connection.unchecked_transaction().unwrap();
    let public = identity::sandbox_id(NS, SID).unwrap();
    let input_rows: Vec<(i64, Vec<u8>, Vec<u8>)> = {
        let mut statement = transaction
            .prepare(
                "SELECT id,action_json,receipt_json FROM outbox
                 WHERE sandbox_id=?1 AND dispatch_state='receipted' ORDER BY id",
            )
            .unwrap();
        statement
            .query_map([public.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let dropped_after = std::rc::Rc::new(std::cell::Cell::new(usize::MAX));
    let actual = original_source_using::<Evidence, _>(
        &transaction,
        NS,
        &world,
        SID,
        TrackedRowCodec {
            calls: calls.clone(),
            dropped_after: dropped_after.clone(),
            next: 0,
        },
    )
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        original_source::<Evidence>(&transaction, NS, &world, SID).unwrap(),
        actual
    );
    let mut expected_calls = Vec::new();
    let mut receipt_count = 0;
    for (_, action, receipt) in &input_rows {
        expected_calls.push(DecodeCall::Action(expected_calls.len(), action.clone()));
        let action: WorkAction<core::EffectId> = serde_json::from_slice(action).unwrap();
        if matches!(
            action.command.command(),
            core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }
        ) {
            expected_calls.push(DecodeCall::Receipt(expected_calls.len(), receipt.clone()));
            receipt_count += 1;
        }
    }
    assert_eq!(receipt_count, 2);
    assert!(input_rows.len() > receipt_count);
    assert_eq!(*calls.borrow(), expected_calls);
    assert_eq!(dropped_after.get(), expected_calls.len());

    // First action EOF must stop before its receipt and the next row, while
    // retaining the decoder mutation made before that error.
    transaction
        .execute(
            "UPDATE outbox SET action_json=?1,receipt_json=?2 WHERE id=?3",
            rusqlite::params![b"[".as_slice(), b"[]".as_slice(), input_rows[0].0],
        )
        .unwrap();
    calls.borrow_mut().clear();
    dropped_after.set(usize::MAX);
    let result = original_source_using::<Evidence, _>(
        &transaction,
        NS,
        &world,
        SID,
        TrackedRowCodec {
            calls: calls.clone(),
            dropped_after: dropped_after.clone(),
            next: 0,
        },
    );
    let Err(JournalError::Json(error)) = result else {
        panic!("expected first action JSON error")
    };
    assert!(error.is_eof());
    assert_eq!(*calls.borrow(), vec![DecodeCall::Action(0, b"[".to_vec())]);
    assert_eq!(dropped_after.get(), 1);
    let Err(JournalError::Json(error)) = original_source::<Evidence>(&transaction, NS, &world, SID)
    else {
        panic!("standard wrapper must retain the same first JSON error")
    };
    assert!(error.is_eof());
}

#[test]
fn original_source_is_read_from_real_committed_receipts_after_checkpoint_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("controller.db");
    let expected = source();
    let mut journal = running(&path, Some(&expected), Some(&expected));
    assert_eq!(
        journal.source_for_capture::<Evidence>(SID).unwrap(),
        expected
    );
    journal.publish_checkpoint(&mut NeverCrash).unwrap();
    drop(journal);
    let reopened = crate::controller::tests_support::open(&path, NS).unwrap();
    assert_eq!(
        reopened.source_for_capture::<Evidence>(SID).unwrap(),
        expected
    );
}

#[test]
fn missing_legacy_source_and_conflicting_process_or_boot_never_become_capture_source() {
    let original = source();
    let mut other_boot = original.clone();
    other_boot.boot_id = "later-host-boot".into();
    let mut other_pid = original.clone();
    other_pid.pid += 1;
    let mut other_start = original.clone();
    other_start.start_time_ticks += 1;
    let cases = [
        (None, None),
        (None, Some(original.clone())),
        (Some(original.clone()), None),
        (Some(original.clone()), Some(other_boot)),
        (Some(original.clone()), Some(other_pid)),
        (Some(original), Some(other_start)),
    ];
    for (prepared, activated) in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("controller.db");
        let journal = running(&path, prepared.as_ref(), activated.as_ref());
        let before = journal.world().unwrap();
        assert!(matches!(
            journal.source_for_capture::<Evidence>(SID),
            Err(JournalError::UnresolvedAction)
        ));
        assert_eq!(journal.world().unwrap(), before);
        drop(journal);
        let reopened = crate::controller::tests_support::open(&path, NS).unwrap();
        assert!(matches!(
            reopened.source_for_capture::<Evidence>(SID),
            Err(JournalError::UnresolvedAction)
        ));
    }
}

#[test]
fn source_observation_does_not_authorize_another_sandbox() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("controller.db");
    let original = source();
    let journal = running(&path, Some(&original), Some(&original));
    assert!(matches!(
        journal.source_for_capture::<Evidence>(core::SandboxId(2)),
        Err(JournalError::UnsupportedInput)
    ));
}

struct OriginalRowsTrace<I> {
    inner: I,
    events: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
}
impl<I: Iterator> Iterator for OriginalRowsTrace<I> {
    type Item = I::Item;
    fn next(&mut self) -> Option<Self::Item> {
        self.events.borrow_mut().push("next");
        self.inner.next()
    }
}
impl<I> Drop for OriginalRowsTrace<I> {
    fn drop(&mut self) {
        self.events.borrow_mut().push("iterator_drop");
    }
}

#[test]
fn generic_original_row_driver_keeps_order_prefix_and_stops_before_next() {
    use std::{cell::RefCell, rc::Rc};

    type Case = (
        Vec<rusqlite::Result<u8>>,
        Option<u32>,
        Option<u32>,
        u8,
        Vec<&'static str>,
    );
    let cases: Vec<Case> = vec![
        (vec![], Some(9), Some(8), 0, vec!["next", "iterator_drop"]),
        (
            vec![Ok(1), Ok(2)],
            Some(2),
            None,
            0,
            vec!["next", "accept", "next", "accept", "next", "iterator_drop"],
        ),
        (
            vec![Ok(1), Err(rusqlite::Error::InvalidQuery), Ok(2)],
            Some(1),
            None,
            1,
            vec!["next", "accept", "next", "accept", "iterator_drop"],
        ),
        (
            vec![Ok(1), Ok(3), Ok(2)],
            Some(1),
            Some(3),
            2,
            vec!["next", "accept", "next", "accept", "iterator_drop"],
        ),
        (
            vec![Err(rusqlite::Error::InvalidQuery), Ok(2)],
            None,
            None,
            1,
            vec!["next", "accept", "iterator_drop"],
        ),
    ];
    for (input, prepared, activated, expected_result, expected_events) in cases {
        let observed = |pid: Option<u32>| {
            pid.map(|pid| {
                let mut value = source();
                value.pid = pid;
                value
            })
        };
        let initial = if input.is_empty() {
            OriginalSourceRows {
                prepared: observed(Some(9)),
                activated: observed(Some(8)),
            }
        } else {
            OriginalSourceRows {
                prepared: None,
                activated: None,
            }
        };
        let events = Rc::new(RefCell::new(Vec::new()));
        let iterator = OriginalRowsTrace {
            inner: input.into_iter(),
            events: Rc::clone(&events),
        };
        let (result, state) = accept_original_rows(iterator, initial, |row, mut state| {
            events.borrow_mut().push("accept");
            let value = match row {
                Ok(value) => value,
                Err(error) => return (Err(JournalError::Sqlite(error)), state),
            };
            let mut observed = source();
            observed.pid = u32::from(value);
            if value == 3 {
                state.activated = Some(observed);
                return (Err(JournalError::UnresolvedAction), state);
            }
            state.prepared = Some(observed);
            (Ok(()), state)
        });
        let actual_result = match result {
            Ok(()) => 0,
            Err(JournalError::Sqlite(rusqlite::Error::InvalidQuery)) => 1,
            Err(JournalError::UnresolvedAction) => 2,
            other => panic!("unexpected actual driver result: {other:?}"),
        };
        assert_eq!(actual_result, expected_result);
        assert_eq!(state.prepared, observed(prepared));
        assert_eq!(state.activated, observed(activated));
        assert_eq!(*events.borrow(), expected_events);
    }
}
