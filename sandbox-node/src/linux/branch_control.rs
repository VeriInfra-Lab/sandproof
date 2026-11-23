use super::{LinuxWorker, core};
use sandbox_runtime::{OriginalPauseObservation, Runtime, RuntimeProcessIdentity};
use sandproof_controller_service::application::{
    ApplicationError,
    branching::{BranchProviderReply, BranchProviderRequest},
};
use sandproof_protocol::ResourceId;
use std::time::Duration;

/// Connection factory restricted to the protected socket of one original VM.
pub struct NativeBranchControl {
    runtime: Runtime,
    vm: String,
    process: Option<RuntimeProcessIdentity>,
}

impl NativeBranchControl {
    /// The caller retains the original helper gate throughout this exchange.
    /// # Errors
    /// A timeout, replacement or failed exchange gives no outcome evidence.
    pub fn exchange(
        &self,
        request: BranchProviderRequest<'_>,
    ) -> Result<BranchProviderReply, ApplicationError> {
        match request {
            BranchProviderRequest::Guest(bytes) => {
                let process = self
                    .process
                    .as_ref()
                    .ok_or(ApplicationError::OutcomeUnknown)?;
                self.runtime
                    .exchange_hot_fanout_control(&self.vm, process, bytes, Duration::from_secs(8))
                    .map(BranchProviderReply::Guest)
                    .map_err(|_| ApplicationError::OutcomeUnknown)
            }
            BranchProviderRequest::ConfirmParentExit(expected) => {
                let expected = RuntimeProcessIdentity {
                    pid: expected.pid,
                    start_time_ticks: expected.start_time_ticks,
                    boot_id: expected.boot_id.clone(),
                };
                match self
                    .runtime
                    .observe_original_pause_state(&self.vm, &expected)
                {
                    Ok(OriginalPauseObservation::Exited) => Ok(BranchProviderReply::ParentExited),
                    _ => Err(ApplicationError::OutcomeUnknown),
                }
            }
        }
    }
}

impl LinuxWorker {
    pub(crate) fn prepare_branch_control(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
        checkpoint: &ResourceId,
        count: usize,
    ) -> Result<Option<NativeBranchControl>, ApplicationError> {
        let template = self
            .hot_fanout_template_for_generation(sandbox, generation)
            .map_err(|_| ApplicationError::OutcomeUnknown)?;
        let Some(template) = template.filter(|pin| pin.checkpoint_id == checkpoint.as_str()) else {
            return Ok(None);
        };
        if count > usize::from(template.max_branches) {
            return Err(ApplicationError::QuotaExceeded);
        }
        let runtime = self
            .runtime(sandbox, vm)
            .map_err(|_| ApplicationError::OutcomeUnknown)?;
        let vm = self.vm_name(vm);
        let process = runtime.source_process_identity(&vm).ok();
        Ok(Some(NativeBranchControl {
            runtime,
            vm,
            process,
        }))
    }
}
