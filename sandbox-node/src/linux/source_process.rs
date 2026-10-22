//! Source facts retained by the original claim and successful receipt.
use super::{
    HostObservation, LinuxWorker, NodeResult, Observation, ObservedResult, ResourceRef, core,
};
use sandbox_runtime::RuntimeProcessIdentity;

impl LinuxWorker {
    /// Legacy claims are deliberately not upgraded using the current boot.
    pub(super) fn source_boot_id(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        observed: &Observation,
    ) -> NodeResult<Option<String>> {
        let Some(source) = self
            .runtime(sandbox, vm)?
            .source_process_identity(&self.vm_name(vm))?
        else {
            return Ok(None);
        };
        if source.pid != observed.pid || source.start_time_ticks != observed.start_time_ticks {
            return Err("original VM source differs from successful observation".into());
        }
        Ok(Some(source.boot_id))
    }
}

impl HostObservation {
    /// Return original receipt facts after matching the independent action's
    /// command and resource. The caller also binds the enclosing receipt's
    /// owner/operation in its persistent ledger. This makes no future-live claim.
    #[must_use]
    pub fn recorded_source_identity(
        &self,
        command: &core::Command,
        expected_resource: &ResourceRef,
    ) -> Option<RuntimeProcessIdentity> {
        if self.resource != *expected_resource || !super::decisions::evidence_matches(self, command)
        {
            return None;
        }
        let (pid, start_time_ticks, boot_id) = match (&self.result, command) {
            (
                ObservedResult::Prepared {
                    pid,
                    start_ticks,
                    boot_id: Some(boot),
                },
                core::Command::PrepareVm { .. },
            )
            | (
                ObservedResult::Activated {
                    pid,
                    start_ticks,
                    boot_id: Some(boot),
                },
                core::Command::ActivateVm { .. },
            ) => (*pid, *start_ticks, boot.clone()),
            _ => return None,
        };
        let source = RuntimeProcessIdentity {
            pid,
            start_time_ticks,
            boot_id,
        };
        source.validate_fields().ok()?;
        Some(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandproof_controller_service::HostEvidence;
    use sandproof_protocol::{ResourceId, ResourceKind};
    use serde::Serialize;

    const BOOT: &str = "01234567-89ab-cdef-0123-456789abcdef";
    fn fixture(activated: bool) -> HostObservation {
        let command = if activated {
            core::Command::ActivateVm {
                sandbox: core::SandboxId(1),
                epoch: 2,
                generation: 3,
                vm: core::VmId(4),
            }
        } else {
            core::Command::PrepareVm {
                sandbox: core::SandboxId(1),
                epoch: 2,
                generation: 3,
                vm: core::VmId(4),
                slot: core::NetSlotId(5),
                source: None,
            }
        };
        let result = if activated {
            ObservedResult::Activated {
                pid: 71,
                start_ticks: 93,
                boot_id: Some(BOOT.into()),
            }
        } else {
            ObservedResult::Prepared {
                pid: 71,
                start_ticks: 93,
                boot_id: Some(BOOT.into()),
            }
        };
        HostObservation {
            resource: ResourceRef {
                kind: ResourceKind::Vm,
                id: ResourceId::try_from("vm-test-4").unwrap(),
            },
            command,
            result,
        }
    }

    #[test]
    fn complete_original_prepare_and_activate_source_survives_serialization() {
        for activated in [false, true] {
            let observation = fixture(activated);
            let saved = serde_json::to_vec(&observation).unwrap();
            let replayed: HostObservation = serde_json::from_slice(&saved).unwrap();
            let source = replayed
                .recorded_source_identity(&observation.command, &observation.resource)
                .unwrap();
            assert_eq!(
                source,
                RuntimeProcessIdentity {
                    pid: 71,
                    start_time_ticks: 93,
                    boot_id: BOOT.into()
                }
            );
            assert_eq!(
                replayed.source_process_identity(&observation.command, &observation.resource),
                Some(
                    sandproof_controller_service::capture::SourceProcessIdentity {
                        pid: 71,
                        start_time_ticks: 93,
                        boot_id: BOOT.into()
                    }
                )
            );
            assert_eq!(serde_json::to_vec(&replayed).unwrap(), saved);
        }
    }

    #[test]
    fn legacy_receipt_has_no_certified_source_and_preserves_its_original_bytes() {
        #[derive(Serialize)]
        enum LegacyResult {
            Prepared { pid: u32, start_ticks: u64 },
            Activated { pid: u32, start_ticks: u64 },
        }
        #[derive(Serialize)]
        struct Legacy<'a> {
            resource: &'a ResourceRef,
            command: &'a core::Command,
            result: LegacyResult,
        }
        for activated in [false, true] {
            let original = fixture(activated);
            let result = if activated {
                LegacyResult::Activated {
                    pid: 71,
                    start_ticks: 93,
                }
            } else {
                LegacyResult::Prepared {
                    pid: 71,
                    start_ticks: 93,
                }
            };
            let bytes = serde_json::to_vec(&Legacy {
                resource: &original.resource,
                command: &original.command,
                result,
            })
            .unwrap();
            let legacy: HostObservation = serde_json::from_slice(&bytes).unwrap();
            assert!(legacy.validates(&original.command));
            assert!(
                legacy
                    .recorded_source_identity(&original.command, &original.resource)
                    .is_none()
            );
            assert!(
                legacy
                    .source_process_identity(&original.command, &original.resource)
                    .is_none()
            );
            assert_eq!(serde_json::to_vec(&legacy).unwrap(), bytes);
        }
    }

    #[test]
    fn foreign_command_resource_and_malformed_source_are_refused() {
        let original = fixture(false);
        let mut command = original.command.clone();
        if let core::Command::PrepareVm { generation, .. } = &mut command {
            *generation += 1;
        }
        assert!(
            original
                .recorded_source_identity(&command, &original.resource)
                .is_none()
        );
        let mut resource = original.resource.clone();
        resource.id = ResourceId::try_from("vm-other-deployment-4").unwrap();
        assert!(
            original
                .recorded_source_identity(&original.command, &resource)
                .is_none()
        );
        assert!(
            original
                .source_process_identity(&original.command, &resource)
                .is_none()
        );
        for bad in ["", "current", "01234567-89AB-CDEF-0123-456789ABCDEF"] {
            let mut invalid = fixture(false);
            if let ObservedResult::Prepared { boot_id, .. } = &mut invalid.result {
                *boot_id = Some(bad.into());
            }
            assert!(
                invalid
                    .recorded_source_identity(&invalid.command, &invalid.resource)
                    .is_none()
            );
        }
        let mut invalid = fixture(false);
        if let ObservedResult::Prepared { pid, .. } = &mut invalid.result {
            *pid = 0;
        }
        assert!(
            invalid
                .recorded_source_identity(&invalid.command, &invalid.resource)
                .is_none()
        );
    }
}
