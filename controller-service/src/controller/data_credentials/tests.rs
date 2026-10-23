use super::*;
use crate::{NeverCrash, finish_admitted_create};
use sandproof_protocol::{ResourceEvidence, ResourceRef, WorkAction, WorkOutcome, WorkReceipt};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

const NS: &str = "0123456789abcdef";
fn open(path: &std::path::Path) -> ControllerJournal {
    crate::controller::tests_support::open(path, NS).unwrap()
}
fn admit(journal: &mut ControllerJournal, id: u64) {
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox: core::SandboxId(id),
                workspace: core::FsId(id),
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
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Evidence {
    resource: ResourceRef,
    command: core::Command,
}
impl ResourceEvidence for Evidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}
impl crate::HostEvidence for Evidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.command == *command
    }
}
fn success(
    action: &WorkAction<core::EffectId>,
) -> Result<WorkReceipt<Evidence>, crate::WorkerTransportError> {
    action
        .identity
        .validate_version()
        .map_err(|_| crate::WorkerTransportError::ConnectionLost)?;
    Ok(WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(Evidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
        }),
    })
}
fn start(journal: &mut ControllerJournal, id: u64) {
    finish_admitted_create(
        journal,
        core::SandboxId(id),
        core::OperationId(1),
        &mut success,
    )
    .unwrap();
}
fn kill(journal: &mut ControllerJournal, id: u64) {
    crate::kill(
        journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox: core::SandboxId(id),
            operation: core::OperationId(2),
        },
        &mut success,
    )
    .unwrap();
}
fn release_retire(journal: &mut ControllerJournal, id: u64) {
    let context = journal
        .host_slot_context(core::SandboxId(id))
        .unwrap()
        .unwrap();
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    journal
        .retire_reclaimed(core::SandboxId(id), 1001, |_| Ok(()), &mut NeverCrash)
        .unwrap();
}
struct FailAt(CrashPoint);
impl CrashInjector for FailAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}
fn register(journal: &mut ControllerJournal, id: u64, digest: [u8; 32]) {
    journal
        .register_data_credential(core::SandboxId(id), 1, digest, &mut NeverCrash)
        .unwrap();
}

#[test]
fn exact_registration_is_immutable_idempotent_and_survives_current_restart() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    admit(&mut journal, 1);
    register(&mut journal, 1, [1; 32]);
    start(&mut journal, 1);
    let world = journal.world().unwrap();
    let progress = journal.checkpoint_progress().unwrap();
    register(&mut journal, 1, [1; 32]);
    assert_eq!(journal.world().unwrap(), world);
    assert_eq!(
        journal
            .checkpoint_progress()
            .unwrap()
            .unwrap()
            .input_high_water,
        progress.unwrap().input_high_water
    );
    assert_eq!(
        journal.lookup_data_credential(&[1; 32]).unwrap(),
        Some((core::SandboxId(1), 1))
    );
    assert_eq!(journal.lookup_data_credential(&[2; 32]).unwrap(), None);
    drop(journal);
    let journal = open(&path);
    assert_eq!(
        journal.data_credential(core::SandboxId(1)).unwrap(),
        Some((1, [1; 32]))
    );
    assert_eq!(
        journal.lookup_data_credential(&[1; 32]).unwrap(),
        Some((core::SandboxId(1), 1))
    );
}

#[test]
fn physical_identity_read_preserves_poison_without_authorizing_data() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    for id in 1..=2 {
        admit(&mut journal, id);
        register(&mut journal, id, [u8::try_from(id).unwrap(); 32]);
        start(&mut journal, id);
    }
    assert!(matches!(
        journal.register_data_credential(core::SandboxId(1), 1, [3; 32], &mut NeverCrash),
        Err(JournalError::CredentialIndexPoisoned)
    ));
    let world = journal.world().unwrap();
    for restart in [false, true] {
        if restart {
            drop(journal);
            journal = open(&path);
        }
        for id in 1..=2 {
            assert_eq!(
                journal
                    .original_data_credential(core::SandboxId(id))
                    .unwrap(),
                Some((1, [u8::try_from(id).unwrap(); 32]))
            );
            assert!(matches!(
                journal.data_credential(core::SandboxId(id)),
                Err(JournalError::CredentialIndexPoisoned)
            ));
        }
        assert!(matches!(
            journal.lookup_data_credential(&[1; 32]),
            Err(JournalError::CredentialIndexPoisoned)
        ));
        assert_eq!(journal.world().unwrap(), world);
    }
}

#[test]
fn physical_identity_read_rejects_an_unrelated_corrupt_binding() {
    let dir = TempDir::new().unwrap();
    let mut journal = open(&dir.path().join("db"));
    for id in 1..=2 {
        admit(&mut journal, id);
        register(&mut journal, id, [u8::try_from(id).unwrap(); 32]);
    }
    // Fault the shared index directly, not a target host descriptor. Even a
    // read-only inventory of object 1 must reject noncanonical object 2.
    journal
        .journal
        .connection
        .execute(
            "UPDATE controller_data_credentials SET generation='01' WHERE sandbox_id=?1",
            [format!("sb-{NS}-2")],
        )
        .unwrap();
    assert!(matches!(
        journal.original_data_credential(core::SandboxId(1)),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn missing_running_registration_rejects_data_auth_without_blocking_management() {
    let dir = TempDir::new().unwrap();
    let mut journal = open(&dir.path().join("db"));
    admit(&mut journal, 1);
    register(&mut journal, 1, [1; 32]);
    start(&mut journal, 1);
    admit(&mut journal, 2);
    start(&mut journal, 2);
    assert!(matches!(
        journal.lookup_data_credential(&[1; 32]),
        Err(JournalError::CorruptStore)
    ));
    assert_eq!(journal.data_credential(core::SandboxId(2)).unwrap(), None);
    kill(&mut journal, 2);
    assert_eq!(
        journal.lookup_data_credential(&[1; 32]).unwrap(),
        Some((core::SandboxId(1), 1))
    );
}

#[test]
fn cross_object_duplicate_poisons_durably_but_cleanup_and_admission_continue() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    admit(&mut journal, 1);
    register(&mut journal, 1, [1; 32]);
    start(&mut journal, 1);
    admit(&mut journal, 2);
    assert!(matches!(
        journal.register_data_credential(core::SandboxId(2), 1, [1; 32], &mut NeverCrash),
        Err(JournalError::CredentialIndexPoisoned)
    ));
    assert!(matches!(
        journal.lookup_data_credential(&[1; 32]),
        Err(JournalError::CredentialIndexPoisoned)
    ));
    drop(journal);
    let mut journal = open(&path);
    assert!(matches!(
        journal.lookup_data_credential(&[1; 32]),
        Err(JournalError::CredentialIndexPoisoned)
    ));
    kill(&mut journal, 1);
    release_retire(&mut journal, 1);
    kill(&mut journal, 2);
    release_retire(&mut journal, 2);
    assert!(journal.world().unwrap().sandbox_records().is_empty());
    assert!(matches!(
        journal.lookup_data_credential(&[1; 32]),
        Err(JournalError::CredentialIndexPoisoned)
    ));
    admit(&mut journal, 3);
    assert!(matches!(
        journal.register_data_credential(core::SandboxId(3), 1, [3; 32], &mut NeverCrash),
        Err(JournalError::CredentialIndexPoisoned)
    ));
}

#[test]
fn existing_object_cannot_rotate_its_registered_digest() {
    let dir = TempDir::new().unwrap();
    let mut journal = open(&dir.path().join("db"));
    admit(&mut journal, 1);
    register(&mut journal, 1, [1; 32]);
    assert!(matches!(
        journal.register_data_credential(core::SandboxId(1), 1, [2; 32], &mut NeverCrash),
        Err(JournalError::CredentialIndexPoisoned)
    ));
    let digest: Vec<u8> = journal
        .journal
        .connection
        .query_row(
            "SELECT digest FROM controller_data_credentials",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(digest, [1; 32]);
    assert!(matches!(
        journal.lookup_data_credential(&[1; 32]),
        Err(JournalError::CredentialIndexPoisoned)
    ));
}

#[test]
fn registration_faults_and_lost_commit_reply_reconcile_both_actual_results() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        admit(&mut journal, 1);
        super::super::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.register_data_credential(core::SandboxId(1), 1, [1; 32], &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        assert_eq!(
            journal.data_credential(core::SandboxId(1)).unwrap(),
            durable.then_some((1, [1; 32]))
        );
        drop(journal);
        let mut journal = open(&path);
        register(&mut journal, 1, [1; 32]);
        assert_eq!(
            journal.data_credential(core::SandboxId(1)).unwrap(),
            Some((1, [1; 32]))
        );
    }
    for (point, durable) in [
        (CrashPoint::BeforeCommit, false),
        (CrashPoint::AfterCommit, true),
    ] {
        let dir = TempDir::new().unwrap();
        let mut journal = open(&dir.path().join("db"));
        admit(&mut journal, 1);
        assert!(
            journal
                .register_data_credential(core::SandboxId(1), 1, [1; 32], &mut FailAt(point))
                .is_err()
        );
        assert_eq!(
            journal.data_credential(core::SandboxId(1)).unwrap(),
            durable.then_some((1, [1; 32]))
        );
    }
}

#[test]
fn lost_poison_reply_never_assumes_a_rollback_or_clears_a_durable_fence() {
    for durable in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        admit(&mut journal, 1);
        register(&mut journal, 1, [1; 32]);
        admit(&mut journal, 2);
        super::super::transaction::fail_next_commit(durable);
        assert!(matches!(
            journal.register_data_credential(core::SandboxId(2), 1, [1; 32], &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        drop(journal);
        let mut journal = open(&path);
        if durable {
            assert!(matches!(
                journal.lookup_data_credential(&[1; 32]),
                Err(JournalError::CredentialIndexPoisoned)
            ));
        } else {
            assert_eq!(
                journal.lookup_data_credential(&[1; 32]).unwrap(),
                Some((core::SandboxId(1), 1))
            );
        }
        assert!(matches!(
            journal.register_data_credential(core::SandboxId(2), 1, [1; 32], &mut NeverCrash),
            Err(JournalError::CredentialIndexPoisoned)
        ));
    }
}

#[test]
fn invalid_target_or_generation_never_creates_a_registration() {
    let dir = TempDir::new().unwrap();
    let mut journal = open(&dir.path().join("db"));
    assert!(matches!(
        journal.register_data_credential(core::SandboxId(1), 1, [1; 32], &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    admit(&mut journal, 1);
    assert!(matches!(
        journal.register_data_credential(core::SandboxId(1), u64::MAX, [1; 32], &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    start(&mut journal, 1);
    kill(&mut journal, 1);
    assert!(matches!(
        journal.register_data_credential(core::SandboxId(1), 1, [1; 32], &mut NeverCrash),
        Err(JournalError::Conflict)
    ));
    assert_eq!(journal.data_credential(core::SandboxId(1)).unwrap(), None);
    assert_eq!(generation(&u64::MAX.to_string()).unwrap(), u64::MAX);
    for malformed in ["01", "+1", "-1", "18446744073709551616"] {
        assert!(generation(malformed).is_err());
    }
}

#[test]
fn index_retirement_is_exact_atomic_and_does_not_forget_other_live_bindings() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let mut journal = open(&path);
    for id in [1, 2] {
        admit(&mut journal, id);
        register(&mut journal, id, [u8::try_from(id).unwrap(); 32]);
        start(&mut journal, id);
    }
    kill(&mut journal, 1);
    let context = journal
        .host_slot_context(core::SandboxId(1))
        .unwrap()
        .unwrap();
    journal
        .release_host_slot(context, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    assert!(
        journal
            .retire_reclaimed(
                core::SandboxId(1),
                1001,
                |_| Ok(()),
                &mut FailAt(CrashPoint::AfterRetirementDelete)
            )
            .is_err()
    );
    assert_eq!(
        journal.data_credential(core::SandboxId(1)).unwrap(),
        Some((1, [1; 32]))
    );
    journal
        .retire_reclaimed(core::SandboxId(1), 1001, |_| Ok(()), &mut NeverCrash)
        .unwrap();
    assert_eq!(journal.data_credential(core::SandboxId(1)).unwrap(), None);
    assert_eq!(journal.lookup_data_credential(&[1; 32]).unwrap(), None);
    assert_eq!(
        journal.lookup_data_credential(&[2; 32]).unwrap(),
        Some((core::SandboxId(2), 1))
    );
    drop(journal);
    let journal = open(&path);
    assert_eq!(
        journal.lookup_data_credential(&[2; 32]).unwrap(),
        Some((core::SandboxId(2), 1))
    );
}

#[test]
fn missing_or_malformed_shared_index_is_rejected_instead_of_repaired() {
    for mutation in [
        "DROP TABLE controller_data_credentials",
        "DELETE FROM controller_data_credential_state",
        "PRAGMA ignore_check_constraints=ON;UPDATE controller_data_credential_state SET id=2",
        "UPDATE controller_data_credentials SET generation='01'",
        "PRAGMA ignore_check_constraints=ON;UPDATE controller_data_credentials SET digest=x'01'",
        "ALTER TABLE controller_data_credentials RENAME TO old_data_credentials;CREATE TABLE controller_data_credentials(sandbox_id TEXT,generation TEXT,digest BLOB)",
    ] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("db");
        let mut journal = open(&path);
        admit(&mut journal, 1);
        register(&mut journal, 1, [1; 32]);
        journal.journal.connection.execute_batch(mutation).unwrap();
        assert!(
            journal.lookup_data_credential(&[1; 32]).is_err(),
            "{mutation}"
        );
        drop(journal);
        assert!(
            crate::controller::tests_support::open(&path, NS).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn previous_complete_format_is_not_upgraded_to_add_credentials() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("db");
    let journal = open(&path);
    journal.journal.connection.execute_batch("DROP TABLE controller_data_credentials;DROP TABLE controller_data_credential_state;PRAGMA user_version=4;").unwrap();
    drop(journal);
    let before = std::fs::read(&path).unwrap();
    assert!(crate::controller::tests_support::open(&path, NS).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
