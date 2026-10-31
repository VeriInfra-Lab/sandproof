//! Capture responsibility inside the original operation plan.
//!
//! Each fixed use slot is named by its containing plan's full operation,
//! source generation, VM and target snapshot. These are logical obligations,
//! never Arc counts or evidence that a file/worker has physically disappeared.
//! A registered bundle still requires the service's exact durable receipt and
//! the host's full immutable-bundle contract; a manifest counter is not that
//! evidence. This module does not enable the product pause/resume profile.

#[cfg(test)]
mod tests;

pub mod continuation;
pub mod drain;
pub use continuation::{
    CaptureInput, CaptureSealEnd, CaptureWorkEnd, CaptureWorkEndReceipt, OriginalContinueIdentity,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CaptureCommit {
    NotRegistered,
    Unknown,
    Registered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RestoreEligibility {
    Capturing,
    Recoverable,
    Revoked,
}

/// Fixed-size use slots are reserved with the accepted operation, before
/// capture work. Revoking execution eligibility never releases either slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CaptureHandoff {
    commit: CaptureCommit,
    eligibility: RestoreEligibility,
    capture_work: bool,
    unload_source: bool,
    original_continuation: Option<continuation::OriginalContinuation>,
    terminal_drain: Option<drain::CaptureDrainCoverage>,
}

impl CaptureHandoff {
    #[must_use]
    pub const fn accepted() -> Self {
        Self {
            commit: CaptureCommit::NotRegistered,
            eligibility: RestoreEligibility::Capturing,
            capture_work: true,
            unload_source: false,
            original_continuation: None,
            terminal_drain: None,
        }
    }

    #[must_use]
    pub const fn commit(self) -> CaptureCommit {
        self.commit
    }

    #[must_use]
    pub const fn eligibility(self) -> RestoreEligibility {
        self.eligibility
    }

    #[must_use]
    pub const fn has_capture_work(self) -> bool {
        self.capture_work
    }

    #[must_use]
    pub const fn has_unload_source(self) -> bool {
        self.unload_source
    }

    /// No new capture dispatch after ordinary failure/termination.
    #[must_use]
    pub fn accepts_registration(self) -> bool {
        self.eligibility == RestoreEligibility::Capturing
            && self.capture_work
            && !self.unload_source
            && self.commit != CaptureCommit::Registered
    }

    /// Persist the conservative uncertainty before dispatching the commit.
    #[must_use]
    pub fn registration_dispatched(mut self) -> Self {
        if self.accepts_registration() {
            self.commit = CaptureCommit::Unknown;
        }
        self
    }

    /// Logical registration in the same successor as the durable result.
    /// The caller validates the exact original bundle and operation first.
    #[must_use]
    pub fn register_bundle(self) -> Option<Self> {
        if !self.accepts_registration() {
            return None;
        }
        self.observe_bundle()
    }

    /// A previously dispatched commit may complete after cancellation. Record
    /// its material without resurrecting eligibility or granting a stop.
    #[must_use]
    pub fn observe_bundle(mut self) -> Option<Self> {
        if !self.capture_work
            || self.unload_source
            || self.commit == CaptureCommit::Registered
            || self.eligibility == RestoreEligibility::Recoverable
        {
            return None;
        }
        self.commit = CaptureCommit::Registered;
        if self.eligibility == RestoreEligibility::Capturing {
            self.eligibility = RestoreEligibility::Recoverable;
        }
        self.unload_source = true;
        Some(self)
    }

    /// Before handoff, keep the original VM and invalidate late captures.
    /// After handoff, keep the registered source and continue that unload.
    /// An unknown result stays unknown and all material uses remain charged.
    #[must_use]
    pub fn ordinary_failure(mut self) -> Self {
        if self.commit != CaptureCommit::Registered {
            self.eligibility = RestoreEligibility::Revoked;
        }
        self
    }

    /// Owner destroy/expiry may forfeit state; material cleanup is separate.
    #[must_use]
    pub const fn terminate(mut self) -> Self {
        self.eligibility = RestoreEligibility::Revoked;
        self
    }

    /// A logical stop permit, conditional on exact bundle registration.
    #[must_use]
    pub fn may_stop_original(self) -> bool {
        self.commit == CaptureCommit::Registered
            && self.eligibility == RestoreEligibility::Recoverable
            && self.unload_source
    }

    /// Future thaw must also discharge the *original* work and uncertainty.
    /// This batch deliberately exposes no API which fabricates that evidence.
    #[must_use]
    pub fn may_reactivate_original(self) -> bool {
        self.commit == CaptureCommit::NotRegistered
            && self.eligibility == RestoreEligibility::Revoked
            && !self.capture_work
            && !self.unload_source
            && self.original_continuation.is_some()
            && self.terminal_drain.is_none()
    }

    #[must_use]
    pub const fn original_continuation(self) -> Option<continuation::OriginalContinuation> {
        self.original_continuation
    }

    #[must_use]
    pub const fn terminal_drain(self) -> Option<drain::CaptureDrainCoverage> {
        self.terminal_drain
    }
    pub(crate) fn finish_terminal_drain(
        mut self,
        value: drain::CaptureDrainCoverage,
    ) -> Option<Self> {
        if self.commit != CaptureCommit::NotRegistered
            || self.eligibility != RestoreEligibility::Revoked
            || !self.capture_work
            || self.unload_source
            || self.original_continuation.is_some()
            || self.terminal_drain.is_some()
        {
            return None;
        }
        self.capture_work = false;
        self.terminal_drain = Some(value);
        Some(self)
    }

    // Only the original guarded core transition ends this work. The service
    // first durably establishes the complete original I/O end facts; partial
    // materials remain independently charged and are not deleted here.
    pub(crate) fn prepare_original_continuation(
        mut self,
        value: continuation::OriginalContinuation,
    ) -> Option<Self> {
        if self.commit != CaptureCommit::NotRegistered
            || self.eligibility != RestoreEligibility::Revoked
            || !self.capture_work
            || self.unload_source
            || self.original_continuation.is_some()
            || self.terminal_drain.is_some()
        {
            return None;
        }
        self.capture_work = false;
        self.original_continuation = Some(value);
        Some(self)
    }
}
