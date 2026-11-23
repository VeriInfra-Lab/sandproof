use super::{LinuxWorker, core};
use sandbox_runtime::{Runtime, RuntimeProcessIdentity};
use sandproof_controller_service::application::ApplicationError;
use sandproof_protocol::ResourceId;
use std::time::Duration;

/// Connection factory restricted to the protected socket of one original VM.
pub struct NativeBranchControl {
    runtime: Runtime,
    vm: String,
    process: RuntimeProcessIdentity,
}

impl NativeBranchControl {
    /// The caller retains the original helper gate throughout this exchange.
    /// # Errors
    /// A timeout, replacement or failed exchange gives no outcome evidence.
    pub fn exchange(&self, request: &[u8]) -> Result<Vec<u8>, ApplicationError> {
        self.runtime
            .exchange_hot_fanout_control(&self.vm, &self.process, request, Duration::from_secs(8))
            .map_err(|_| ApplicationError::OutcomeUnknown)
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
        let process = runtime
            .source_process_identity(&vm)
            .map_err(|_| ApplicationError::OutcomeUnknown)?;
        Ok(Some(NativeBranchControl {
            runtime,
            vm,
            process,
        }))
    }
}
