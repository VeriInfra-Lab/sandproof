//! Current lifecycle decisions in the original center authority.
//! One Sandbox row owns one bounded operation history; no shadow lifecycle table.
use crate::{CenterState, DecisionError, NodeRole, Sandbox, TenantBudget};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CaptureId, CenterIdentity, CreateRefusalReason, CurrentReadyFacts,
    ExpectedSession, IncarnationId, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey, RequestWindowId, SessionId, TerminalLifecycleObservation,
};
use sandproof_protocol::{SandboxId, TenantId};
use serde::{Deserialize, Serialize};
mod key;
mod ready;
mod renewal;
mod validation;
pub use sandproof_protocol::lifecycle::CreateParameters;

/// One supported current format. Missing registration is not a legacy profile.
pub const LIFECYCLE_PROFILE_VERSION: u16 = 5;

pub(crate) fn required_option<'de, D, T>(input: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(input)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleWindow {
    pub owner: TenantId,
    pub window: RequestWindowId,
}

/// Full original input. Absolute renewal deadlines are derived only on NEW admission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleSubmission {
    Create {
        key: OperationKey,
        parameters: CreateParameters,
    },
    Unload {
        key: OperationKey,
        session: SessionId,
        expected_version: u64,
    },
    Renew {
        key: OperationKey,
        session: SessionId,
        expected_version: u64,
        timeout_secs: u64,
        extend_only: bool,
    },
    Destroy {
        key: OperationKey,
        session: SessionId,
        expected_version: u64,
    },
}
impl LifecycleSubmission {
    #[must_use]
    pub const fn key(&self) -> &OperationKey {
        match self {
            Self::Create { key, .. }
            | Self::Unload { key, .. }
            | Self::Renew { key, .. }
            | Self::Destroy { key, .. } => key,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Creating,
    Running,
    Renewing,
    Unloading,
    Terminating,
    Cleaned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleOperationRecord {
    pub(crate) submission: LifecycleSubmission,
    pub(crate) intent: AcceptedLifecycleIntent,
    /// Original global allocation frontier, independent of a caller's key sequence.
    pub(crate) allocation_number: u64,
    pub(crate) acceptance_time: AcceptanceTime,
    pub(crate) running: Option<CurrentReadyFacts>,
    /// Actual predecessor lifetime sampled on admission, not reconstructed
    /// from Create or a later native observation after compaction.
    #[serde(deserialize_with = "required_option")]
    pub(crate) previous_effective_end_at_ms: Option<u64>,
    /// Exact original Create refusal; later Destroy never erases its cause.
    pub(crate) create_refusal: Option<CreateRefusalReason>,
    pub(crate) terminal: Option<RetainedTerminalResult>,
}
/// Timed requests preserve their actual sample for deadline reconstruction.
/// Termination is clock-independent and explicitly makes no new time claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AcceptanceTime {
    Sampled { at_ms: u64 },
    Unsampled,
}
impl AcceptanceTime {
    #[must_use]
    pub const fn sampled_at_ms(self) -> Option<u64> {
        match self {
            Self::Sampled { at_ms } => Some(at_ms),
            Self::Unsampled => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetainedTerminalResult {
    pub(crate) result: TerminalLifecycleObservation,
    pub(crate) recorded_at_ms: u64,
    /// A temporary bad clock leaves this uninitialized. The same finishing
    /// CAS may establish it ONCE after clock recovery; never refresh it.
    pub(crate) retention_started_at_ms: Option<u64>,
    /// None with a start means checked arithmetic exhausted; without a start
    /// it means the actual Cleaned fact is awaiting a trustworthy clock.
    pub(crate) retain_until_ms: Option<u64>,
}
impl LifecycleOperationRecord {
    fn followup(
        submission: &LifecycleSubmission,
        previous: &Self,
        intent: AcceptedLifecycleIntent,
        allocation_number: u64,
        now_ms: u64,
    ) -> Self {
        Self {
            submission: submission.clone(),
            intent,
            allocation_number,
            acceptance_time: if matches!(submission, LifecycleSubmission::Destroy { .. }) {
                AcceptanceTime::Unsampled
            } else {
                AcceptanceTime::Sampled { at_ms: now_ms }
            },
            running: None,
            previous_effective_end_at_ms: if matches!(submission, LifecycleSubmission::Renew { .. })
            {
                previous.running.as_ref().map(|facts| facts.end_at_ms)
            } else {
                None
            },
            create_refusal: None,
            terminal: None,
        }
    }

    #[must_use]
    pub const fn intent(&self) -> &AcceptedLifecycleIntent {
        &self.intent
    }
    #[must_use]
    pub const fn running(&self) -> Option<&CurrentReadyFacts> {
        self.running.as_ref()
    }
    #[must_use]
    pub const fn create_refusal(&self) -> Option<CreateRefusalReason> {
        self.create_refusal
    }
    #[must_use]
    pub const fn acceptance_time(&self) -> AcceptanceTime {
        self.acceptance_time
    }
    #[must_use]
    pub fn terminal(&self) -> Option<&TerminalLifecycleObservation> {
        self.terminal.as_ref().map(|retained| &retained.result)
    }
    /// # Errors
    /// Reusing a key with any changed original field conflicts.
    pub fn replay(
        &self,
        submission: &LifecycleSubmission,
    ) -> Result<AcceptedLifecycleIntent, DecisionError> {
        if self.submission != *submission {
            return Err(DecisionError::Conflict);
        }
        Ok(self.intent.clone())
    }
}
impl Sandbox {
    fn find_operation(&self, key: &OperationKey) -> Option<&LifecycleOperationRecord> {
        let index = self.operation_index(key)?;
        self.operations.get(index)
    }
    /// Resolve the same first match before borrowing its returned row.
    fn operation_index(&self, key: &OperationKey) -> Option<usize> {
        let mut index = 0;
        while index < self.operations.len() {
            if self.operations[index].intent.request.binding.key == *key {
                return Some(index);
            }
            index += 1;
        }
        None
    }
    #[must_use]
    pub fn operations(&self) -> &[LifecycleOperationRecord] {
        &self.operations
    }
    #[must_use]
    pub fn original_sandbox(&self) -> Option<&SandboxId> {
        Some(
            self.operations
                .first()?
                .intent
                .request
                .binding
                .session
                .as_protocol_id(),
        )
    }
    #[must_use]
    pub fn phase(&self) -> SessionPhase {
        let Some(operation) = self.operations.last() else {
            return SessionPhase::Creating;
        };
        match &operation.intent.request.operation {
            LifecycleOperation::Create { .. } | LifecycleOperation::Renew { .. }
                if operation.running.is_some() =>
            {
                SessionPhase::Running
            }
            LifecycleOperation::Create { .. } => SessionPhase::Creating,
            LifecycleOperation::Renew { .. } => SessionPhase::Renewing,
            LifecycleOperation::Destroy {} if self.released() => SessionPhase::Cleaned,
            LifecycleOperation::Destroy {} => SessionPhase::Terminating,
            LifecycleOperation::Unload { .. } | LifecycleOperation::Resume { .. } => {
                SessionPhase::Unloading
            }
        }
    }
    #[must_use]
    pub fn current_version(&self) -> Option<u64> {
        self.operations
            .last()
            .map(|operation| operation.intent.accepted_version)
    }
    #[must_use]
    pub fn result_retention_elapsed(&self, now_ms: u64) -> bool {
        self.operations
            .last()
            .and_then(|operation| operation.terminal.as_ref())
            .is_some_and(|retained| {
                retained
                    .retain_until_ms
                    .is_some_and(|until| now_ms >= until)
            })
    }
    pub(crate) fn retention_deadline(&self, now_ms: u64, retention_ms: u64) -> Option<u64> {
        if !self.acceptance_times_not_after(now_ms) {
            return None;
        }
        now_ms.checked_add(retention_ms)
    }
    pub(crate) fn acceptance_times_not_after(&self, now_ms: u64) -> bool {
        if self
            .compacted_sampled_acceptance_max_ms
            .is_some_and(|maximum| maximum > now_ms)
        {
            return false;
        }
        let mut valid = true;
        let mut index = 0;
        while valid && index < self.operations.len() {
            valid = match self.operations[index].acceptance_time {
                AcceptanceTime::Sampled { at_ms } => at_ms <= now_ms,
                AcceptanceTime::Unsampled => true,
            };
            index += 1;
        }
        valid
    }
    #[must_use]
    pub fn termination_due(&self, now_ms: u64) -> bool {
        if self.terminating() {
            return false;
        }
        let Some(current) = self.operations.last() else {
            return false;
        };
        let Some(origin) = self.operations.first() else {
            return false;
        };
        let LifecycleOperation::Create { create, .. } = &origin.intent.request.operation else {
            return false;
        };
        let Some(prior) = self.last_running() else {
            return now_ms >= create.startup_deadline_ms;
        };
        let end = match &current.intent.request.operation {
            LifecycleOperation::Renew {
                end_at_ms,
                extend_only,
                ..
            } => {
                let Ok(target) = u64::try_from(*end_at_ms) else {
                    return true;
                };
                if *extend_only {
                    prior.end_at_ms.max(target)
                } else {
                    target
                }
            }
            _ => prior.end_at_ms,
        };
        now_ms >= end || now_ms >= prior.max_end_at_ms
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleProfile {
    pub(crate) format_version: u16,
    pub(crate) center: CenterIdentity,
    pub(crate) windows: Vec<LifecycleWindowState>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LifecycleWindowState {
    pub(crate) registration: LifecycleWindow,
    /// None is checked public-window exhaustion, never absent registration.
    pub(crate) current: Option<RequestWindowId>,
}
impl LifecycleWindowState {
    pub(crate) fn contains_history(&self, key: &OperationKey) -> bool {
        if key.owner != self.registration.owner {
            return false;
        }
        let initial = self.registration.window.get();
        if initial == 2 {
            return key.window.get() == 2;
        }
        key.window.get() % 2 == 1
            && key.window.get() >= initial
            && self
                .current
                .is_none_or(|current| key.window.get() <= current.get())
    }
}
impl LifecycleProfile {
    pub(crate) fn new(
        center: CenterIdentity,
        mut windows: Vec<LifecycleWindow>,
        tenants: &[TenantBudget],
    ) -> Result<Self, DecisionError> {
        if windows.len() != tenants.len()
            || windows.iter().any(|window| window.window.get() % 2 == 0)
        {
            return Err(DecisionError::Invalid);
        }
        let sdk = RequestWindowId::try_from(2).map_err(|_| DecisionError::Invalid)?;
        let original_count = windows.len();
        for index in 0..original_count {
            windows.push(LifecycleWindow {
                owner: windows[index].owner.clone(),
                window: sdk,
            });
        }
        Ok(Self {
            format_version: LIFECYCLE_PROFILE_VERSION,
            center,
            windows: windows
                .into_iter()
                .map(|registration| LifecycleWindowState {
                    current: Some(registration.window),
                    registration,
                })
                .collect(),
        })
    }
}

/// # Errors
/// A stale version and exhausted successor are distinct refusals.
pub fn next_lifecycle_version(expected: u64, current: u64) -> Result<u64, DecisionError> {
    if expected != current {
        return Err(DecisionError::Stale);
    }
    current.checked_add(1).ok_or(DecisionError::Exhausted)
}

/// Preserve the existing SDK cap for even the largest positive second value.
/// U128 can represent U64 seconds * 1000 plus a U64 clock without overflow.
pub(crate) fn normalized_renewal_end(now_ms: u64, seconds: u64, maximum_ms: u64) -> u64 {
    let requested = u128::from(now_ms) + u128::from(seconds) * 1000;
    // The minimum is at most the original U64 maximum, so narrowing is exact.
    u64::try_from(requested.min(u128::from(maximum_ms))).expect("bounded by original U64 maximum")
}

impl CenterState {
    /// Persist due termination for all original rows, including private work.
    /// This is a bounded pure candidate in the original business CAS, not host
    /// cleanup, a second queue or permission to release any resource.
    /// # Errors
    /// Invalid state or exhausted reserved identity keeps the original authority.
    pub fn terminate_expired(&self, now_ms: u64) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let due = self
            .sandboxes
            .iter()
            .filter(|record| record.termination_due(now_ms))
            .filter_map(|record| {
                record.operations.first().map(|operation| {
                    (
                        operation.intent.request.binding.key.owner.clone(),
                        operation.intent.request.binding.session.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let mut next = self.clone();
        for (owner, session) in due {
            let current = next
                .lifecycle_session(&owner, &session)
                .ok_or(DecisionError::Invalid)?;
            let expected_version = current.current_version().ok_or(DecisionError::Invalid)?;
            let key = next.next_sdk_operation_key(&owner)?;
            let submission = LifecycleSubmission::Destroy {
                key,
                session,
                expected_version,
            };
            next = next.accept_lifecycle(&owner, &submission, now_ms)?.0;
        }
        Ok(next)
    }
    #[must_use]
    pub const fn lifecycle_profile_version(&self) -> u16 {
        self.lifecycle.format_version
    }
    #[must_use]
    pub const fn center_identity(&self) -> &CenterIdentity {
        &self.lifecycle.center
    }
    /// Current authenticated client's window. Exhaustion is explicit; no reset.
    /// # Errors
    /// An unknown owner cannot inspect another registration.
    pub fn public_request_window(
        &self,
        owner: &TenantId,
    ) -> Result<Option<RequestWindowId>, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        self.lifecycle
            .windows
            .iter()
            .find(|window| {
                window.registration.owner == *owner && window.registration.window.get() % 2 == 1
            })
            .map(|window| window.current)
            .ok_or(DecisionError::NotFound)
    }
    #[must_use]
    pub fn required_authority_bytes(&self) -> usize {
        self.bounds
            .required_block_bytes()
            .saturating_add(self.lifecycle_required_bytes())
    }
    pub(crate) fn lifecycle_required_bytes(&self) -> usize {
        usize::from(self.bounds.max_sandboxes)
            .saturating_mul(
                usize::from(self.bounds.operations_per_sandbox)
                    .saturating_mul(
                        (self.bounds.max_options_bytes as usize)
                            .saturating_mul(8)
                            .saturating_add(32768),
                    )
                    .saturating_add(8192),
            )
            .saturating_add(self.tenants.len().saturating_mul(2 * 512))
            .saturating_add(8192)
    }
    #[must_use]
    pub fn lifecycle_session(&self, owner: &TenantId, session: &SessionId) -> Option<&Sandbox> {
        self.sandboxes.iter().find(|record| {
            record.operations.first().is_some_and(|operation| {
                operation.intent.request.binding.key.owner == *owner
                    && operation.intent.request.binding.session == *session
            })
        })
    }
    #[must_use]
    pub fn pending_lifecycle(&self) -> Vec<AcceptedLifecycleIntent> {
        self.sandboxes
            .iter()
            .filter_map(|record| {
                let operation = record.operations.last()?;
                if operation.running.is_some() && record.unconfirmed_sdk_renewal().is_none() {
                    return None;
                }
                if operation.terminal.as_ref().is_some_and(|retained| {
                    retained.retention_started_at_ms.is_some()
                        && record.terminal_confirmation.is_some()
                }) {
                    return None;
                }
                Some(operation.intent.clone())
            })
            .collect()
    }
    fn find_lifecycle_operation(&self, key: &OperationKey) -> Option<&LifecycleOperationRecord> {
        for record in &self.sandboxes {
            if let Some(operation) = record.find_operation(key) {
                return Some(operation);
            }
        }
        None
    }
    /// # Errors
    /// Replay checks full original input before the new clock, phase or capacity.
    pub fn replay_lifecycle(
        &self,
        owner: &TenantId,
        submission: &LifecycleSubmission,
    ) -> Result<Option<AcceptedLifecycleIntent>, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        if submission.key().owner != *owner || submission.key().deployment != self.deployment {
            return Err(DecisionError::NotFound);
        }
        self.find_lifecycle_operation(submission.key())
            .map(|operation| operation.replay(submission))
            .transpose()
    }
    /// # Errors
    /// All refusals are pure. Allocation and intent are saved in the SAME authority CAS.
    /// `now_ms` supplies Create/Unload/Renew samples. Destroy deliberately
    /// ignores it and records Unsampled, so termination needs no working clock.
    pub fn accept_lifecycle(
        &self,
        owner: &TenantId,
        submission: &LifecycleSubmission,
        now_ms: u64,
    ) -> Result<(Self, AcceptedLifecycleIntent), DecisionError> {
        self.accept_lifecycle_for_role(owner, submission, now_ms, NodeRole::Runtime)
    }
    /// Protected private-build admission, using the same original allocator.
    /// No public request can select this role.
    /// # Errors
    /// Only a Create may enter this role; replay retains its actual node role.
    pub fn accept_builder_lifecycle(
        &self,
        owner: &TenantId,
        submission: &LifecycleSubmission,
        now_ms: u64,
    ) -> Result<(Self, AcceptedLifecycleIntent), DecisionError> {
        if !matches!(submission, LifecycleSubmission::Create { .. }) {
            return Err(DecisionError::Invalid);
        }
        self.accept_lifecycle_for_role(owner, submission, now_ms, NodeRole::Builder)
    }
    fn accept_lifecycle_for_role(
        &self,
        owner: &TenantId,
        submission: &LifecycleSubmission,
        now_ms: u64,
        role: NodeRole,
    ) -> Result<(Self, AcceptedLifecycleIntent), DecisionError> {
        if let Some(intent) = self.replay_lifecycle(owner, submission)? {
            if matches!(submission, LifecycleSubmission::Create { .. })
                && !self
                    .nodes
                    .iter()
                    .any(|node| node.identity == intent.request.binding.node && node.role == role)
            {
                return Err(DecisionError::Conflict);
            }
            return Ok((self.clone(), intent));
        }
        if !self.lifecycle.windows.iter().any(|window| {
            window.registration.owner == *owner && window.current == Some(submission.key().window)
        }) {
            return Err(DecisionError::Stale);
        }
        if submission.key().window.get() == 2
            && Some(submission.key().sequence.get()) != self.operation_highwater.checked_add(1)
        {
            return Err(DecisionError::Stale);
        }
        match submission {
            LifecycleSubmission::Create { parameters, .. } => {
                self.accept_lifecycle_create(submission, parameters, now_ms, role)
            }
            LifecycleSubmission::Unload {
                session,
                expected_version,
                ..
            }
            | LifecycleSubmission::Renew {
                session,
                expected_version,
                ..
            }
            | LifecycleSubmission::Destroy {
                session,
                expected_version,
                ..
            } => self.accept_lifecycle_followup(
                submission,
                owner,
                session,
                *expected_version,
                now_ms,
            ),
        }
    }
    fn accept_lifecycle_create(
        &self,
        submission: &LifecycleSubmission,
        parameters: &CreateParameters,
        now_ms: u64,
        role: NodeRole,
    ) -> Result<(Self, AcceptedLifecycleIntent), DecisionError> {
        if parameters.startup_timeout_ms == 0 {
            return Err(DecisionError::Invalid);
        }
        let create = parameters
            .binding_at(now_ms)
            .ok_or(DecisionError::Exhausted)?;
        // The original allocator returns current fields; it constructs no old wire request.
        let allocation = self.allocate_session(&submission.key().owner, &create, now_ms, role)?;
        let intent = AcceptedLifecycleIntent {
            retention: allocation.retention,
            center: self.lifecycle.center.clone(),
            accepted_version: 1,
            request: LifecycleRequest {
                binding: OperationBinding {
                    protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
                    node: allocation.node,
                    session: allocation.session,
                    key: submission.key().clone(),
                    expected: ExpectedSession::Absent,
                },
                operation: LifecycleOperation::Create {
                    create,
                    target_incarnation: IncarnationId::try_from(1)
                        .map_err(|_| DecisionError::Invalid)?,
                },
            },
        };
        let mut next = self.clone();
        next.sandbox_highwater = allocation.sandbox_number;
        next.operation_highwater = allocation.operation_number;
        next.sandboxes.push(Sandbox {
            operations: vec![LifecycleOperationRecord {
                submission: submission.clone(),
                intent: intent.clone(),
                allocation_number: allocation.operation_number,
                acceptance_time: AcceptanceTime::Sampled { at_ms: now_ms },
                running: None,
                previous_effective_end_at_ms: None,
                create_refusal: None,
                terminal: None,
            }],
            terminal_confirmation: None,
            confirmed_renewal: None,
            compacted_sdk_through: None,
            compacted_sampled_acceptance_max_ms: None,
        });
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok((next, intent))
    }
    fn accept_lifecycle_followup(
        &self,
        submission: &LifecycleSubmission,
        owner: &TenantId,
        session: &SessionId,
        expected: u64,
        now_ms: u64,
    ) -> Result<(Self, AcceptedLifecycleIntent), DecisionError> {
        let index = self.index(owner, session.as_protocol_id())?;
        let previous = &self.sandboxes[index];
        let prior = previous.operations.last().ok_or(DecisionError::Invalid)?;
        let version = next_lifecycle_version(expected, prior.intent.accepted_version)?;
        let destroy = matches!(submission, LifecycleSubmission::Destroy { .. });
        let cap = usize::from(self.bounds.operations_per_sandbox);
        if previous.operations.len() >= cap - usize::from(!destroy) {
            return Err(DecisionError::Capacity);
        }
        if previous.terminating() || (!destroy && previous.phase() != SessionPhase::Running) {
            return Err(DecisionError::Conflict);
        }
        if matches!(submission, LifecycleSubmission::Renew { key, .. } if key.window.get() == 2)
            && previous.unconfirmed_sdk_renewal().is_some()
        {
            return Err(DecisionError::Unavailable);
        }
        let number = self
            .operation_highwater
            .checked_add(1)
            .ok_or(DecisionError::Exhausted)?;
        let remaining = self
            .sandboxes
            .iter()
            .filter(|record| !record.released() && !record.terminating())
            .count() as u64
            - u64::from(destroy);
        if number.checked_add(remaining).is_none() {
            return Err(DecisionError::Exhausted);
        }
        let incarnation = match &prior.intent.request.operation {
            LifecycleOperation::Create {
                target_incarnation, ..
            } => *target_incarnation,
            LifecycleOperation::Renew { incarnation, .. } => *incarnation,
            _ if destroy => IncarnationId::try_from(1).map_err(|_| DecisionError::Invalid)?,
            _ => return Err(DecisionError::Conflict),
        };
        let mut request = prior.intent.request.clone();
        request.binding.key = submission.key().clone();
        request.binding.expected = ExpectedSession::Version(expected);
        request.operation = match submission {
            LifecycleSubmission::Destroy { .. } => LifecycleOperation::Destroy {},
            LifecycleSubmission::Unload { .. } => LifecycleOperation::Unload {
                source_incarnation: incarnation,
                capture: CaptureId::try_from(1).map_err(|_| DecisionError::Invalid)?,
            },
            LifecycleSubmission::Renew {
                timeout_secs,
                extend_only,
                ..
            } => {
                let facts = prior.running.as_ref().ok_or(DecisionError::Conflict)?;
                let end = normalized_renewal_end(now_ms, *timeout_secs, facts.max_end_at_ms);
                if *timeout_secs == 0 || facts.end_at_ms <= now_ms || facts.max_end_at_ms <= now_ms
                {
                    return Err(DecisionError::Conflict);
                }
                if !self.node_is_reachable(&request.binding.node, now_ms) {
                    return Err(DecisionError::Unavailable);
                }
                LifecycleOperation::Renew {
                    incarnation,
                    end_at_ms: i64::try_from(end).map_err(|_| DecisionError::Exhausted)?,
                    extend_only: *extend_only,
                }
            }
            LifecycleSubmission::Create { .. } => return Err(DecisionError::Invalid),
        };
        let intent = AcceptedLifecycleIntent {
            retention: prior.intent.retention,
            request,
            accepted_version: version,
            center: prior.intent.center.clone(),
        };
        let mut next = self.clone();
        next.operation_highwater = number;
        next.sandboxes[index]
            .operations
            .push(LifecycleOperationRecord::followup(
                submission,
                prior,
                intent.clone(),
                number,
                now_ms,
            ));
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok((next, intent))
    }
}
