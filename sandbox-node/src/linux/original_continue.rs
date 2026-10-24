//! Continue only the original VM after the journal has durably ended capture.
//! The saved end digest names that service fact; it is no physical observation.
//! This path never loads a snapshot, initializes a guest or replaces credentials.
use super::{HostObservation, LinuxWorker, NodeResult, ObservedResult, WorkAction, core};
use crate::ProtectedHostEpochSource;
use sandbox_runtime::{OriginalPauseObservation, Runtime, RuntimeProcessIdentity};
use sandproof_controller_service::capture::{OriginalContinuePermit, OriginalContinueWorkRequest};
use sandproof_protocol::ResourceRef;

fn require_original_activation(
    action: &WorkAction<core::EffectId>,
    identity: core::capture::OriginalContinueIdentity,
    resource: &ResourceRef,
    namespace: &str,
) -> NodeResult<()> {
    super::decisions::validate_action_identity(action, namespace)?;
    if *action.command.command() != identity.activation_command()
        || action.identity.operation.as_str()
            != format!(
                "op-{namespace}-{}-{}",
                identity.key.sandbox.0, identity.key.operation.0
            )
        || &action.identity.resource != resource
    {
        return Err("continuation differs from the original Pause action/source".into());
    }
    Ok(())
}

struct ContinueContext<'a> {
    action: &'a WorkAction<core::EffectId>,
    work: &'a OriginalContinueWorkRequest,
    provider: &'a ProtectedHostEpochSource,
    runtime: Runtime,
    expected: RuntimeProcessIdentity,
    original: sandproof_controller_service::HostSlotContext,
}
impl ContinueContext<'_> {
    fn require_running(&self) -> NodeResult<()> {
        if !matches!(
            self.runtime.observe_original_pause_state(
                self.work.source().resource.id.as_str(),
                &self.expected,
            )?,
            OriginalPauseObservation::Running
        ) {
            return Err("original continuation is not observed Running".into());
        }
        Ok(())
    }
}

impl LinuxWorker {
    pub(crate) fn original_continue_action(
        &self,
        provider: &ProtectedHostEpochSource,
        action: &WorkAction<core::EffectId>,
        work: &OriginalContinueWorkRequest,
        permit: Option<&OriginalContinuePermit>,
        observe_only: bool,
    ) -> NodeResult<HostObservation> {
        let context = self.prepare_continue_context(provider, action, work)?;
        self.check_continue_context(&context)?;
        let before = crate::source_observation::observe(self, Some(provider), work.source())?;
        if !observe_only {
            self.execute_original_continue(&context, permit)?;
        }
        let result = self.confirm_original_continue(&context)?;
        let after = crate::source_observation::observe(self, Some(provider), work.source())?;
        if before != after {
            return Err("original continuation source configuration changed".into());
        }
        self.check_continue_context(&context)?;
        Ok(HostObservation {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            result,
        })
    }

    fn prepare_continue_context<'a>(
        &self,
        provider: &'a ProtectedHostEpochSource,
        action: &'a WorkAction<core::EffectId>,
        work: &'a OriginalContinueWorkRequest,
    ) -> NodeResult<ContinueContext<'a>> {
        self.validate_action(action)?;
        require_original_activation(
            action,
            work.identity(),
            &work.source().resource,
            &self.config.namespace,
        )?;
        super::capture::require_source_binding(&work.full().pause, work.source())?;
        if !work.full().valid()
            || work.full().binding_digest()?.as_bytes() != &work.end_receipt().work_binding
            || work.file_policy().digest != self.config.file_policy_digest
        {
            return Err("continuation Full/end binding or deployed file policy differs".into());
        }
        let identity = work.identity();
        Ok(ContinueContext {
            action,
            work,
            provider,
            runtime: self.runtime(identity.key.sandbox, identity.vm)?,
            expected: crate::source_observation::expected_source(work.source())?,
            original: self.original_source_context(&work.source().resource)?,
        })
    }

    fn execute_original_continue(
        &self,
        context: &ContinueContext<'_>,
        permit: Option<&OriginalContinuePermit>,
    ) -> NodeResult<()> {
        let permit = permit.ok_or("continuation lacks durable original dispatch permit")?;
        if permit.action() != context.action
            || permit.work_binding() != &context.work.binding_digest()?
        {
            return Err("continuation permit differs from the complete original work".into());
        }
        let slot = self.runtime_host_slot(context.original.position, context.work.identity().vm)?;
        // Release this exclusive gate before guest/network checks acquire their
        // own tracked shared routes. No new Full or initialization is sent.
        let _guard = slot.helper_gate.try_acquire(Some(&slot))?;
        self.check_continue_context(context)?;
        context.runtime.continue_original_vm(
            context.work.source().resource.id.as_str(),
            &context.expected,
        )?;
        Ok(())
    }

    fn confirm_original_continue(
        &self,
        context: &ContinueContext<'_>,
    ) -> NodeResult<ObservedResult> {
        let identity = context.work.identity();
        context.require_running()?;
        self.observe_network_after_candidate(
            identity.key.sandbox,
            identity.generation,
            identity.vm,
            context.original.binding.network,
            context.work.network_policy(),
        )?;
        self.observe_private_disk_after_candidate(identity.key.sandbox, identity.generation)?;
        let secret = self.secret(identity.key.sandbox, identity.generation)?;
        if self.guest_file_status(context.work.source().resource.id.as_str(), &secret.token)?
            != "200"
        {
            return Err("original continuation guest authorization not ready".into());
        }
        context.require_running()?;
        if self.secret(identity.key.sandbox, identity.generation)? != secret {
            return Err("original continuation credential changed".into());
        }
        Ok(ObservedResult::Activated {
            pid: context.expected.pid,
            start_ticks: context.expected.start_time_ticks,
            boot_id: context.expected.boot_id.clone(),
        })
    }

    fn check_continue_context(&self, context: &ContinueContext<'_>) -> NodeResult<()> {
        let id = context.work.identity();
        self.validate_action(context.action)?;
        self.require_fence(id.key.sandbox, id.epoch, false)?;
        context.provider.observe_original(
            &context.work.source().node,
            &context.work.source().host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        let original = self.original_source_context(&context.work.source().resource)?;
        if original != context.original
            || original.binding.sandbox != id.key.sandbox
            || original.binding.generation != id.generation
            || original.binding.vm != id.vm
        {
            return Err("continuation original occupied binding changed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "original_continue_tests.rs"]
mod tests;
