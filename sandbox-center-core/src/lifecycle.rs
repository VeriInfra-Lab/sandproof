//! Opt-in lifecycle metadata inside the original center authority block.
//! Allocation, reservations and host lanes stay in the original `CenterState`.
use crate::{CenterState, DecisionError, NodeRole};
use sandproof_protocol::internal::PositiveCounter;
use sandproof_protocol::internal::lifecycle::{
    CreateBinding, LifecycleFact, LifecycleResponse, Resources,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CaptureId, CenterIdentity, ExpectedSession, IncarnationId,
    LifecycleOperation, LifecycleRequest, OperationBinding, OperationKey, RequestWindowId,
    SessionId,
};
use sandproof_protocol::{ResourceId, SandboxId, TenantId};
use serde::{Deserialize, Serialize};
mod validation;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleWindow {
    pub owner: TenantId,
    pub window: RequestWindowId,
}

/// Relative user parameters, saved before any now-dependent canonical fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateParameters {
    pub template_revision: ResourceId,
    pub template_digest: [u8; 32],
    pub resources: Resources,
    pub options: Vec<u8>,
    pub startup_timeout_ms: u64,
    pub timeout_ms: u64,
    pub max_lifetime_ms: u64,
}

/// The normalized, complete submission. Owner is additionally checked against
/// the separately authenticated caller; no public field selects a node or ID.
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
            Self::Create { key, .. } | Self::Unload { key, .. } | Self::Destroy { key, .. } => key,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Creating,
    Running,
    Unloading,
    Terminating,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleOperationRecord {
    pub(crate) submission: LifecycleSubmission,
    pub(crate) intent: AcceptedLifecycleIntent,
    /// Only a checked original Ready result is currently supported. Unloaded
    /// cannot be constructed from this legacy result representation.
    pub(crate) ready: Option<LifecycleResponse>,
}
impl LifecycleOperationRecord {
    #[must_use]
    pub const fn intent(&self) -> &AcceptedLifecycleIntent {
        &self.intent
    }
    /// Actual same-key decision, shared by admission and the extracted proof.
    /// # Errors
    /// A changed full submission is a conflict even if its operation key agrees.
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleSession {
    pub(crate) original_sandbox: SandboxId,
    pub(crate) created_at_ms: u64,
    pub(crate) operations: Vec<LifecycleOperationRecord>,
}
impl LifecycleSession {
    #[expect(
        clippy::manual_find,
        reason = "Keep this original short-circuit loop separate for the fixed Aeneas frontend"
    )]
    fn find_operation(&self, key: &OperationKey) -> Option<&LifecycleOperationRecord> {
        for operation in &self.operations {
            if operation.intent.request.binding.key == *key {
                return Some(operation);
            }
        }
        None
    }
    #[must_use]
    pub fn operations(&self) -> &[LifecycleOperationRecord] {
        &self.operations
    }
    #[must_use]
    pub const fn original_sandbox(&self) -> &SandboxId {
        &self.original_sandbox
    }
    #[must_use]
    pub fn phase(&self) -> SessionPhase {
        match self.operations.last() {
            Some(op) => match op.intent.request.operation {
                LifecycleOperation::Create { .. } if op.ready.is_some() => SessionPhase::Running,
                LifecycleOperation::Create { .. } => SessionPhase::Creating,
                LifecycleOperation::Destroy { .. } => SessionPhase::Terminating,
                _ => SessionPhase::Unloading,
            },
            None => SessionPhase::Creating, // rejected by full state validation
        }
    }
    #[must_use]
    pub fn current_version(&self) -> Option<u64> {
        self.operations.last().map(|op| op.intent.accepted_version)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleProfile {
    pub(crate) format_version: u16,
    pub(crate) center: CenterIdentity,
    pub(crate) windows: Vec<LifecycleWindow>,
    pub(crate) sessions: Vec<LifecycleSession>,
}

impl LifecycleProfile {
    fn find_operation(&self, key: &OperationKey) -> Option<&LifecycleOperationRecord> {
        for session in &self.sessions {
            if let Some(operation) = session.find_operation(key) {
                return Some(operation);
            }
        }
        None
    }
}

/// Checks new-key expected-version arithmetic; it never classifies a replay.
/// # Errors
/// Stale versions and exhaustion are separate, and neither changes the state.
pub fn next_lifecycle_version(expected: u64, current: u64) -> Result<u64, DecisionError> {
    if expected != current {
        return Err(DecisionError::Stale);
    }
    current.checked_add(1).ok_or(DecisionError::Exhausted)
}

impl CenterState {
    /// Explicit fresh provisioning only, never a missing-storage recovery path.
    /// This bounded slice has one protected window per registered owner, no
    /// rotation/retirement or Resume. New operations fail before effects at cap.
    /// # Errors
    /// Existing histories/profile, incomplete owner windows and foreign centers reject.
    pub fn enable_lifecycle(
        self,
        center: CenterIdentity,
        windows: Vec<LifecycleWindow>,
    ) -> Result<Self, DecisionError> {
        if !self.is_valid()
            || self.lifecycle.is_some()
            || !self.sandboxes.is_empty()
            || self.sandbox_highwater != 0
            || self.operation_highwater != 0
            || self.bounds.operations_per_sandbox < 3
            || center.deployment != self.deployment
        {
            return Err(DecisionError::Invalid);
        }
        let mut next = self;
        next.lifecycle = Some(LifecycleProfile {
            format_version: 1,
            center,
            windows,
            sessions: vec![],
        });
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok(next)
    }

    #[must_use]
    pub const fn lifecycle_enabled(&self) -> bool {
        self.lifecycle.is_some()
    }
    #[must_use]
    pub fn required_authority_bytes(&self) -> usize {
        self.bounds
            .required_block_bytes()
            .saturating_add(self.lifecycle_required_bytes())
    }
    pub(crate) fn lifecycle_manages(&self, sandbox: &SandboxId) -> bool {
        self.lifecycle
            .as_ref()
            .is_some_and(|p| p.sessions.iter().any(|s| s.original_sandbox == *sandbox))
    }
    pub(crate) fn lifecycle_required_bytes(&self) -> usize {
        if self.lifecycle.is_none() {
            return 0;
        }
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
            .saturating_add(8192)
    }
    #[must_use]
    pub fn lifecycle_session(
        &self,
        owner: &TenantId,
        session: &SessionId,
    ) -> Option<&LifecycleSession> {
        self.lifecycle.as_ref()?.sessions.iter().find(|s| {
            s.operations.first().is_some_and(|op| {
                op.intent.request.binding.key.owner == *owner
                    && op.intent.request.binding.session == *session
            })
        })
    }
    #[must_use]
    pub fn pending_lifecycle(&self) -> Vec<AcceptedLifecycleIntent> {
        self.lifecycle.as_ref().map_or_else(Vec::new, |p| {
            p.sessions
                .iter()
                .filter_map(|s| s.operations.last())
                .filter(|op| op.ready.is_none())
                .map(|op| op.intent.clone())
                .collect()
        })
    }

    /// Full-key replay is checked before current window/version/phase/time.
    /// New Create uses the original allocator, lane and resource reservation.
    /// # Errors
    /// All refusals leave this original state untouched; no host I/O occurs.
    pub fn accept_lifecycle(
        &self,
        owner: &TenantId,
        submission: &LifecycleSubmission,
        now_ms: u64,
    ) -> Result<(Self, AcceptedLifecycleIntent), DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let profile = self.lifecycle.as_ref().ok_or(DecisionError::Invalid)?;
        let key = submission.key();
        if key.owner != *owner || key.deployment != self.deployment {
            return Err(DecisionError::NotFound);
        }
        if let Some(operation) = profile.find_operation(key) {
            return operation
                .replay(submission)
                .map(|intent| (self.clone(), intent));
        }
        if !profile
            .windows
            .iter()
            .any(|w| w.owner == *owner && w.window == key.window)
        {
            return Err(DecisionError::Stale);
        }
        match submission {
            LifecycleSubmission::Create { parameters, .. } => {
                self.accept_lifecycle_create(submission, parameters, now_ms)
            }
            LifecycleSubmission::Unload {
                session,
                expected_version,
                ..
            } => {
                self.accept_lifecycle_followup(submission, owner, session, *expected_version, false)
            }
            LifecycleSubmission::Destroy {
                session,
                expected_version,
                ..
            } => {
                self.accept_lifecycle_followup(submission, owner, session, *expected_version, true)
            }
        }
    }

    fn accept_lifecycle_create(
        &self,
        submission: &LifecycleSubmission,
        parameters: &CreateParameters,
        now_ms: u64,
    ) -> Result<(Self, AcceptedLifecycleIntent), DecisionError> {
        if parameters.startup_timeout_ms == 0 {
            return Err(DecisionError::Invalid);
        }
        let create = CreateBinding {
            template_revision: parameters.template_revision.clone(),
            template_digest: parameters.template_digest,
            resources: parameters.resources,
            options: parameters.options.clone(),
            startup_deadline_ms: now_ms
                .checked_add(parameters.startup_timeout_ms)
                .ok_or(DecisionError::Exhausted)?,
            timeout_ms: parameters.timeout_ms,
            max_lifetime_ms: parameters.max_lifetime_ms,
        };
        let (mut next, allocation) = self.create_for_role(
            &submission.key().owner,
            create.clone(),
            now_ms,
            NodeRole::Runtime,
        )?;
        let profile = next.lifecycle.as_mut().ok_or(DecisionError::Invalid)?;
        let intent = AcceptedLifecycleIntent {
            center: profile.center.clone(),
            accepted_version: 1,
            request: LifecycleRequest {
                binding: OperationBinding {
                    protocol_version: sandproof_protocol::lifecycle::PROTOCOL_VERSION,
                    node: allocation.identity.node,
                    session: SessionId::from_protocol_id(allocation.identity.sandbox_id.clone()),
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
        profile.sessions.push(LifecycleSession {
            original_sandbox: allocation.identity.sandbox_id,
            created_at_ms: now_ms,
            operations: vec![LifecycleOperationRecord {
                submission: submission.clone(),
                intent: intent.clone(),
                ready: None,
            }],
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
        destroy: bool,
    ) -> Result<(Self, AcceptedLifecycleIntent), DecisionError> {
        let previous = self
            .lifecycle_session(owner, session)
            .ok_or(DecisionError::NotFound)?;
        let prior = previous.operations.last().ok_or(DecisionError::Invalid)?;
        let version = next_lifecycle_version(expected, prior.intent.accepted_version)?;
        let cap = usize::from(self.bounds.operations_per_sandbox);
        if previous.operations.len() >= cap - usize::from(!destroy) {
            return Err(DecisionError::Capacity);
        }
        if previous.phase() == SessionPhase::Terminating
            || (!destroy && previous.phase() != SessionPhase::Running)
        {
            return Err(DecisionError::Conflict);
        }
        let original = previous.original_sandbox.clone();
        let mut request = prior.intent.request.clone();
        request.binding.key = submission.key().clone();
        request.binding.expected = ExpectedSession::Version(expected);
        request.operation = if destroy {
            LifecycleOperation::Destroy {}
        } else {
            LifecycleOperation::Unload {
                source_incarnation: IncarnationId::try_from(1)
                    .map_err(|_| DecisionError::Invalid)?,
                capture: CaptureId::try_from(1).map_err(|_| DecisionError::Invalid)?,
            }
        };
        let intent = AcceptedLifecycleIntent {
            request,
            accepted_version: version,
            center: prior.intent.center.clone(),
        };
        let mut next = if destroy {
            let index = self.index(owner, &original)?;
            self.append_candidate(
                index,
                sandproof_protocol::internal::lifecycle::Intent::Destroy,
            )?
            .0
        } else {
            self.clone()
        };
        let target = next
            .lifecycle
            .as_mut()
            .ok_or(DecisionError::Invalid)?
            .sessions
            .iter_mut()
            .find(|s| s.original_sandbox == original)
            .ok_or(DecisionError::NotFound)?;
        target.operations.push(LifecycleOperationRecord {
            submission: submission.clone(),
            intent: intent.clone(),
            ready: None,
        });
        if !next.is_valid() {
            return Err(DecisionError::Invalid);
        }
        Ok((next, intent))
    }

    /// Bind an authenticated host's Ready observation to the exact current v3
    /// intent and original allocation. No legacy HTTP response can call through
    /// this path without the full canonical envelope supplied by its adapter.
    /// # Errors
    /// Superseded/non-Create envelopes or invalid original lifetime facts reject.
    pub fn observe_lifecycle_ready(
        &self,
        intent: &AcceptedLifecycleIntent,
        observation_version: PositiveCounter,
        started_at_ms: u64,
        end_at_ms: u64,
        max_end_at_ms: u64,
    ) -> Result<Self, DecisionError> {
        let session = self
            .lifecycle_session(
                &intent.request.binding.key.owner,
                &intent.request.binding.session,
            )
            .ok_or(DecisionError::NotFound)?;
        let current = session.operations.last().ok_or(DecisionError::Invalid)?;
        if current.intent != *intent
            || !matches!(intent.request.operation, LifecycleOperation::Create { .. })
        {
            return Err(DecisionError::Stale);
        }
        let index = self.index(&intent.request.binding.key.owner, &session.original_sandbox)?;
        let request = &self.sandboxes[index].operations[0].request;
        let response = LifecycleResponse {
            identity: request.identity.clone(),
            observation_version,
            fact: LifecycleFact::Ready {
                started_at_ms,
                end_at_ms,
                max_end_at_ms,
            },
        };
        self.apply_response_inner(&response)
    }

    pub(crate) fn bind_lifecycle_ready(
        &mut self,
        response: &LifecycleResponse,
    ) -> Result<(), DecisionError> {
        let Some(profile) = self.lifecycle.as_mut() else {
            return Ok(());
        };
        let Some(session) = profile
            .sessions
            .iter_mut()
            .find(|s| s.original_sandbox == response.identity.sandbox_id)
        else {
            return Ok(());
        };
        let operation = session
            .operations
            .last_mut()
            .ok_or(DecisionError::Invalid)?;
        if !matches!(
            operation.intent.request.operation,
            LifecycleOperation::Create { .. }
        ) || !matches!(response.fact, LifecycleFact::Ready { .. })
        {
            return Err(DecisionError::Stale);
        }
        operation.ready = Some(response.clone());
        Ok(())
    }
}
