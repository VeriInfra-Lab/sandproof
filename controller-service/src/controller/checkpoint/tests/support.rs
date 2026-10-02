use super::*;
use crate::{AdmissionLimits, NeverCrash, finish_admitted_create_v1, kill_v1};
use crate::{HostEvidence, WorkerTransportError};
use sandbox_controller_core::maintenance::{MaintenanceLimits, windows::WindowLimits};
use sandproof_protocol::ResourceEvidence;
use sandproof_protocol::ResourceRef;
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};
use serde::Deserialize;

pub(super) const NS: &str = "0123456789abcdef";
pub(super) fn host_config(slots: u64) -> BoundedHostConfig {
    BoundedHostConfig {
        uid_base: 1000,
        uplink_base: 8,
        max_data_routes_per_slot: 4,
        max_data_routes_global: 8,
        data_route_control_reserve: 1,
        maintenance: MaintenanceLimits {
            max_host_slots: slots,
            max_retained_results: 16,
            windows: WindowLimits::new(2, 1000).unwrap(),
        },
    }
}
pub(super) fn request(id: u64) -> core::CreateRequest {
    core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(id)),
        owner: core::TenantId(id),
        sandbox: core::SandboxId(id),
        workspace: core::FsId(id),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(1),
    }
}
pub(super) fn limits() -> AdmissionLimits {
    AdmissionLimits {
        max_vm_id: 1,
        max_active_global: 4,
        max_active_per_tenant: 1,
        max_disk_bytes_global: 100_000,
        max_disk_bytes_per_tenant: 10_000,
        cleanup_reserve_bytes: 1000,
        max_timeout_secs: 600,
        max_lifetime_secs: 600,
        startup_timeout_secs: 60,
    }
}
pub(super) fn admit(
    journal: &mut ControllerJournal,
    id: u64,
    crash: &mut impl CrashInjector,
) -> Result<core::ControllerDecision, AdmissionError> {
    journal.admit_create(request(id), 1000, 60, 1000, limits(), crash)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Evidence {
    resource: ResourceRef,
    command: core::Command,
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
}
pub(super) fn success(
    action: &WorkAction<core::EffectId>,
) -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
    action
        .identity
        .validate_version()
        .map_err(|_| WorkerTransportError::ConnectionLost)?;
    Ok(WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(Evidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
        }),
    })
}
pub(super) struct FailAt(pub(super) CrashPoint);
impl CrashInjector for FailAt {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == self.0 {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}
pub(super) fn create(journal: &mut ControllerJournal, id: u64) {
    admit(journal, id, &mut NeverCrash).unwrap();
    finish_admitted_create_v1(
        journal,
        core::SandboxId(id),
        core::OperationId(1),
        &mut success,
    )
    .unwrap();
}
pub(super) fn destroy(journal: &mut ControllerJournal, id: u64) {
    kill_v1(
        journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(id)),
            sandbox: core::SandboxId(id),
            operation: core::OperationId(2),
        },
        &mut success,
    )
    .unwrap();
}

pub(super) fn publication(id: char) -> crate::template_registry::Publication {
    use crate::{
        template_registry::{Publication, PublicationState},
        templates::{ResourceShape, TemplateArtifact, TemplateBinding},
    };
    let candidate_id = id.to_string().repeat(32);
    Publication {
        binding: TemplateBinding {
            template_id: format!("tpl-{candidate_id}"),
            revision: "r1".into(),
            owner: Some(1),
            startup: None,
            kernel: TemplateArtifact {
                path: "/test/kernel".into(),
                sha256: "a".repeat(64),
                size_bytes: 1,
            },
            rootfs: TemplateArtifact {
                path: "/test/rootfs".into(),
                sha256: "b".repeat(64),
                size_bytes: 1000,
            },
            resources: ResourceShape {
                cpu_count: 1,
                memory_mib: 128,
                disk_bytes: 1000,
                host_memory_bytes: 256 * 1024 * 1024,
            },
            envd_version: "0.9.0".into(),
        },
        candidate_id,
        image_digest: format!("sha256:{}", "a".repeat(64)),
        free_bytes: 500,
        min_free_bytes: 100,
        state: PublicationState::Checking,
        checked: false,
        validation_sandbox: None,
        failure: None,
    }
}

pub(super) fn registry_config(dir: &tempfile::TempDir) -> crate::template_registry::RegistryConfig {
    crate::template_registry::RegistryConfig {
        root: dir.path().join("registry"),
        max_templates_global: 10,
        max_templates_per_tenant: 10,
        max_artifact_bytes_global: 100_000,
        max_artifact_bytes_per_tenant: 100_000,
        validation_timeout_secs: 60,
    }
}
