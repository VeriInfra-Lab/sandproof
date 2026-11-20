//! Terminal-only original source/worker/descriptor observation. No VM command,
//! Full, Continue, material deletion or quota settlement is performed here.
//! The exact helper protocol may retire its own already-dead marker metadata.
use super::{LinuxWorker, NodeResult, WorkAction, check_trusted_path, core};
use crate::ProtectedHostEpochSource;
use sandbox_runtime::RuntimeProcessIdentity;
use sandproof_controller_service::capture::{
    CaptureWorkerBinding, TerminatedCaptureObservation, TerminatedCaptureWorkRequest,
};
use std::os::unix::fs::MetadataExt;

fn require_terminal_pair(
    namespace: &str,
    identity: core::capture::OriginalContinueIdentity,
    seal: &WorkAction<core::EffectId>,
    stop: &WorkAction<core::EffectId>,
    coverage: core::StopCoverage,
) -> NodeResult<()> {
    super::decisions::validate_action_identity(seal, namespace)?;
    super::decisions::validate_action_identity(stop, namespace)?;
    let core::Command::SealSnapshot {
        sandbox,
        epoch,
        generation,
        vm,
        ..
    } = seal.command.command()
    else {
        return Err("terminal observation lacks original Seal action".into());
    };
    let expected_stop = core::Command::StopVm {
        sandbox: *sandbox,
        epoch: coverage.epoch,
        generation: *generation,
        vm: *vm,
    };
    if *sandbox != identity.key.sandbox
        || *epoch != identity.epoch
        || *generation != identity.generation
        || *vm != identity.vm
        || seal.identity.operation.as_str()
            != format!("op-{namespace}-{}-{}", sandbox.0, identity.key.operation.0)
        || *stop.command.command() != expected_stop
        || coverage.epoch <= identity.epoch
        || coverage.operation.sandbox != *sandbox
        || coverage.operation == identity.key
        || stop.command.attempt != coverage.attempt
        || stop.identity.operation.as_str()
            != format!(
                "op-{namespace}-{}-{}",
                sandbox.0, coverage.operation.operation.0
            )
        || seal.identity.owner != stop.identity.owner
    {
        return Err("terminal observation differs from original capture/terminal Stop".into());
    }
    Ok(())
}

fn require_worker_ended(
    worker: Option<&CaptureWorkerBinding>,
    source_boot: &str,
) -> NodeResult<()> {
    if let Some(worker) = worker {
        let expected = RuntimeProcessIdentity {
            pid: worker.process().pid,
            start_time_ticks: worker.process().start_time_ticks,
            boot_id: worker.process().boot_id.clone(),
        };
        expected.validate_fields()?;
        if !worker.valid() || expected.boot_id != source_boot || expected.is_alive()? {
            return Err("original capture worker has not been observed exited".into());
        }
    }
    // None is only the journal's never-authorized question. It supplies no
    // physical fact: the caller still must obtain the original exclusive gate.
    Ok(())
}
struct TerminalContext<'a> {
    work: &'a TerminatedCaptureWorkRequest,
    provider: &'a ProtectedHostEpochSource,
    original: sandproof_controller_service::HostSlotContext,
    runtime: sandbox_runtime::Runtime,
    source: RuntimeProcessIdentity,
}
impl LinuxWorker {
    pub(crate) fn observe_terminated_capture(
        &self,
        provider: &ProtectedHostEpochSource,
        work: &TerminatedCaptureWorkRequest,
    ) -> NodeResult<TerminatedCaptureObservation> {
        let context = self.terminal_context(provider, work)?;
        self.check_terminal_context(&context)?;
        let id = work.identity();
        let original = context.original;
        let slot = self.runtime_host_slot(original.position, id.vm)?;
        // The repaired Full exec gate retains its own fd3 through manager
        // cancellation. Exclusive acquisition must fail while any such owner
        // or tracked route lives. Do not take this gate before stopping the VM.
        let gate = slot.helper_gate.try_acquire(Some(&slot))?;
        let held = gate.into_file();
        let metadata = held.metadata()?;
        if metadata.dev() != slot.helper_gate.device || metadata.ino() != slot.helper_gate.inode {
            return Err("terminal drain opened a different original gate".into());
        }
        self.check_terminal_context(&context)?;
        check_trusted_path(&slot.helper_gate.path)?;
        let current = std::fs::symlink_metadata(&slot.helper_gate.path)?;
        if !current.is_file()
            || current.file_type().is_symlink()
            || (
                current.dev(),
                current.ino(),
                current.uid(),
                current.gid(),
                current.mode(),
                current.nlink(),
            ) != (
                metadata.dev(),
                metadata.ino(),
                metadata.uid(),
                metadata.gid(),
                metadata.mode(),
                metadata.nlink(),
            )
        {
            return Err("terminal drain original gate changed during observation".into());
        }
        Ok(TerminatedCaptureObservation::StoppedAndDrained {
            work_binding: work.binding_digest()?,
            full_work_binding: work.full().binding_digest()?,
            seal_action: work.seal_action().clone(),
            stop_action: work.stop_action().clone(),
            source: work.source().source.clone(),
            worker: work.worker().cloned(),
            host_slot: original,
            helper_gate_device: metadata.dev(),
            helper_gate_inode: metadata.ino(),
        })
    }

    fn terminal_context<'a>(
        &self,
        provider: &'a ProtectedHostEpochSource,
        work: &'a TerminatedCaptureWorkRequest,
    ) -> NodeResult<TerminalContext<'a>> {
        let id = work.identity();
        require_terminal_pair(
            &self.config.namespace,
            id,
            work.seal_action(),
            work.stop_action(),
            work.stop(),
        )?;
        self.validate_action(work.seal_action())?;
        self.validate_action(work.stop_action())?;
        super::capture::require_source_binding(&work.full().pause, work.source())?;
        if !work.full().valid() || work.source().resource != work.stop_action().identity.resource {
            return Err("terminal drain Full/source resource differs".into());
        }
        let original = self.original_source_context(&work.source().resource)?;
        if original != work.host_slot()
            || original.binding.sandbox != id.key.sandbox
            || original.binding.generation != id.generation
            || original.binding.vm != id.vm
        {
            return Err("terminal drain original occupied slot differs".into());
        }
        Ok(TerminalContext {
            work,
            provider,
            original,
            runtime: self.runtime(id.key.sandbox, id.vm)?,
            source: crate::source_observation::expected_source(work.source())?,
        })
    }

    fn check_terminal_context(&self, context: &TerminalContext<'_>) -> NodeResult<()> {
        let work = context.work;
        let id = work.identity();
        self.require_fence(id.key.sandbox, work.stop().epoch, true)?;
        self.validate_action(work.seal_action())?;
        self.validate_action(work.stop_action())?;
        if self.original_source_context(&work.source().resource)? != context.original {
            return Err("terminal drain occupied binding changed during observation".into());
        }
        context.provider.observe_original(
            &work.source().node,
            &work.source().host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        context
            .runtime
            .observe_original_stopped(work.source().resource.id.as_str(), &context.source)?;
        require_worker_ended(work.worker(), &context.source.boot_id)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "terminated_capture_tests.rs"]
mod tests;
