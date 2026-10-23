//! One bounded synchronous Full worker. Never assemble a service or spawn here.
use super::capture::require_source_binding;
use super::full_capture::{check_measured_source, expected_compatibility};
use super::{NodeResult, Runtime, WorkAction, core};
use crate::{
    ProtectedHostEpochSource,
    full_snapshot::{self, ExpectedFullCapture},
};
use sandbox_runtime::{
    FullCaptureLimits, OriginalPauseObservation, PreparedTrackedChild, RuntimeConfig,
    RuntimeHelperGuard, RuntimeProcessIdentity, SnapshotLayout, snapshot_store::BundleReadLimits,
};
use sandproof_controller_service::capture::{
    FullCaptureWorkRequest, RawSourceObservation, SourceObservationRequirement,
};
use sandproof_controller_service::templates::TemplateBinding;
use sandproof_protocol::lifecycle::ContentDigest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};

pub const CAPTURE_WORKER_INPUT_BYTES: usize = 1_048_576;
pub const CAPTURE_WORKER_OUTPUT_BYTES: usize = 4096;

/// Only an actual prepared process. It grants no release, budget or Full success.
pub struct PreparedCaptureWorker {
    pub(super) child: PreparedTrackedChild,
    pub(super) gate: RuntimeHelperGuard,
    pub(super) job: CaptureWorkerJob,
    pub(super) body: Vec<u8>,
    job_digest: ContentDigest,
    full_work_binding: ContentDigest,
}
impl PreparedCaptureWorker {
    #[must_use]
    pub fn process(&self) -> &RuntimeProcessIdentity {
        self.child.process()
    }
    #[must_use]
    pub const fn job_digest(&self) -> &ContentDigest {
        &self.job_digest
    }
    #[must_use]
    pub const fn full_work_binding(&self) -> &ContentDigest {
        &self.full_work_binding
    }
    /// Stop only this still-prepared child before dropping its slot lock.
    /// This is not an exit/Full-end or budget-release receipt; uncertain child
    /// exits remain in the original witness/reaper for separate observation.
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
        sandbox_runtime::check_trusted_path(executable)?;
        if !executable.is_file() {
            return Err("capture worker executable is not a protected file".into());
        }
        let body = serde_json::to_vec(&job)?;
        if body.len() > CAPTURE_WORKER_INPUT_BYTES {
            return Err("capture worker input exceeds bound".into());
        }
        let job_digest = digest(&body);
        let full_work_binding = job.full.binding_digest()?;
        let name = executable
            .to_str()
            .ok_or("capture worker executable path is not UTF-8")?;
        let hex = format!("{:x}", Sha256::digest(&body));
        let child = gate.prepare_tracked(slot, name, &["capture-worker", &hex])?;
        Ok(Self {
            child,
            gate,
            job,
            body,
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
fn digest(body: &[u8]) -> ContentDigest {
    ContentDigest::from_sha256(Sha256::digest(body).into())
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
    fn execute(&self) -> NodeResult<ContentDigest> {
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
        let layout = SnapshotLayout::reopen(&self.layout_parent, *self.layout_binding.as_bytes())?;
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
        Ok(ContentDigest::from_sha256(
            captured.stored().manifest_sha256(),
        ))
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
    let expected = decode_digest(expected_job_digest)?;
    let mut body = Vec::new();
    let outcome = match std::io::stdin()
        .take(u64::try_from(CAPTURE_WORKER_INPUT_BYTES)? + 1)
        .read_to_end(&mut body)
    {
        Ok(_) => match decode_job(expected, &body).and_then(|job| job.execute()) {
            Ok(manifest_digest) => CaptureWorkerOutcome::Captured { manifest_digest },
            Err(_) => CaptureWorkerOutcome::Failed,
        },
        Err(_) => CaptureWorkerOutcome::Failed,
    };
    let reply = serde_json::to_vec(&CaptureWorkerReply {
        job_digest: expected,
        outcome,
    })?;
    if reply.len() > CAPTURE_WORKER_OUTPUT_BYTES {
        return Err("capture worker reply exceeds bound".into());
    }
    let mut output = std::io::stdout().lock();
    output.write_all(&reply)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
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
