//! Canonical identities and accepted intent order. Local lifecycle phase stays
//! in the original World; these rows do not invent another phase authority.
use crate::{InternalLocalBinding, JournalError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, CreateRefusalReason,
    ExpectedSession, IncarnationId, LifecycleOperation, SessionId,
};
use sandproof_protocol::{TenantId, internal::NodeIdentity};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleInboxConfig {
    pub center: CenterIdentity,
    pub node: NodeIdentity,
    /// Protected registration reference, never supplied by a lifecycle body.
    /// None permits recording/fencing but refuses native V3 create admission.
    pub host_epoch: Option<crate::capture::HostEpochRegistration>,
    pub max_sessions: u16,
    /// Finite retained operations; the last position is reserved for Destroy.
    pub max_operations_per_session: u16,
    pub max_options_bytes: u32,
    pub max_profile_bytes: u32,
    pub max_manifest_bytes: u32,
}
impl LifecycleInboxConfig {
    #[must_use]
    pub fn required_bytes(&self) -> u64 {
        // Each finite position keeps its bounded operation history, an explicit
        // native mapping, one capture work and immutable bundle receipt.
        // The final operation position is reserved for terminal responsibility.
        u64::from(self.max_sessions)
            * (4 * u64::from(self.max_options_bytes)
                + u64::from(self.max_operations_per_session) * 8192
                + 2 * u64::from(self.max_manifest_bytes)
                + u64::from(self.max_profile_bytes)
                + 3 * 8192)
            + 8192
    }
    pub fn valid(&self) -> bool {
        self.center.deployment == self.node.deployment_id
            && self
                .host_epoch
                .as_ref()
                .is_none_or(crate::capture::HostEpochRegistration::valid)
            && (1..=4096).contains(&self.max_sessions)
            && (8..=128).contains(&self.max_operations_per_session)
            && (1..=65_536).contains(&self.max_options_bytes)
            && (1..=65_536).contains(&self.max_profile_bytes)
            && (1..=262_144).contains(&self.max_manifest_bytes)
            && self.required_bytes() <= 16 * 1024 * 1024
    }
    pub(super) fn context(&self) -> AuthenticatedCenterContext {
        AuthenticatedCenterContext::from_verified_peer_registration(
            self.center.clone(),
            self.node.clone(),
        )
    }
}

/// Data returned for query/assembly, never an execution or release permit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleWorkState {
    pub intent: AcceptedLifecycleIntent,
    pub current_session_version: u64,
    pub current: bool,
    pub terminal: bool,
    /// Original pre-admission refusal; no release or refund permission.
    pub refusal: Option<CreateRefusalReason>,
    pub local: Option<InternalLocalBinding>,
    pub local_operation: Option<sandbox_controller_core::OperationId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub intent: AcceptedLifecycleIntent,
    pub local_operation: Option<sandbox_controller_core::OperationId>,
    /// Exact effective deadline committed in the same transaction as renewal.
    pub applied_deadline: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Session {
    pub id: SessionId,
    pub retention: sandproof_protocol::lifecycle::RetentionBinding,
    pub owner: TenantId,
    pub current_version: u64,
    pub terminal: bool,
    pub refusal: Option<CreateRefusalReason>,
    pub incarnation: Option<IncarnationId>,
    pub local: Option<InternalLocalBinding>,
    pub entries: Vec<Entry>,
    pub confirmed_terminal: Option<sandproof_protocol::lifecycle::TerminalLifecycleObservation>,
}

/// Same original per-position retirement frontier and one complete recent
/// stamp. This is part of the existing inbox, never another lifecycle phase.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleRetentionLane {
    pub slot: u16,
    pub retired_through: u64,
    pub current_window: Option<sandproof_protocol::lifecycle::RetentionWindowId>,
    pub last_retired: Option<sandproof_protocol::lifecycle::TerminalLifecycleObservation>,
}
impl LifecycleRetentionLane {
    pub(super) fn fresh(slot: u16) -> Self {
        Self {
            slot,
            retired_through: 0,
            current_window: Some(1.try_into().expect("positive literal")),
            last_retired: None,
        }
    }
    fn valid(&self, context: &AuthenticatedCenterContext) -> bool {
        let successor = self
            .retired_through
            .checked_add(1)
            .and_then(|next| next.try_into().ok());
        if self.current_window != successor {
            return false;
        }
        match (&self.last_retired, self.retired_through) {
            (None, 0) => true,
            (Some(last), floor) if floor > 0 => {
                last.status == sandproof_protocol::lifecycle::TerminalStatus::Cleaned
                    && last.matches_request(&last.request)
                    && last.request.validate(context).is_ok()
                    && last.request.retention.slot == self.slot
                    && last.request.retention.window.get() == floor
                    && serde_json::to_vec(last).is_ok_and(|bytes| bytes.len() <= 8192)
            }
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Owner {
    pub external: TenantId,
    pub local: sandbox_controller_core::TenantId,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inbox {
    pub config: LifecycleInboxConfig,
    pub host_epoch_observation: Option<crate::capture::HostEpochObservation>,
    pub owner_high_water: u64,
    pub owners: Vec<Owner>,
    pub sessions: Vec<Session>,
    pub lanes: Vec<LifecycleRetentionLane>,
}
impl Inbox {
    pub(super) fn verify_context(
        &self,
        context: &AuthenticatedCenterContext,
    ) -> Result<(), JournalError> {
        if self.config.center != *context.center() || self.config.node != *context.expected_node() {
            return Err(JournalError::Conflict);
        }
        Ok(())
    }

    pub(super) fn find(
        &self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<Option<(usize, usize)>, JournalError> {
        for (s, session) in self.sessions.iter().enumerate() {
            for (e, entry) in session.entries.iter().enumerate() {
                if entry.intent.request.binding.key == intent.request.binding.key {
                    if entry.intent != *intent {
                        return Err(JournalError::Conflict);
                    }
                    return Ok(Some((s, e)));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn state(&self, s: usize, e: usize) -> LifecycleWorkState {
        let session = &self.sessions[s];
        let entry = &session.entries[e];
        LifecycleWorkState {
            intent: entry.intent.clone(),
            current_session_version: session.current_version,
            current: entry.intent.accepted_version == session.current_version,
            terminal: session.terminal,
            refusal: session.refusal,
            local: session.local,
            local_operation: entry.local_operation,
        }
    }

    // Representation/shape validation only. The receiver verifies TLS context
    // before calling this; constructing context values does not authenticate.
    pub(super) fn validate(&self) -> Result<(), JournalError> {
        if !self.config.valid()
            || self.sessions.len() > usize::from(self.config.max_sessions)
            || self.owners.len() > usize::from(self.config.max_sessions)
        {
            return Err(JournalError::CorruptStore);
        }
        let context = self.config.context();
        if self.lanes.len() != usize::from(self.config.max_sessions)
            || self
                .lanes
                .iter()
                .enumerate()
                .any(|(index, lane)| usize::from(lane.slot) != index + 1 || !lane.valid(&context))
        {
            return Err(JournalError::CorruptStore);
        }
        if self.host_epoch_observation.as_ref().is_some_and(|o| {
            o.node != self.config.node || Some(&o.registration) != self.config.host_epoch.as_ref()
        }) {
            return Err(JournalError::CorruptStore);
        }
        for (i, owner) in self.owners.iter().enumerate() {
            if owner.local.0 > self.owner_high_water
                || self.owners[..i]
                    .iter()
                    .any(|old| old.external == owner.external || old.local == owner.local)
            {
                return Err(JournalError::CorruptStore);
            }
        }
        for (i, session) in self.sessions.iter().enumerate() {
            self.validate_session_shape(i, session)?;
            let mut maximum = 0;
            let mut kinds = [0_u16; 4];
            for entry in &session.entries {
                let intent = &entry.intent;
                if intent.accepted_version <= maximum
                    || !matches!(intent.request.operation, LifecycleOperation::Destroy {})
                        && intent.request.binding.expected
                            != if maximum == 0 {
                                ExpectedSession::Absent
                            } else {
                                ExpectedSession::Version(maximum)
                            }
                {
                    return Err(JournalError::CorruptStore);
                }
                maximum = maximum.max(intent.accepted_version);
                let kind = self.entry_kind(session, entry, &context)?;
                kinds[kind] += 1;
                if kind != 3 && kinds[kind] > 1 {
                    return Err(JournalError::CorruptStore);
                }
            }
            if session.current_version != maximum
                || session.terminal != (kinds[2] == 1)
                || session.local.is_some() && kinds[0] != 1
                || session.incarnation.is_some() != (kinds[0] == 1)
                || session.terminal
                    && !session.entries.iter().any(|e| {
                        matches!(e.intent.request.operation, LifecycleOperation::Destroy {})
                            && e.intent.accepted_version == maximum
                    })
                || session.local.is_some_and(|local| {
                    !self
                        .owners
                        .iter()
                        .any(|o| o.external == session.owner && o.local == local.owner)
                })
            {
                return Err(JournalError::CorruptStore);
            }
        }
        Ok(())
    }

    fn validate_session_shape(&self, index: usize, session: &Session) -> Result<(), JournalError> {
        if session.entries.is_empty()
            || session.retention.slot == 0
            || session.retention.slot > self.config.max_sessions
            || self
                .lanes
                .iter()
                .find(|lane| lane.slot == session.retention.slot)
                .is_none_or(|lane| lane.current_window != Some(session.retention.window))
            || session.local.is_some() && self.host_epoch_observation.is_none()
            || session.entries.len() > usize::from(self.config.max_operations_per_session)
            || self.sessions[..index].iter().any(|old| {
                old.id == session.id
                    || old.retention.slot == session.retention.slot
                    || session.local.is_some_and(|local| {
                        old.local
                            .is_some_and(|prior| prior.sandbox == local.sandbox)
                    })
            })
        {
            return Err(JournalError::CorruptStore);
        }
        if session.refusal.is_some()
            && (session.local.is_some()
                || session.entries.first().is_none_or(|entry| {
                    entry.intent.accepted_version != 1
                        || !matches!(
                            entry.intent.request.operation,
                            LifecycleOperation::Create { .. }
                        )
                })
                || session.entries.iter().any(|entry| {
                    entry.local_operation.is_some()
                        || entry.applied_deadline.is_some()
                        || !matches!(
                            entry.intent.request.operation,
                            LifecycleOperation::Create { .. } | LifecycleOperation::Destroy {}
                        )
                }))
        {
            return Err(JournalError::CorruptStore);
        }
        if let Some(confirmed) = &session.confirmed_terminal
            && (!session.terminal
                || confirmed.status != sandproof_protocol::lifecycle::TerminalStatus::Cleaned
                || !confirmed.matches_request(&confirmed.request)
                || session
                    .entries
                    .last()
                    .is_none_or(|entry| entry.intent != confirmed.request)
                || serde_json::to_vec(confirmed)?.len() > 8192)
        {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }

    fn entry_kind(
        &self,
        session: &Session,
        entry: &Entry,
        context: &AuthenticatedCenterContext,
    ) -> Result<usize, JournalError> {
        let intent = &entry.intent;
        let binding = &intent.request.binding;
        if intent.validate(context).is_err()
            || intent.retention != session.retention
            || binding.session != session.id
            || binding.key.owner != session.owner
            || session.local.is_none() && entry.local_operation.is_some()
            || !matches!(intent.request.operation, LifecycleOperation::Renew { .. })
                && entry.applied_deadline.is_some()
            || self
                .sessions
                .iter()
                .flat_map(|s| &s.entries)
                .filter(|e| e.intent.request.binding.key == binding.key)
                .count()
                != 1
        {
            return Err(JournalError::CorruptStore);
        }
        match &intent.request.operation {
            LifecycleOperation::Create {
                create,
                target_incarnation,
            } => {
                if create.options.len() > self.config.max_options_bytes as usize
                    || session.incarnation != Some(*target_incarnation)
                {
                    return Err(JournalError::CorruptStore);
                }
                Ok(0)
            }
            LifecycleOperation::Unload {
                source_incarnation, ..
            } => {
                if session.incarnation != Some(*source_incarnation) || session.local.is_none() {
                    return Err(JournalError::CorruptStore);
                }
                Ok(1)
            }
            LifecycleOperation::Destroy {} => Ok(2),
            LifecycleOperation::Renew {
                incarnation,
                end_at_ms,
                extend_only,
            } => {
                if session.incarnation != Some(*incarnation)
                    || session.local.is_none()
                    || *end_at_ms < 0
                    || entry.applied_deadline.is_some_and(|end| {
                        if *extend_only {
                            end < *end_at_ms
                        } else {
                            end != *end_at_ms
                        }
                    })
                {
                    return Err(JournalError::CorruptStore);
                }
                Ok(3)
            }
            LifecycleOperation::Resume { .. } => Err(JournalError::UnsupportedInput),
        }
    }

    pub(super) fn record(
        &mut self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<(usize, usize), JournalError> {
        if let Some(found) = self.find(intent)? {
            return Ok(found);
        }
        let request = &intent.request;
        if matches!(request.operation, LifecycleOperation::Resume { .. }) {
            return Err(JournalError::UnsupportedInput);
        }
        let existing = self
            .sessions
            .iter()
            .position(|s| s.id == request.binding.session);
        if let Some(s) = existing {
            return self.record_existing(s, intent);
        }
        self.record_fresh(intent)
    }

    fn record_existing(
        &mut self,
        s: usize,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<(usize, usize), JournalError> {
        let request = &intent.request;
        let session = &mut self.sessions[s];
        if session.owner != request.binding.key.owner || session.terminal {
            return Err(JournalError::Conflict);
        }
        if session.retention != intent.retention {
            return Err(JournalError::Conflict);
        }
        let terminal = matches!(request.operation, LifecycleOperation::Destroy {});
        if (!terminal
            && request.binding.expected != ExpectedSession::Version(session.current_version))
            || (terminal && intent.accepted_version <= session.current_version)
            || session.entries.len() >= usize::from(self.config.max_operations_per_session)
            || !terminal
                && session.entries.len() + 1 >= usize::from(self.config.max_operations_per_session)
        {
            return Err(JournalError::Conflict);
        }
        match &request.operation {
            LifecycleOperation::Unload {
                source_incarnation, ..
            } if session.local.is_some()
                && session.incarnation == Some(*source_incarnation)
                && !session.entries.iter().any(|e| {
                    matches!(
                        e.intent.request.operation,
                        LifecycleOperation::Unload { .. }
                    )
                }) => {}
            LifecycleOperation::Destroy {} => {}
            LifecycleOperation::Renew {
                incarnation,
                end_at_ms,
                ..
            } if session.local.is_some()
                && session.incarnation == Some(*incarnation)
                && *end_at_ms >= 0
                && !session.entries.iter().any(|e| {
                    matches!(
                        e.intent.request.operation,
                        LifecycleOperation::Unload { .. }
                    )
                }) => {}
            _ => return Err(JournalError::UnsupportedInput),
        }
        session.current_version = intent.accepted_version;
        session.terminal = terminal;
        session.entries.push(Entry {
            intent: intent.clone(),
            local_operation: None,
            applied_deadline: None,
        });
        Ok((s, session.entries.len() - 1))
    }

    fn record_fresh(
        &mut self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<(usize, usize), JournalError> {
        let request = &intent.request;
        if intent.retention.slot == 0 || intent.retention.slot > self.config.max_sessions {
            return Err(JournalError::Conflict);
        }
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.slot == intent.retention.slot)
            .ok_or(JournalError::Conflict)?;
        if lane.current_window != Some(intent.retention.window) {
            return Err(JournalError::Conflict);
        }
        if self.sessions.len() >= usize::from(self.config.max_sessions)
            || self
                .sessions
                .iter()
                .any(|session| session.retention.slot == intent.retention.slot)
        {
            return Err(JournalError::MaintenanceCapacity);
        }
        let (terminal, incarnation) = match &request.operation {
            LifecycleOperation::Create {
                create,
                target_incarnation,
            } if create.options.len() <= self.config.max_options_bytes as usize => {
                (false, Some(*target_incarnation))
            }
            LifecycleOperation::Destroy {} => (true, None),
            _ => return Err(JournalError::UnsupportedInput),
        };
        self.sessions.push(Session {
            id: request.binding.session.clone(),
            retention: intent.retention,
            owner: request.binding.key.owner.clone(),
            current_version: intent.accepted_version,
            terminal,
            refusal: None,
            incarnation,
            local: None,
            entries: vec![Entry {
                intent: intent.clone(),
                local_operation: None,
                applied_deadline: None,
            }],
            confirmed_terminal: None,
        });
        Ok((self.sessions.len() - 1, 0))
    }
}
