//! Original Full provider. Durable claim, reservations and completion stay V5.
use super::capture::require_source_binding;
use super::capture_profile::ProtectedCaptureProfile;
use super::{LinuxWorker, NodeResult, Runtime, WorkAction, core};
use crate::{
    ProtectedHostEpochSource,
    full_snapshot::{self, CheckedStoredCapture, ExpectedFullCapture, FullCaptureObservation},
};
use sandbox_runtime::{
    FullCaptureLimits, OriginalPauseObservation, RuntimeProcessIdentity,
    snapshot_store::BundleReadLimits,
};
use sandproof_controller_service::capture::{
    CommittedCaptureExpectation, FullCaptureWorkRequest, PauseWorkRequest, RawSourceObservation,
    SourceObservationRequirement,
};
use sandproof_controller_service::templates::TemplateBinding;
use sandproof_protocol::{
    ResourceId,
    lifecycle::{ContentDigest, VmCompatibility, WritableDrive},
};
use sha2::{Digest, Sha256};

const SNAPSHOT_FORMAT: u16 = 10; // Fixed FC 1.16.1 SNAPSHOT_VERSION=10.0.0.

fn hex(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(64);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}
pub(super) fn expected_compatibility(
    template: &TemplateBinding,
    device: [u8; 32],
    tool: [u8; 32],
    cpu: ContentDigest,
) -> NodeResult<VmCompatibility> {
    if !template.is_valid() || template.resources.cpu_count > 32 {
        return Err("Full requires the original supported sealed template".into());
    }
    let image = Sha256::digest(serde_json::to_vec(&(
        "cold-template-image",
        template
            .canonical_digest()
            .map_err(|_| "invalid original sealed template digest")?,
        device,
    ))?);
    Ok(VmCompatibility {
        snapshot_format: SNAPSHOT_FORMAT,
        tool: ContentDigest::from_sha256(tool),
        cpu,
        devices: ContentDigest::from_sha256(device),
        image_configuration: ContentDigest::from_sha256(image.into()),
        writable_drives: vec![WritableDrive {
            drive_id: ResourceId::try_from("rootfs")?,
            jail_relative_path: "rootfs.ext4".into(),
        }],
    })
}
pub(super) fn check_measured_source(
    template: &TemplateBinding,
    raw: &RawSourceObservation,
    expected: &VmCompatibility,
) -> NodeResult<()> {
    if raw.executable_sha256 != *expected.tool.as_bytes()
        || hex(&raw.kernel_sha256) != template.kernel.sha256
        || raw.kernel_bytes != template.kernel.size_bytes
        || raw.disk_bytes != template.rootfs.size_bytes
    {
        return Err(
            "actual Full source does not match protected tool and original template".into(),
        );
    }
    Ok(())
}

struct ExpectedColdWork {
    context: sandproof_controller_service::HostSlotContext,
    runtime: Runtime,
    runtime_config: sandbox_runtime::RuntimeConfig,
    template: TemplateBinding,
    request: FullCaptureWorkRequest,
}

fn require_expected_request(
    request: &FullCaptureWorkRequest,
    expected: &FullCaptureWorkRequest,
) -> NodeResult<()> {
    if request != expected {
        return Err("Full request differs from independent protected source/profile".into());
    }
    Ok(())
}

impl LinuxWorker {
    pub(crate) fn recover_full_request_end(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        action: &WorkAction<core::EffectId>,
        full: &FullCaptureWorkRequest,
        source: &SourceObservationRequirement,
        worker: &sandproof_controller_service::capture::CaptureWorkerBinding,
    ) -> NodeResult<sandproof_controller_service::capture::CaptureRequestEndObservation> {
        let context = self.require_full_action(action, full)?;
        require_source_binding(&full.pause, source)?;
        let process = RuntimeProcessIdentity {
            pid: worker.process().pid,
            start_time_ticks: worker.process().start_time_ticks,
            boot_id: worker.process().boot_id.clone(),
        };
        if !worker.valid() || process.boot_id != source.source.boot_id || process.is_alive()? {
            return Err("original Full worker exit not confirmed on this boot".into());
        }
        let slot = self.runtime_host_slot(context.position, context.binding.vm)?;
        let _gate = slot.helper_gate.try_acquire(Some(&slot))?;
        let expected = self.expected_cold_work(provider, profile, &full.pause, source)?;
        require_expected_request(full, &expected.request)?;
        let original = crate::source_observation::expected_source(source)?;
        let record = profile
            .layout
            .read_full_request_end(full_snapshot::capture_directory_key(&full.pause.binding)?)?;
        record.require_original(
            source.resource.id.as_str(),
            &original,
            full_snapshot::capture_directory_key(&full.pause.binding)?,
            super::capture_worker::api_context(action, full, *worker.job_digest())?,
            &process,
        )?;
        // This is not a pause/continue action, material adoption or SQL receipt.
        // A GET on a different socket without the original tagged end record is insufficient.
        if !matches!(
            expected
                .runtime
                .observe_original_pause_state(source.resource.id.as_str(), &original)?,
            OriginalPauseObservation::Paused(_)
        ) || process.is_alive()?
            || self.require_full_action(action, full)? != context
        {
            return Err("original Full request-end context/source changed".into());
        }
        provider.observe_original(
            &source.node,
            &source.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        Ok(
            sandproof_controller_service::capture::CaptureRequestEndObservation {
                action: action.clone(),
                full_work_binding: full.binding_digest()?,
                worker: worker.clone(),
                source: source.source.clone(),
                outcome: match record.outcome() {
                    sandbox_runtime::FullCaptureRequestOutcome::NoRequest => sandproof_controller_service::capture::CaptureRequestEndOutcome::NoRequest,
                    sandbox_runtime::FullCaptureRequestOutcome::Returned { http_status, .. } => sandproof_controller_service::capture::CaptureRequestEndOutcome::Returned { http_status },
                },
            },
        )
    }

    // Independent local expectation. It does not read a manifest/Full DTO and
    // does not claim this cold-source rule automatically covers restored MSRs.
    fn expected_cold_work(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        pause: &PauseWorkRequest,
        original: &SourceObservationRequirement,
    ) -> NodeResult<ExpectedColdWork> {
        require_source_binding(pause, original)?;
        profile.require_original_source(original)?;
        let context = self.original_source_context(&pause.source_resource)?;
        let secret = self.secret(context.binding.sandbox, context.binding.generation)?;
        let template = secret
            .template
            .as_ref()
            .ok_or("Full requires original sealed managed-template provenance")?;
        let runtime_config = self.runtime_config(context.binding.sandbox, context.binding.vm)?;
        let runtime = Runtime::for_existing(runtime_config.clone())?;
        let tool = provider.original_tool_digest(
            &original.node,
            &original.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        let compatibility = expected_compatibility(
            template,
            runtime.boot_plan_digest()?,
            tool,
            original.host_bootstrap.policy,
        )?;
        let memory_bytes = u64::from(template.resources.memory_mib) * 1_048_576;
        if memory_bytes == 0
            || memory_bytes > profile.memory_max
            || template.rootfs.size_bytes > profile.disk_bytes
        {
            return Err("original template exceeds protected Full copy/read caps".into());
        }
        let request = FullCaptureWorkRequest {
            pause: pause.clone(),
            compatibility,
            state_bytes: profile.state_bytes,
            memory_bytes,
            disk_bytes: profile.disk_bytes,
            api_timeout_ms: u64::try_from(profile.api_timeout.as_millis())?,
            worker_timeout_ms: u64::try_from(profile.worker_timeout.as_millis())?,
        };
        if !request.valid() {
            return Err("independent Full request is invalid".into());
        }
        Ok(ExpectedColdWork {
            context,
            runtime,
            runtime_config,
            template: template.clone(),
            request,
        })
    }

    pub(crate) fn prepare_full_capture(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        pause: &PauseWorkRequest,
        original: &SourceObservationRequirement,
    ) -> NodeResult<FullCaptureWorkRequest> {
        let expected = self.expected_cold_work(provider, profile, pause, original)?;
        let raw = crate::source_observation::observe(self, Some(provider), original)?;
        check_measured_source(&expected.template, &raw, &expected.request.compatibility)?;
        let current = self.secret(
            expected.context.binding.sandbox,
            expected.context.binding.generation,
        )?;
        if self.original_source_context(&pause.source_resource)? != expected.context
            || current.template.as_ref() != Some(&expected.template)
        {
            return Err(
                "Full source context or original template changed during preparation".into(),
            );
        }
        Ok(expected.request)
    }

    fn require_full_action(
        &self,
        action: &WorkAction<core::EffectId>,
        request: &FullCaptureWorkRequest,
    ) -> NodeResult<sandproof_controller_service::HostSlotContext> {
        self.validate_action(action)?;
        let core::Command::SealSnapshot {
            sandbox,
            epoch,
            generation,
            vm,
            ..
        } = action.command.command()
        else {
            return Err("Full requires its original SealSnapshot action".into());
        };
        let context = self.original_source_context(&request.pause.source_resource)?;
        if context.binding.sandbox != *sandbox
            || context.binding.generation != *generation
            || context.binding.vm != *vm
        {
            return Err("Full action differs from original source position".into());
        }
        self.require_fence(*sandbox, *epoch, false)?;
        Ok(context)
    }

    pub(crate) fn prepare_capture_worker(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        action: &WorkAction<core::EffectId>,
        request: &FullCaptureWorkRequest,
        original: &SourceObservationRequirement,
        executable: &std::path::Path,
    ) -> NodeResult<super::capture_worker::PreparedCaptureWorker> {
        let context = self.require_full_action(action, request)?;
        let slot = self.runtime_host_slot(context.position, context.binding.vm)?;
        let gate = slot.helper_gate.try_acquire(Some(&slot))?;
        let actual = self.prepare_full_capture(provider, profile, &request.pause, original)?;
        require_expected_request(request, &actual)?;
        let expected = self.expected_cold_work(provider, profile, &request.pause, original)?;
        if expected.context != context
            || expected.request != actual
            || self.require_full_action(action, request)? != context
        {
            return Err("capture worker source changed before preparation".into());
        }
        super::capture_worker::PreparedCaptureWorker::prepare(
            gate,
            &slot,
            executable,
            super::capture_worker::CaptureWorkerJob {
                action: action.clone(),
                full: actual,
                source: original.clone(),
                runtime: expected.runtime_config,
                template: expected.template,
                namespace: self.config.namespace.clone(),
                bootstrap_path: provider.original_path().to_path_buf(),
                layout_parent: profile.layout_parent.clone(),
                layout_binding: profile.layout_binding,
                material_project: profile.material_project.clone(),
            },
        )
    }

    pub(crate) fn observe_capture_commit(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        action: &WorkAction<core::EffectId>,
        full: &FullCaptureWorkRequest,
        source: &SourceObservationRequirement,
        expected: &CommittedCaptureExpectation,
    ) -> NodeResult<super::HostObservation> {
        let seal = expected.seal_action();
        self.require_capture_commit_action(action, seal)?;
        self.require_full_action(seal, full)?;
        require_source_binding(&full.pause, source)?;
        if expected.work() != full {
            return Err("Commit differs from original durably completed Full".into());
        }
        let context = self.original_source_context(&source.resource)?;
        let slot = self.runtime_host_slot(context.position, context.binding.vm)?;
        let _gate = slot.helper_gate.try_acquire(Some(&slot))?;
        provider.observe_original(
            &source.node,
            &source.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        let stored = observe_committed_capture(profile, expected)?;
        let material_bytes = stored
            .manifest()
            .files
            .iter()
            .try_fold(0_u64, |n, f| n.checked_add(f.length))
            .filter(|n| *n > 0)
            .ok_or("invalid actual material length sum")?;
        self.require_full_action(seal, full)?;
        if self.original_source_context(&source.resource)? != context {
            return Err("source changed during material registration observation".into());
        }
        Ok(super::HostObservation {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            result: super::ObservedResult::SnapshotCommitted {
                manifest_digest: ContentDigest::from_sha256(stored.manifest_sha256()),
                material_bytes,
            },
        })
    }

    pub(crate) fn execute_prepared_full_capture(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        prepared: super::capture_worker::PreparedCaptureWorker,
        permit: sandproof_controller_service::capture::FullCaptureLaunchPermit,
    ) -> sandproof_controller_service::capture::FullCaptureResult<super::HostObservation> {
        use sandproof_controller_service::capture::FullCaptureResult;
        let action = permit.action().clone();
        match self.release_prepared_full_capture(provider, profile, prepared, permit) {
            Ok(stored) => {
                let Some(material_bytes) = stored
                    .manifest()
                    .files
                    .iter()
                    .try_fold(0_u64, |total, file| total.checked_add(file.length))
                else {
                    return FullCaptureResult::UnknownCapture;
                };
                if material_bytes == 0 {
                    return FullCaptureResult::UnknownCapture;
                }
                FullCaptureResult::Captured(sandproof_protocol::WorkReceipt {
                    identity: action.identity.clone(),
                    outcome: sandproof_protocol::WorkOutcome::ObservedSuccess(
                        super::HostObservation {
                            resource: action.identity.resource.clone(),
                            command: action.command.command().clone(),
                            result: super::ObservedResult::SnapshotSealed {
                                manifest_digest: ContentDigest::from_sha256(
                                    stored.manifest_sha256(),
                                ),
                                material_bytes,
                            },
                        },
                    ),
                })
            }
            Err(_) => FullCaptureResult::UnknownCapture,
        }
    }

    pub(crate) fn release_prepared_full_capture(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        prepared: super::capture_worker::PreparedCaptureWorker,
        permit: sandproof_controller_service::capture::FullCaptureLaunchPermit,
    ) -> NodeResult<CheckedStoredCapture> {
        self.require_full_action(&prepared.job.action, &prepared.job.full)?;
        let expected = self.prepare_full_capture(
            provider,
            profile,
            &prepared.job.full.pause,
            &prepared.job.source,
        )?;
        require_expected_request(&prepared.job.full, &expected)?;
        let worker = prepared.worker_binding()?;
        prepared.release_and_read(permit, |job, manifest| {
            // Read the actual original bundle under the SAME retained gate.
            // This is material observation, not durable adoption or new access.
            let stored = self.observe_full_capture(
                provider,
                profile,
                &job.full,
                &job.source,
                Some(*manifest.as_bytes()),
            )?;
            let completion = super::capture_worker::read_completion(
                &profile.layout,
                &job.action,
                &job.full,
                &job.source,
                &worker,
            )?;
            let bytes = stored
                .manifest()
                .files
                .iter()
                .try_fold(0_u64, |n, f| n.checked_add(f.length))
                .ok_or("material length overflow")?;
            if completion.manifest_digest != manifest || completion.material_bytes != bytes {
                return Err("worker completion differs from actual published material".into());
            }
            self.require_full_action(&job.action, &job.full)?;
            Ok(stored)
        })
    }

    pub(crate) fn recover_completed_full(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        action: &WorkAction<core::EffectId>,
        full: &FullCaptureWorkRequest,
        source: &SourceObservationRequirement,
        worker: &sandproof_controller_service::capture::CaptureWorkerBinding,
    ) -> NodeResult<super::HostObservation> {
        let context = self.require_full_action(action, full)?;
        require_source_binding(&full.pause, source)?;
        provider.observe_original(
            &source.node,
            &source.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        let process = RuntimeProcessIdentity {
            pid: worker.process().pid,
            start_time_ticks: worker.process().start_time_ticks,
            boot_id: worker.process().boot_id.clone(),
        };
        if process.boot_id != source.source.boot_id || process.is_alive()? {
            return Err("original Full worker exit not confirmed on this boot".into());
        }
        let slot = self.runtime_host_slot(context.position, context.binding.vm)?;
        let _gate = slot.helper_gate.try_acquire(Some(&slot))?;
        let completion =
            super::capture_worker::read_completion(&profile.layout, action, full, source, worker)?;
        let runtime = self.runtime(context.binding.sandbox, context.binding.vm)?;
        let original = crate::source_observation::expected_source(source)?;
        if !matches!(
            runtime.observe_original_pause_state(source.resource.id.as_str(), &original)?,
            OriginalPauseObservation::Paused(_)
        ) {
            return Err("completed Full original source no longer paused".into());
        }
        let stored = self.observe_full_capture(
            provider,
            profile,
            full,
            source,
            Some(*completion.manifest_digest.as_bytes()),
        )?;
        let bytes = stored
            .manifest()
            .files
            .iter()
            .try_fold(0_u64, |n, f| n.checked_add(f.length))
            .filter(|n| *n > 0)
            .ok_or("invalid completed material length sum")?;
        if completion.material_bytes != bytes || self.require_full_action(action, full)? != context
        {
            return Err("completed Full material or original context differs".into());
        }
        Ok(super::HostObservation {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            result: super::ObservedResult::SnapshotSealed {
                manifest_digest: completion.manifest_digest,
                material_bytes: bytes,
            },
        })
    }

    pub(crate) fn capture_original(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        action: &WorkAction<core::EffectId>,
        request: &FullCaptureWorkRequest,
        original: &SourceObservationRequirement,
    ) -> NodeResult<FullCaptureObservation> {
        let context = self.require_full_action(action, request)?;
        let slot = self.runtime_host_slot(context.position, context.binding.vm)?;
        let _gate = slot.helper_gate.try_acquire(Some(&slot))?;
        let expected = self.prepare_full_capture(provider, profile, &request.pause, original)?;
        require_expected_request(request, &expected)?;
        if self.require_full_action(action, request)? != context {
            return Err("Full original context changed".into());
        }
        let execution = self.expected_cold_work(provider, profile, &request.pause, original)?;
        if execution.context != context || execution.request != expected {
            return Err("Full configuration changed before actual execution".into());
        }
        let runtime = execution.runtime;
        let source = crate::source_observation::expected_source(original)?;
        let paused = match runtime
            .observe_original_pause_state(request.pause.source_resource.id.as_str(), &source)?
        {
            OriginalPauseObservation::Paused(paused) => paused,
            OriginalPauseObservation::Running | OriginalPauseObservation::Exited => {
                return Err("original Full source not observed Paused".into());
            }
        };
        let observed = full_snapshot::capture_paused_original(
            &runtime,
            &profile.layout,
            &paused,
            &ExpectedFullCapture {
                resource_id: request.pause.source_resource.id.as_str(),
                source: &source,
                binding: &request.pause.binding,
                compatibility: &expected.compatibility,
            },
            FullCaptureLimits {
                // This direct physical probe is not the prepared SQL worker.
                // Its distinct job hash cannot satisfy a saved worker binding.
                api_context: super::capture_worker::api_context(
                    action,
                    request,
                    ContentDigest::from_sha256(
                        Sha256::digest(serde_json::to_vec(&(
                            "direct-physical-full",
                            action,
                            request,
                            original,
                        ))?)
                        .into(),
                    ),
                )?,
                state_bytes: profile.state_bytes,
                disk_bytes: profile.disk_bytes,
                api_timeout: profile.api_timeout,
            },
            read_limits(&expected),
        )?;
        // Any failure here can follow actual Full/publication. Keep the same
        // durable Dispatched/Unknown work and its concurrency/material charge.
        let after = self.prepare_full_capture(provider, profile, &request.pause, original)?;
        if after != expected || self.require_full_action(action, request)? != context {
            return Err("original Full source/profile or current fence changed".into());
        }
        Ok(observed)
    }

    pub(crate) fn observe_full_capture(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        request: &FullCaptureWorkRequest,
        original: &SourceObservationRequirement,
        manifest: Option<[u8; 32]>,
    ) -> NodeResult<CheckedStoredCapture> {
        // No live-source assumption for this FILE observation. It cannot prove
        // a prior Full/helper/copy worker ended, or replace saved provenance.
        let expected = self
            .expected_cold_work(provider, profile, &request.pause, original)?
            .request;
        require_expected_request(request, &expected)?;
        provider.observe_original(
            &original.node,
            &original.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        let source = RuntimeProcessIdentity {
            pid: original.source.pid,
            start_time_ticks: original.source.start_time_ticks,
            boot_id: original.source.boot_id.clone(),
        };
        full_snapshot::observe_stored_capture(
            &profile.layout,
            &ExpectedFullCapture {
                resource_id: request.pause.source_resource.id.as_str(),
                source: &source,
                binding: &request.pause.binding,
                compatibility: &expected.compatibility,
            },
            manifest,
            read_limits(&expected),
        )
    }
}
// This path deliberately has no worker, position, credential or live-source
// input. The service-only expectation is the original durable material record.
pub(crate) fn observe_committed_capture(
    profile: &ProtectedCaptureProfile,
    expected: &CommittedCaptureExpectation,
) -> NodeResult<CheckedStoredCapture> {
    let request = expected.work();
    require_committed_read_scope(
        request,
        profile.epoch.node(),
        profile.epoch.registration().policy,
        BundleReadLimits {
            state_bytes: profile.state_bytes,
            memory_bytes: profile.memory_max,
            disk_bytes: profile.disk_bytes,
        },
    )?;
    let source = RuntimeProcessIdentity {
        pid: request.pause.source.pid,
        start_time_ticks: request.pause.source.start_time_ticks,
        boot_id: request.pause.source.boot_id.clone(),
    };
    full_snapshot::observe_stored_capture(
        &profile.layout,
        &ExpectedFullCapture {
            resource_id: request.pause.source_resource.id.as_str(),
            source: &source,
            binding: &request.pause.binding,
            compatibility: &request.compatibility,
        },
        Some(*expected.manifest_digest().as_bytes()),
        read_limits(request),
    )
}
fn require_committed_read_scope(
    request: &FullCaptureWorkRequest,
    node: &sandproof_protocol::internal::NodeIdentity,
    epoch: ContentDigest,
    caps: BundleReadLimits,
) -> NodeResult<()> {
    if !request.valid()
        || &request.pause.node != node
        || request.compatibility.cpu != epoch
        || request.state_bytes > caps.state_bytes
        || request.memory_bytes > caps.memory_bytes
        || request.disk_bytes > caps.disk_bytes
    {
        return Err("committed capture differs from protected store epoch or read caps".into());
    }
    Ok(())
}
fn read_limits(request: &FullCaptureWorkRequest) -> BundleReadLimits {
    BundleReadLimits {
        state_bytes: request.state_bytes,
        memory_bytes: request.memory_bytes,
        disk_bytes: request.disk_bytes,
    }
}

#[cfg(test)]
#[path = "full_capture_tests.rs"]
mod tests;
