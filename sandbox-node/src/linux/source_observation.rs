//! Exact existing position/runtime only; no source adopted from GET JSON.
use super::{LinuxWorker, NodeResult};
use sandproof_controller_service::capture::{
    RawSourceObservation, SourceObservationRequirement, SourceProcessIdentity,
};

impl LinuxWorker {
    pub(crate) fn collect_original_source(
        &self,
        requirement: &SourceObservationRequirement,
        allowed_cpus: &[u32],
    ) -> NodeResult<RawSourceObservation> {
        let context = self.original_source_context(&requirement.resource)?;
        let expected = crate::source_observation::expected_source(requirement)?;
        let limits = crate::source_observation::runtime_limits(&requirement.limits)?;
        crate::source_observation::checked_context(
            &context,
            || self.original_source_context(&requirement.resource),
            || {
                let runtime = self.runtime(context.binding.sandbox, context.binding.vm)?;
                runtime.verify_original_cpu_domain(
                    requirement.resource.id.as_str(),
                    &expected,
                    allowed_cpus,
                )?;
                let measured = runtime.observe_original_configuration(
                    requirement.resource.id.as_str(),
                    &expected,
                    limits,
                )?;
                if measured.resource_id() != requirement.resource.id.as_str()
                    || measured.source() != &expected
                {
                    return Err(
                        "original source or occupied position changed during observation".into(),
                    );
                }
                runtime.verify_original_cpu_domain(
                    requirement.resource.id.as_str(),
                    &expected,
                    allowed_cpus,
                )?;
                let source = measured.source();
                Ok(RawSourceObservation {
                    work_binding: requirement.work_binding,
                    resource: requirement.resource.clone(),
                    source: SourceProcessIdentity {
                        pid: source.pid,
                        start_time_ticks: source.start_time_ticks,
                        boot_id: source.boot_id.clone(),
                    },
                    configuration: measured.configuration().to_vec(),
                    executable_sha256: *measured.executable_sha256(),
                    kernel_sha256: *measured.kernel_sha256(),
                    kernel_bytes: measured.kernel_bytes(),
                    disk_bytes: measured.disk_bytes(),
                })
            },
        )
    }
}
