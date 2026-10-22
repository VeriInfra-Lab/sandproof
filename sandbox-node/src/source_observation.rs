//! Consumes an original persisted question; collects raw facts, never permits.
use crate::{LinuxWorker, ProtectedHostEpochSource};
use sandbox_runtime::{RuntimeProcessIdentity, SourceConfigurationLimits};
use sandproof_controller_service::HostSlotContext;
use sandproof_controller_service::capture::{
    RawSourceObservation, SourceObservationConfig, SourceObservationRequirement,
};
use std::error::Error;

pub(super) type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(super) fn observe(
    worker: &LinuxWorker,
    protected: Option<&ProtectedHostEpochSource>,
    requirement: &SourceObservationRequirement,
) -> Result<RawSourceObservation> {
    checked_collection(
        requirement,
        || {
            protected
                .ok_or("original protected bootstrap source unavailable")?
                .observe_original(
                    &requirement.node,
                    &requirement.host_bootstrap,
                    worker.host_bootstrap_inputs(),
                )?;
            Ok(())
        },
        || worker.collect_original_source(requirement),
    )
}

fn checked_collection(
    requirement: &SourceObservationRequirement,
    mut bootstrap: impl FnMut() -> Result<()>,
    collect: impl FnOnce() -> Result<RawSourceObservation>,
) -> Result<RawSourceObservation> {
    runtime_limits(&requirement.limits)?;
    expected_source(requirement)?;
    bootstrap()?;
    let raw = collect()?;
    if !raw.matches(requirement) {
        return Err("original source observation does not match persisted question".into());
    }
    // Recheck the same registered input, never upgrade a changed epoch. Equal
    // samples still rely on the deployment's original writer/epoch conditions.
    bootstrap()?;
    Ok(raw)
}

pub(super) fn runtime_limits(
    limits: &SourceObservationConfig,
) -> Result<SourceConfigurationLimits> {
    if !limits.valid() {
        return Err("invalid original source read limits".into());
    }
    Ok(SourceConfigurationLimits {
        executable_bytes: limits.max_executable_bytes,
        kernel_bytes: limits.max_kernel_bytes,
        disk_bytes: limits.max_disk_bytes,
        configuration_bytes: usize::try_from(limits.max_configuration_bytes)?,
    })
}

pub(super) fn expected_source(
    requirement: &SourceObservationRequirement,
) -> Result<RuntimeProcessIdentity> {
    let source = RuntimeProcessIdentity {
        pid: requirement.source.pid,
        start_time_ticks: requirement.source.start_time_ticks,
        boot_id: requirement.source.boot_id.clone(),
    };
    source.validate_fields()?;
    if source.boot_id != requirement.host_bootstrap.boot_id {
        return Err("original source boot differs from registered bootstrap".into());
    }
    Ok(source)
}

pub(super) fn checked_context(
    expected: &HostSlotContext,
    mut current: impl FnMut() -> Result<HostSlotContext>,
    collect: impl FnOnce() -> Result<RawSourceObservation>,
) -> Result<RawSourceObservation> {
    if current()? != *expected {
        return Err("original occupied context changed before source read".into());
    }
    let observed = collect()?;
    if current()? != *expected {
        return Err("original occupied context changed during source read".into());
    }
    Ok(observed)
}

#[cfg(test)]
mod tests;
