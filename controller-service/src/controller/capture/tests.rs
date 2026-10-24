use super::*;
use crate::{NeverCrash, finish_admitted_create};
use sandproof_protocol::{ResourceEvidence, ResourceRef};
use serde::{Deserialize, Serialize};

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
