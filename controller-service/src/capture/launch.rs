//! Actual worker identity is data. Only the original committed journal can
//! construct the single-consumption permission used to release that worker.
use super::SourceProcessIdentity;
use crate::JournalError;
use sandbox_controller_core::EffectId;
use sandproof_protocol::{WorkAction, lifecycle::ContentDigest};
use serde::{Deserialize, Serialize};

/// Provider result, never a public response. Unknown keeps the original
/// physical Full, material and source responsibilities; it proves no end.
pub enum FullCaptureResult<E> {
    Captured(sandproof_protocol::WorkReceipt<E>),
    UnknownCapture,
}

/// Observed process/job data, never execution permission by itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureWorkerBinding {
    process: SourceProcessIdentity,
    job_digest: ContentDigest,
}
impl CaptureWorkerBinding {
    /// Record the actual prepared helper returned by the trusted provider.
    /// Shape checks do not attest existence, ownership or a held start gate.
    /// # Errors
    /// Invalid process representation cannot enter the original work record.
    pub fn new(
        process: SourceProcessIdentity,
        job_digest: ContentDigest,
    ) -> Result<Self, JournalError> {
        let value = Self {
            process,
            job_digest,
        };
        if !value.valid() {
            return Err(JournalError::UnsupportedInput);
        }
        Ok(value)
    }

    #[must_use]
    pub fn valid(&self) -> bool {
        self.process.pid > 0
            && self.process.start_time_ticks > 0
            && !self.process.boot_id.is_empty()
            && self.process.boot_id.len() <= 128
            && self
                .process
                .boot_id
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
    }

    #[must_use]
    pub const fn process(&self) -> &SourceProcessIdentity {
        &self.process
    }

    #[must_use]
    pub const fn job_digest(&self) -> &ContentDigest {
        &self.job_digest
    }
}

/// One original post-commit release permission. No public constructor,
/// serialization, cloning or copying can manufacture another permission.
/// The node must compare the complete action as well as all worker bindings.
pub struct FullCaptureLaunchPermit {
    action: WorkAction<EffectId>,
    full_work_binding: ContentDigest,
    worker: CaptureWorkerBinding,
}
impl FullCaptureLaunchPermit {
    // The only caller is the original journal's confirmed-commit return path.
    #[cfg(feature = "durable")]
    pub(crate) fn after_commit(
        action: WorkAction<EffectId>,
        full_work_binding: ContentDigest,
        worker: CaptureWorkerBinding,
    ) -> Self {
        Self {
            action,
            full_work_binding,
            worker,
        }
    }

    #[must_use]
    pub const fn action(&self) -> &WorkAction<EffectId> {
        &self.action
    }

    #[must_use]
    pub const fn full_work_binding(&self) -> &ContentDigest {
        &self.full_work_binding
    }

    #[must_use]
    pub const fn worker(&self) -> &CaptureWorkerBinding {
        &self.worker
    }
}

/// A prepared original child whose gate is still closed. The provider keeps
/// actual process ownership until release/exchange ends, including cancellation.
pub trait PreparedFullCapture<E>: Send {
    fn binding(&self) -> CaptureWorkerBinding;
    fn release(self: Box<Self>, permit: FullCaptureLaunchPermit) -> FullCaptureResult<E>;
}
