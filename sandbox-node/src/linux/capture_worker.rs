//! One bounded synchronous Full worker. Never assemble a service or spawn here.
use super::capture::require_source_binding;
use super::full_capture::{check_measured_source, expected_compatibility};
use super::{NodeResult, Runtime, WorkAction, core};
use crate::{
    ProtectedHostEpochSource,
    full_snapshot::{self, ExpectedFullCapture},
};
use sandbox_runtime::{
    FullCaptureApiContext, FullCaptureLimits, HelperLimits, OriginalPauseObservation, OwnedChild,
    RuntimeConfig, RuntimeHelperGuard, RuntimeProcessIdentity, SnapshotLayout,
    snapshot_store::BundleReadLimits,
};
use sandproof_controller_service::capture::{
    CaptureWorkerBinding, FullCaptureLaunchPermit, FullCaptureWorkRequest, RawSourceObservation,
    SourceObservationRequirement, SourceProcessIdentity,
};
use sandproof_controller_service::templates::TemplateBinding;
use sandproof_protocol::lifecycle::ContentDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[path = "capture_worker_handshake.rs"]
mod handshake;

#[path = "capture_worker_completion.rs"]
mod completion;
pub(super) use completion::read_completion;

pub const CAPTURE_WORKER_INPUT_BYTES: usize = 1_048_576;
pub const CAPTURE_WORKER_OUTPUT_BYTES: usize = 4096;

/// A complete bound Ready from the original worker, not permission for Full.
pub struct PreparedCaptureWorker {
    pub(super) child: OwnedChild,
    process: RuntimeProcessIdentity,
    ready_bytes: usize,
    started: Instant,
    pub(super) gate: RuntimeHelperGuard,
    pub(super) job: CaptureWorkerJob,
    pub(super) body: Vec<u8>,
    job_digest: ContentDigest,
    full_work_binding: ContentDigest,
}
impl PreparedCaptureWorker {
    #[must_use]
    pub fn process(&self) -> &RuntimeProcessIdentity {
        &self.process
    }
    #[must_use]
    pub const fn job_digest(&self) -> &ContentDigest {
        &self.job_digest
    }
    #[must_use]
    pub const fn full_work_binding(&self) -> &ContentDigest {
        &self.full_work_binding
    }
    /// Actual prepared process/job data for the original authorization callback.
    /// This value has no release or completion authority.
    /// # Errors
    /// Refuses a process representation that cannot enter the original record.
    pub fn worker_binding(&self) -> NodeResult<CaptureWorkerBinding> {
        let process = self.process();
        Ok(CaptureWorkerBinding::new(
            SourceProcessIdentity {
                pid: process.pid,
                start_time_ticks: process.start_time_ticks,
                boot_id: process.boot_id.clone(),
            },
            self.job_digest,
        )?)
    }

    fn require_launch_permit(&self, permit: &FullCaptureLaunchPermit) -> NodeResult<()> {
        let payload = handshake::frame_payload(&self.body)?;
        if permit.action() != &self.job.action
            || permit.full_work_binding() != &self.full_work_binding
            || self.job.full.binding_digest()? != self.full_work_binding
            || permit.worker() != &self.worker_binding()?
            || self.body.len() > CAPTURE_WORKER_INPUT_BYTES
            || serde_json::to_vec(&self.job)? != payload
            || digest(payload) != self.job_digest
            || !self.job.full.valid()
        {
            return Err("original committed Full/action/process/job permission differs".into());
        }
        Ok(())
    }

    // Keep the same gate alive through exchange AND the original material readback.
    // Every error is unknown to the durable capture owner, never NoEffect/refund.
    pub(super) fn release_and_read<T>(
        self,
        permit: FullCaptureLaunchPermit,
        read: impl FnOnce(&CaptureWorkerJob, ContentDigest) -> NodeResult<T>,
    ) -> NodeResult<T> {
        self.require_launch_permit(&permit)?;
        // Consume this unique post-commit permission before opening the child gate.
        drop(permit);
        let timeout = Duration::from_millis(self.job.full.worker_timeout_ms)
            .checked_sub(self.started.elapsed())
            .filter(|value| !value.is_zero())
            .ok_or("Full caller deadline elapsed before release")?;
        let limits = HelperLimits {
            timeout,
            input_bytes: CAPTURE_WORKER_INPUT_BYTES,
            stdout_bytes: CAPTURE_WORKER_OUTPUT_BYTES - self.ready_bytes,
            stderr_bytes: 0,
        };
        let mut child = self.child;
        let output = child.exchange(handshake::RELEASE, limits)?;
        let manifest = decode_reply(self.job_digest, &output)?;
        read(&self.job, manifest)
    }

    /// Close the Ready worker's release pipe. Only that worker may persist its
    /// NoRequest decision; dropping these handles supplies no end receipt.
    pub fn cancel(self) {
        drop(self.child);
        drop(self.gate);
        drop(self.job);
        drop(self.body);
    }
    pub(super) fn prepare(
        gate: RuntimeHelperGuard,
        slot: &sandbox_runtime::RuntimeHostSlot,
        executable: &std::path::Path,
        job: CaptureWorkerJob,
    ) -> NodeResult<Self> {
        let started = Instant::now();
        sandbox_runtime::check_trusted_path(executable)?;
        if !executable.is_file() {
            return Err("capture worker executable is not a protected file".into());
        }
        let body = serde_json::to_vec(&job)?;
        if body.len() > CAPTURE_WORKER_INPUT_BYTES - 4 {
            return Err("capture worker input exceeds bound".into());
        }
        let job_digest = digest(&body);
        let full_work_binding = job.full.binding_digest()?;
        let name = executable
            .to_str()
            .ok_or("capture worker executable path is not UTF-8")?;
        let hex = format!("{:x}", Sha256::digest(&body));
        let prepared = gate.prepare_tracked(slot, name, &["capture-worker", &hex])?;
        let process = prepared.process().clone();
        let mut child = gate.release_tracked(prepared)?;
        let frame = handshake::job_frame(body)?;
        let expected_ready = handshake::ready_frame(job_digest, process.clone())?;
        let ready = child.prepare_full(
            &frame,
            &expected_ready,
            HelperLimits {
                timeout: Duration::from_millis(job.full.worker_timeout_ms),
                input_bytes: CAPTURE_WORKER_INPUT_BYTES,
                stdout_bytes: 1024,
                stderr_bytes: 0,
            },
        )?;
        handshake::check_ready(&ready, job_digest, &process)?;
        if !process.is_alive()? {
            return Err("Full worker exited before Ready handoff".into());
        }
        Ok(Self {
            child,
            process,
            ready_bytes: ready.len(),
            started,
            gate,
            job,
            body: frame,
            job_digest,
            full_work_binding,
        })
    }
}

// Private pipe format. Its expected hash is bound to the original journal
// before release; deserializing this object is not an execution permission.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureWorkerJob {
    pub(super) action: WorkAction<core::EffectId>,
    pub(super) full: FullCaptureWorkRequest,
    pub(super) source: SourceObservationRequirement,
    pub(super) runtime: RuntimeConfig,
    pub(super) template: TemplateBinding,
    pub(super) namespace: String,
    pub(super) bootstrap_path: PathBuf,
    pub(super) layout_parent: PathBuf,
    pub(super) layout_binding: ContentDigest,
    pub(super) material_project: sandbox_runtime::MaterialProjectSettings,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureWorkerReply {
    pub(super) job_digest: ContentDigest,
    pub(super) outcome: CaptureWorkerOutcome,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum CaptureWorkerOutcome {
    Captured { manifest_digest: ContentDigest },
    Failed,
}
pub(super) fn api_context(
    action: &WorkAction<core::EffectId>,
    full: &FullCaptureWorkRequest,
    job_digest: ContentDigest,
) -> NodeResult<FullCaptureApiContext> {
    Ok(FullCaptureApiContext {
        work_sha256: *full.binding_digest()?.as_bytes(),
        job_sha256: *job_digest.as_bytes(),
        action_sha256: Sha256::digest(serde_json::to_vec(action)?).into(),
    })
}

fn digest(body: &[u8]) -> ContentDigest {
    ContentDigest::from_sha256(Sha256::digest(body).into())
}

fn decode_reply(
    expected: ContentDigest,
    output: &std::process::Output,
) -> NodeResult<ContentDigest> {
    if !output.status.success()
        || !output.stderr.is_empty()
        || output.stdout.is_empty()
        || output.stdout.len() > CAPTURE_WORKER_OUTPUT_BYTES
    {
        return Err("capture worker exit/output is not a complete bounded reply".into());
    }
    let reply: CaptureWorkerReply = serde_json::from_slice(&output.stdout)?;
    if reply.job_digest != expected {
        return Err("capture worker reply names another original job".into());
    }
    match reply.outcome {
        CaptureWorkerOutcome::Captured { manifest_digest } => Ok(manifest_digest),
        CaptureWorkerOutcome::Failed => {
            Err("capture worker reported an unknown Full outcome".into())
        }
    }
}

fn decode_digest(text: &str) -> NodeResult<ContentDigest> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("capture worker requires one lowercase SHA-256 digest".into());
    }
    let nibble = |b: u8| {
        if b.is_ascii_digit() {
            b - b'0'
        } else {
            b - b'a' + 10
        }
    };
    let mut bytes = [0; 32];
    for (out, pair) in bytes.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        *out = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    Ok(ContentDigest::from_sha256(bytes))
}
fn decode_job(expected: ContentDigest, body: &[u8]) -> NodeResult<CaptureWorkerJob> {
    if body.len() > CAPTURE_WORKER_INPUT_BYTES || digest(body) != expected {
        return Err("capture worker input size or original job digest differs".into());
    }
    Ok(serde_json::from_slice(body)?)
}

impl CaptureWorkerJob {
    fn validate_job(&self) -> NodeResult<()> {
        require_source_binding(&self.full.pause, &self.source)?;
        self.action.identity.validate_version()?;
        let core::Command::SealSnapshot { vm, .. } = self.action.command.command() else {
            return Err("capture worker requires its original SealSnapshot".into());
        };
        if !self.full.valid()
            || self
                .runtime
                .host_slot
                .as_ref()
                .is_none_or(|slot| slot.vm != vm.0)
            || self.runtime.vcpu_count != self.template.resources.cpu_count
            || self.runtime.memory_mib != self.template.resources.memory_mib
            || self.full.memory_bytes != u64::from(self.runtime.memory_mib) * 1_048_576
            || self.full.compatibility.cpu != self.source.host_bootstrap.policy
        {
            return Err("capture worker original machine/work binding differs".into());
        }
        Ok(())
    }

    fn execute(&self, job_digest: ContentDigest) -> NodeResult<ContentDigest> {
        self.validate_job()?;
        let provider = ProtectedHostEpochSource::at(self.bootstrap_path.clone())?;
        let inputs = crate::host_epoch::Inputs {
            namespace: &self.namespace,
            firecracker: &self.runtime.firecracker,
            jailer: &self.runtime.jailer,
        };
        provider.observe_original(&self.source.node, &self.source.host_bootstrap, inputs)?;
        let tool = provider.original_tool_digest(
            &self.source.node,
            &self.source.host_bootstrap,
            inputs,
        )?;
        let allowed =
            provider.original_cpu_domain(&self.source.node, &self.source.host_bootstrap, inputs)?;
        let runtime = Runtime::for_existing(self.runtime.clone())?;
        let expected = expected_compatibility(
            &self.template,
            runtime.boot_plan_digest()?,
            tool,
            self.source.host_bootstrap.policy,
        )?;
        if expected != self.full.compatibility {
            return Err("capture worker independent compatibility differs".into());
        }
        let layout = SnapshotLayout::reopen(&self.layout_parent, *self.layout_binding.as_bytes())?
            .with_material_project(self.material_project.clone())?;
        if layout.runtime_root() != self.runtime.chroot_base {
            return Err("capture worker layout differs".into());
        }
        let original = crate::source_observation::expected_source(&self.source)?;
        self.check_source(&runtime, &original, &allowed)?;
        let paused = match runtime
            .observe_original_pause_state(self.full.pause.source_resource.id.as_str(), &original)?
        {
            OriginalPauseObservation::Paused(value) => value,
            OriginalPauseObservation::Running | OriginalPauseObservation::Exited => {
                return Err("capture worker original source not paused".into());
            }
        };
        let captured = full_snapshot::capture_paused_original(
            &runtime,
            &layout,
            &paused,
            &ExpectedFullCapture {
                resource_id: self.full.pause.source_resource.id.as_str(),
                source: &original,
                binding: &self.full.pause.binding,
                compatibility: &expected,
            },
            FullCaptureLimits {
                api_context: api_context(&self.action, &self.full, job_digest)?,
                state_bytes: self.full.state_bytes,
                disk_bytes: self.full.disk_bytes,
                api_timeout: Duration::from_millis(self.full.api_timeout_ms),
            },
            BundleReadLimits {
                state_bytes: self.full.state_bytes,
                memory_bytes: self.full.memory_bytes,
                disk_bytes: self.full.disk_bytes,
            },
        )?;
        self.check_source(&runtime, &original, &allowed)?;
        provider.observe_original(&self.source.node, &self.source.host_bootstrap, inputs)?;
        let manifest = ContentDigest::from_sha256(captured.stored().manifest_sha256());
        let material_bytes = captured
            .stored()
            .manifest()
            .files
            .iter()
            .try_fold(0_u64, |n, f| n.checked_add(f.length))
            .ok_or("completed Full material length overflow")?;
        self.persist_completion(&layout, job_digest, manifest, material_bytes)?;
        Ok(manifest)
    }
    fn check_source(
        &self,
        runtime: &Runtime,
        original: &RuntimeProcessIdentity,
        allowed: &[u32],
    ) -> NodeResult<()> {
        let id = self.full.pause.source_resource.id.as_str();
        runtime.verify_original_cpu_domain(id, original, allowed)?;
        let measured = runtime.observe_original_configuration(
            id,
            original,
            crate::source_observation::runtime_limits(&self.source.limits)?,
        )?;
        if measured.resource_id() != id || measured.source() != original {
            return Err("capture worker measured a different source".into());
        }
        check_measured_source(
            &self.template,
            &RawSourceObservation {
                work_binding: self.source.work_binding,
                resource: self.source.resource.clone(),
                source: self.source.source.clone(),
                configuration: measured.configuration().to_vec(),
                executable_sha256: *measured.executable_sha256(),
                kernel_sha256: *measured.kernel_sha256(),
                kernel_bytes: measured.kernel_bytes(),
                disk_bytes: measured.disk_bytes(),
            },
            &self.full.compatibility,
        )?;
        runtime.verify_original_cpu_domain(id, original, allowed)?;
        Ok(())
    }
}

/// Internal same-binary worker entry, dispatched before Tokio/HostLease setup.
/// Input is one finite private pipe object, bound by the original journal before
/// release. This synchronous function never launches a child or forwards an FD.
/// A structured Failed reply is not a no-effect or completion assertion.
/// # Errors
/// Invalid invocation or reply I/O fails the process; no VM Stop/Resume occurs.
pub fn run_capture_worker(expected_job_digest: &str) -> NodeResult<()> {
    handshake::run(expected_job_digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invocation_and_pipe_guards_fail_before_any_runtime_is_constructed() {
        for value in ["", "aa", &"A".repeat(64), &"g".repeat(64)] {
            assert!(decode_digest(value).is_err());
        }
        assert_eq!(
            decode_digest(&"ab".repeat(32)).unwrap().as_bytes(),
            &[0xab; 32]
        );
        assert!(decode_job(digest(b"{}"), b"{}").is_err());
        assert!(decode_job(digest(b"original"), b"changed").is_err());
        let oversized = vec![b' '; CAPTURE_WORKER_INPUT_BYTES + 1];
        assert!(decode_job(digest(&oversized), &oversized).is_err());
    }
    #[test]
    fn parent_requires_exact_successful_job_and_bounded_complete_frame() {
        use std::os::unix::process::ExitStatusExt;
        let expected = ContentDigest::from_sha256([1; 32]);
        let manifest = ContentDigest::from_sha256([2; 32]);
        let reply = CaptureWorkerReply {
            job_digest: expected,
            outcome: CaptureWorkerOutcome::Captured {
                manifest_digest: manifest,
            },
        };
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: serde_json::to_vec(&reply).unwrap(),
            stderr: vec![],
        };
        assert_eq!(decode_reply(expected, &output).unwrap(), manifest);
        assert!(decode_reply(ContentDigest::from_sha256([3; 32]), &output).is_err());
        for variant in 0..6 {
            let mut bad = output.clone();
            match variant {
                0 => bad.status = std::process::ExitStatus::from_raw(1 << 8),
                1 => bad.stderr.push(b'x'),
                2 => bad.stdout.clear(),
                3 => bad.stdout = vec![b' '; CAPTURE_WORKER_OUTPUT_BYTES + 1],
                4 => bad.stdout.extend_from_slice(b"warning"),
                _ => {
                    bad.stdout = serde_json::to_vec(&CaptureWorkerReply {
                        job_digest: expected,
                        outcome: CaptureWorkerOutcome::Failed,
                    })
                    .unwrap();
                }
            }
            assert!(decode_reply(expected, &bad).is_err(), "variant {variant}");
        }
    }

    #[test]
    fn diagnostic_noise_cannot_be_accepted_as_a_success_frame() {
        let reply = CaptureWorkerReply {
            job_digest: ContentDigest::from_sha256([1; 32]),
            outcome: CaptureWorkerOutcome::Captured {
                manifest_digest: ContentDigest::from_sha256([2; 32]),
            },
        };
        let bytes = serde_json::to_vec(&reply).unwrap();
        assert!(serde_json::from_slice::<CaptureWorkerReply>(&bytes).is_ok());
        for prefix in [true, false] {
            let mut bad = bytes.clone();
            if prefix {
                bad.splice(0..0, b"warning\n".iter().copied());
            } else {
                bad.extend_from_slice(b"warning\n");
            }
            assert!(serde_json::from_slice::<CaptureWorkerReply>(&bad).is_err());
        }
    }
}
