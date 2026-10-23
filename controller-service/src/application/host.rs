//! Host capabilities use application errors, never HTTP types.
use super::ApplicationError;
use crate::{
    CreateOptions, HostEvidence, HostSlotContext, WorkerTransportError,
    template_registry::CandidateVerification, templates::TemplateBinding,
};
use sandbox_controller_core as core;
use sandproof_protocol::{WorkAction, WorkReceipt};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;

/// Shared receipt evidence type; this grants neither execution nor observation.
pub trait WorkEvidence {
    type Evidence: HostEvidence + Serialize + DeserializeOwned + PartialEq + Send;
}

/// Execute only controller-claimed work; transport loss remains unknown.
pub trait WorkExecution: WorkEvidence {
    /// # Errors
    /// Transport loss must remain unknown, never an implicit retry.
    fn execute(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError>;
}

/// Observe or reconcile exact previously dispatched work, never arbitrary replay.
pub trait WorkRecovery: WorkEvidence {
    /// Observe an already dispatched action without performing it again.
    /// `None` means the physical outcome is still unknown and must stay so.
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>>;

    /// Reclaim an incomplete, already dispatched `PrepareVm` only after the
    /// controller has matched it to the durable owner and resource plan.
    /// A live or unobservable VM must remain untouched. The adapter records
    /// exact physical evidence before `observe` may return a final failure.
    /// # Errors
    /// Ambiguous or foreign resources remain unresolved; no retry follows.
    fn reclaim_incomplete_prepare(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError>;

    /// Reconcile an exact current network action without assuming a lost
    /// acknowledgement means no host effect occurred.
    /// # Errors
    /// Ambiguous or foreign network resources remain unresolved.
    fn reclaim_incomplete_network(
        &self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Repeat only the identical, already-dispatched guest initialization
    /// after the controller has matched its current create and sealed VM plan.
    /// The host must use the persisted credential/configuration and verify
    /// the same running VM before and after the request.
    /// # Errors
    /// An unreadable guest, changed VM or conflicting configuration stays unknown.
    fn reclaim_incomplete_activation(
        &self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Complete only the current operation's terminal fence after a lost
    /// acknowledgement; it may not reopen an older execution gate.
    /// # Errors
    /// Ambiguous or stale fence state remains unresolved.
    fn reclaim_incomplete_terminal_fence(
        &self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Complete the original opening fence only while the controller's
    /// matching create is current. The host must reject a terminal-to-open
    /// transition even if its earlier receipt was lost.
    /// # Errors
    /// A stale or unobservable fence remains unresolved.
    fn reclaim_incomplete_open_fence(
        &self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Finish an already-dispatched stop for the current terminal operation.
    /// This may signal only the exact recorded process; it is not permission
    /// to re-dispatch arbitrary uncertain host commands.
    /// # Errors
    /// An unverified process or incomplete stop remains unresolved.
    fn reclaim_incomplete_stop(
        &self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Resume only a dispatched, plan-matched `AbortPrepare` under the
    /// terminal fence. The host verifies the original process identity.
    /// # Errors
    /// An ambiguous or live resource remains unresolved.
    fn reclaim_incomplete_abort_prepare(
        &self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Finish exact, already-dispatched cleanup after proving the recorded
    /// VM is stopped. This may remove only resources owned by that VM.
    /// # Errors
    /// Ambiguous or live resources remain unresolved.
    fn reclaim_incomplete_cleanup(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError>;
}

/// Bind and release exact durable physical positions.
pub trait HostSlots {
    /// Persist or validate the exact controller-committed physical position.
    /// A missing/prior binding may be installed only before any host dispatch,
    /// or for terminal pre-launch cleanup whose only dispatched action was a
    /// terminal fence. No potentially launched VM may acquire a new binding.
    /// # Errors
    /// Unsupported bounded mode or ambiguous identity must fail closed.
    fn bind_host_slot(
        &self,
        _context: &HostSlotContext,
        _allow_install: bool,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Complete the durable exact release and prove physical absence. Called
    /// only after the controller committed Releasing and local routes drained.
    /// # Errors
    /// Unknown cleanup or stale identity keeps the physical position unavailable.
    fn release_host_slot(&self, _context: &HostSlotContext) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }
}

/// Own gated data connections and their exact drain observations.
pub trait DataRelay {
    /// Acquire a shared durable host-helper gate before a data relay can be
    /// spawned. The descriptor must retain its lock across explicit child FD
    /// inheritance; dropping it must never issue an explicit unlock.
    /// # Errors
    /// A changed binding or unavailable gate refuses the route.
    fn acquire_data_gate(
        &self,
        _sandbox: core::SandboxId,
        _vm: core::VmId,
        _generation: core::Generation,
    ) -> Result<Option<std::fs::File>, ApplicationError> {
        Ok(None)
    }

    /// Reserve a bounded durable route cell and record the exact child process
    /// before releasing its start gate. The program is selected by the trusted
    /// adapter, never by the HTTP caller.
    /// # Errors
    /// Capacity or uncertain child identity refuses the new relay.
    fn spawn_data_relay(
        &self,
        _sandbox: core::SandboxId,
        _vm: core::VmId,
        _generation: core::Generation,
        _control: bool,
    ) -> Result<std::process::Child, ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Resolve an earlier unknown relay exit after every local pin has dropped.
    /// # Errors
    /// Refuses changed identity; false leaves the position unavailable.
    fn data_gate_drained(&self, _context: &HostSlotContext) -> Result<bool, ApplicationError> {
        Ok(false)
    }
}

/// Observe deployment storage and network readiness before admission.
pub trait HostCapacity {
    /// Read only the exact source using bounded GET and file reads. No pause,
    /// capture, stop, repair, permission change or replacement process lookup.
    /// Returned raw bytes are not a full compatibility or resource certificate.
    /// # Errors
    /// Any missing/changed/unknown fact leaves the original work unresolved.
    fn observe_lifecycle_source(
        &self,
        _requirement: &crate::capture::SourceObservationRequirement,
    ) -> Result<crate::capture::RawSourceObservation, WorkerTransportError> {
        Err(WorkerTransportError::ConnectionLost)
    }

    /// Read the original protected registration and actual current host. This
    /// runs outside any SQL transaction; echoed fields are not an observation.
    /// # Errors
    /// Missing/unverifiable payload or changed host must fail closed.
    fn observe_host_epoch(
        &self,
        _requirement: &crate::capture::HostEpochRequirement,
    ) -> Result<crate::capture::HostEpochObservation, WorkerTransportError> {
        Err(WorkerTransportError::ConnectionLost)
    }

    /// # Errors
    /// Rejects an unavailable physical disk observation.
    fn available_disk_bytes(&self) -> Result<u64, ApplicationError>;

    /// Fresh trusted host CPU and available memory observation. Diagnostic
    /// adapters without this capability explicitly refuse node-health reports.
    /// # Errors
    /// Missing or malformed host observations do not become free capacity.
    fn available_compute(&self) -> Result<(u64, u64), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Validate the immutable deployment-wide activation lease and current
    /// inode capacity. A local free-space observation alone is insufficient.
    /// # Errors
    /// Adapters without a validated shared profile must refuse full mode.
    fn validate_physical_storage(
        &self,
        _profile: &crate::physical_storage::PhysicalStorageProfile,
        _pool: &crate::physical_storage::PhysicalPool,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    /// Read-only host readiness check before admitting a public IPv4 VM.
    /// # Errors
    /// Treat an unreadable forwarding state as unavailable, not permitted.
    fn public_ipv4_ready(&self) -> Result<bool, ApplicationError> {
        Ok(false)
    }
}

/// Persist and verify the admitted guest configuration and original credential.
pub trait LaunchConfiguration {
    /// # Errors
    /// A host credential creation failure leaves the admitted create pending.
    fn put_secret(&self, sandbox: core::SandboxId, token: &str)
    -> Result<(), ApplicationError>;

    /// Return false only when no credential was ever stored. A malformed or
    /// unreadable credential must be an error, not permission to replace it.
    /// # Errors
    /// Refuses to classify an untrusted or corrupt host credential as absent.
    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, ApplicationError>;

    /// # Errors
    /// Missing credentials cannot be replaced by a new token on connect.
    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, ApplicationError>;

    /// Persist the exact guest initialization environment with its credential
    /// before a VM can be reserved. Hosts without this capability reject
    /// nonempty input instead of silently dropping it.
    /// # Errors
    /// Any uncertain credential write blocks launch until observation.
    fn put_launch_config(
        &self,
        sandbox: core::SandboxId,
        token: &str,
        env_vars: &BTreeMap<String, String>,
    ) -> Result<(), ApplicationError> {
        if !env_vars.is_empty() {
            return Err(ApplicationError::Invalid);
        }
        self.put_secret(sandbox, token)
    }

    /// A present credential with different initialization data is not a
    /// valid recovery candidate for the admitted create.
    /// # Errors
    /// Rejects an unreadable or mismatched launch configuration.
    fn has_launch_config(
        &self,
        sandbox: core::SandboxId,
        env_vars: &BTreeMap<String, String>,
    ) -> Result<bool, ApplicationError> {
        if !env_vars.is_empty() {
            return Err(ApplicationError::Invalid);
        }
        self.has_secret(sandbox)
    }

    /// Persist the exact admitted template alongside the one-time launch credential.
    /// # Errors
    /// Unknown writes stay pending; no replacement template is permitted.
    fn put_bound_launch_config(
        &self,
        sandbox: core::SandboxId,
        token: &str,
        options: &CreateOptions,
    ) -> Result<(), ApplicationError> {
        if options.template.is_some() {
            return Err(ApplicationError::Invalid);
        }
        self.put_launch_config(sandbox, token, &options.env_vars)
    }

    /// # Errors
    /// Recovery must match the full persisted descriptor, not resolve its alias again.
    fn has_bound_launch_config(
        &self,
        sandbox: core::SandboxId,
        options: &CreateOptions,
    ) -> Result<bool, ApplicationError> {
        if options.template.is_some() {
            return Err(ApplicationError::Invalid);
        }
        self.has_launch_config(sandbox, &options.env_vars)
    }
}

/// Validate immutable template artifacts; independent of lifecycle execution.
pub trait TemplateArtifacts {
    /// Validate trusted template artifacts before accepting any create intent.
    /// # Errors
    /// An adapter without multi-template support must reject, not use base.
    fn validate_template(&self, _template: &TemplateBinding) -> Result<(), ApplicationError> {
        Err(ApplicationError::Invalid)
    }

    /// Optionally reuse this process's registry hash checks after sealing.
    /// This is only a cache hint; `validate_template` remains mandatory.
    fn cache_candidate_verification(
        &self,
        _template: &TemplateBinding,
        _verification: &CandidateVerification,
    ) {
    }
}

/// Assembly constraint: one shared host, with short internal metadata locks.
/// Individual helpers depend on their actual capability, not this aggregate.
pub trait ControllerHost:
    WorkExecution
    + WorkRecovery
    + HostSlots
    + DataRelay
    + HostCapacity
    + LaunchConfiguration
    + TemplateArtifacts
    + Send
    + Sync
    + 'static
{
}
impl<T> ControllerHost for T where
    T: WorkExecution
        + WorkRecovery
        + HostSlots
        + DataRelay
        + HostCapacity
        + LaunchConfiguration
        + TemplateArtifacts
        + Send
        + Sync
    + Sync
        + 'static
{
}
