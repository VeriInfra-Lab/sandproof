//! Canonical identities and accepted intent order. Local lifecycle phase stays
//! in the original World; these rows do not invent another phase authority.
use crate::{InternalLocalBinding, JournalError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, CreateRefusalReason,
    ExpectedSession, IncarnationId, LifecycleOperation, SessionId,
};
use sandproof_protocol::{TenantId, internal::NodeIdentity};
use serde::{Deserialize, Serialize};

// Missing fields are not an implicit upgrade from an old developer format.
// Explicit null is allowed where no application/confirmation exists yet.
fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

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
                // The full latest SDK renewal confirmation has its own fixed
                // reservation, alongside terminal confirmation/retirement.
                + 4 * 8192)
            + 8192
    }
    #[must_use]
    pub fn valid(&self) -> bool {
        self.valid_using::<crate::capture::StandardGraphicBytes>()
    }
    #[must_use]
    fn valid_using<B: crate::capture::GraphicBytes>(&self) -> bool {
        self.center.deployment == self.node.deployment_id
            && match self.host_epoch.as_ref() {
                None => true,
                Some(registration) => registration.valid_using::<B>(),
            }
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
    /// Actual native deadline read before that same renewal transaction.
    /// Kept even if its previously confirmed SDK predecessor is compacted.
    #[serde(deserialize_with = "required_option")]
    pub previous_effective_end_at_ms: Option<i64>,
    #[serde(deserialize_with = "required_option")]
    pub unload_resolution: Option<super::unload_resolution::UnloadResolution>,
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
    #[serde(deserialize_with = "required_option")]
    pub confirmed_renewal: Option<sandproof_protocol::lifecycle::RenewalConfirmation>,
    /// Greatest SDK version actually removed by the exact confirmation rule.
    /// This is not the latest accepted/confirmed version or a global key floor.
    #[serde(deserialize_with = "required_option")]
    pub compacted_sdk_through: Option<u64>,
    #[serde(deserialize_with = "required_option")]
    pub native_retirement: Option<super::native_retirement::NativeRetirementReceipt>,
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
pub(super) trait TerminalEncoding {
    fn encode(
        value: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
    ) -> Result<Vec<u8>, serde_json::Error>;
}
pub(super) struct StandardTerminalEncoding;
impl TerminalEncoding for StandardTerminalEncoding {
    fn encode(
        value: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }
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
    fn valid_using<E: TerminalEncoding>(&self, context: &AuthenticatedCenterContext) -> bool {
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
                    && terminal_encoded_within_limit::<E>(last)
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
            if let Some(e) = session.find_entry(intent)? {
                return Ok(Some((s, e)));
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
        self.validate_using::<
            super::native_retirement::StandardRetirementEncoding,
            StandardTerminalEncoding,
            super::renewal_confirmation::StandardRenewalEncoding,
            super::unload_resolution::StandardUnloadEncoding,
            crate::capture::StandardGraphicBytes,
        >()
    }

    pub(super) fn validate_using<
        R: super::native_retirement::RetirementEncoding,
        T: TerminalEncoding,
        N: super::renewal_confirmation::RenewalEncoding,
        U: super::unload_resolution::UnloadEncoding,
        B: crate::capture::GraphicBytes,
    >(
        &self,
    ) -> Result<(), JournalError> {
        if !self.config.valid_using::<B>()
            || self.sessions.len() > usize::from(self.config.max_sessions)
            || self.owners.len() > usize::from(self.config.max_sessions)
        {
            return Err(JournalError::CorruptStore);
        }
        let context = self.config.context();
        if self.lanes.len() != usize::from(self.config.max_sessions)
            || self.lanes_invalid::<T>(&context)
        {
            return Err(JournalError::CorruptStore);
        }
        self.validate_retired_identities()?;
        if self.host_epoch_observation.as_ref().is_some_and(|o| {
            o.node != self.config.node || Some(&o.registration) != self.config.host_epoch.as_ref()
        }) {
            return Err(JournalError::CorruptStore);
        }
        self.validate_owners()?;
        for (i, session) in self.sessions.iter().enumerate() {
            self.validate_one_session::<R, T, N, U>(i, session, &context)?;
        }
        Ok(())
    }

    fn validate_owners(&self) -> Result<(), JournalError> {
        for (i, owner) in self.owners.iter().enumerate() {
            if owner.local.0 > self.owner_high_water || self.prior_owner_conflict(i, owner) {
                return Err(JournalError::CorruptStore);
            }
        }
        Ok(())
    }

    fn validate_one_session<
        R: super::native_retirement::RetirementEncoding,
        T: TerminalEncoding,
        N: super::renewal_confirmation::RenewalEncoding,
        U: super::unload_resolution::UnloadEncoding,
    >(
        &self,
        i: usize,
        session: &Session,
        context: &AuthenticatedCenterContext,
    ) -> Result<(), JournalError> {
        self.validate_session_shape_using::<R, T>(i, session)?;
        super::renewal_confirmation::validate_session_using::<N>(session)?;
        let (maximum, kinds) = self.validate_entry_order_using::<U>(session, context)?;
        session.validate_execution_lineage()?;
        if session.current_version != maximum
            || session.terminal != (kinds[2] == 1)
            || session.local.is_some() && kinds[0] != 1
            || session.incarnation.is_some() != (kinds[0] == 1)
            || session.terminal && !has_destroy_at_maximum(session, maximum)
            || session
                .local
                .is_some_and(|local| !self.has_local_owner(session, local.owner))
        {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }

    fn prior_retired_identity_conflict(
        &self,
        index: usize,
        id: &SessionId,
        key: &sandproof_protocol::lifecycle::OperationKey,
    ) -> bool {
        for other in &self.lanes[..index] {
            if let Some(last) = &other.last_retired
                && (last.request.request.binding.session == *id
                    || last.request.request.binding.key == *key)
            {
                return true;
            }
        }
        false
    }

    fn current_identity_conflict(
        &self,
        id: &SessionId,
        key: &sandproof_protocol::lifecycle::OperationKey,
    ) -> bool {
        for session in &self.sessions {
            if session.id == *id || session.contains_entry_key(key) {
                return true;
            }
        }
        false
    }

    fn validate_retired_identities(&self) -> Result<(), JournalError> {
        for (index, lane) in self.lanes.iter().enumerate() {
            let Some(retired) = &lane.last_retired else {
                continue;
            };
            let binding = &retired.request.request.binding;
            if self.prior_retired_identity_conflict(index, &binding.session, &binding.key)
                || self.current_identity_conflict(&binding.session, &binding.key)
            {
                return Err(JournalError::CorruptStore);
            }
        }
        Ok(())
    }

    #[expect(
        clippy::explicit_iter_loop,
        reason = "fixed extraction uses the registered slice iterator"
    )]
    fn recently_retired_identity(&self, intent: &AcceptedLifecycleIntent) -> bool {
        for lane in self.lanes.iter() {
            if lane.last_retired.as_ref().is_some_and(|last| {
                last.request.request.binding.session == intent.request.binding.session
                    || last.request.request.binding.key == intent.request.binding.key
            }) {
                return true;
            }
        }
        false
    }

    pub(super) fn session_index(&self, id: &SessionId) -> Option<usize> {
        for (index, session) in self.sessions.iter().enumerate() {
            if session.id == *id {
                return Some(index);
            }
        }
        None
    }

    fn retention_lane_index(&self, slot: u16) -> Option<usize> {
        for (index, lane) in self.lanes.iter().enumerate() {
            if lane.slot == slot {
                return Some(index);
            }
        }
        None
    }

    #[expect(
        clippy::explicit_iter_loop,
        reason = "fixed extraction uses the registered slice iterator"
    )]
    fn retention_slot_used(&self, slot: u16) -> bool {
        for session in self.sessions.iter() {
            if session.retention.slot == slot {
                return true;
            }
        }
        false
    }

    fn validate_session_shape_using<
        R: super::native_retirement::RetirementEncoding,
        E: TerminalEncoding,
    >(
        &self,
        index: usize,
        session: &Session,
    ) -> Result<(), JournalError> {
        super::native_retirement::validate_saved_using::<R>(session)?;
        if session.entries.is_empty()
            || session.retention.slot == 0
            || session.retention.slot > self.config.max_sessions
            || !first_lane_window_matches(self, session)
            || session.local.is_some() && self.host_epoch_observation.is_none()
            || session.entries.len() > usize::from(self.config.max_operations_per_session)
            || prior_session_conflict(self, index, session)
        {
            return Err(JournalError::CorruptStore);
        }
        if session.refusal.is_some()
            && (session.local.is_some()
                || first_session_entry(session).is_none_or(|entry| {
                    entry.intent.accepted_version != 1
                        || !matches!(
                            entry.intent.request.operation,
                            LifecycleOperation::Create { .. }
                        )
                })
                || refusal_entries_invalid(session))
        {
            return Err(JournalError::CorruptStore);
        }
        if let Some(confirmed) = &session.confirmed_terminal
            && (!session.terminal
                || confirmed.status != sandproof_protocol::lifecycle::TerminalStatus::Cleaned
                || !confirmed.matches_request(&confirmed.request)
                || session
                    .last_entry()
                    .is_none_or(|entry| entry.intent != confirmed.request)
                || E::encode(confirmed)?.len() > 8192)
        {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }

    fn count_entry_key(&self, key: &sandproof_protocol::lifecycle::OperationKey) -> usize {
        let mut count = 0_usize;
        for session in &self.sessions {
            count = session.count_entry_key_from(key, count);
        }
        count
    }

    fn entry_kind_using<E: super::unload_resolution::UnloadEncoding>(
        &self,
        session: &Session,
        entry: &Entry,
        context: &AuthenticatedCenterContext,
    ) -> Result<usize, JournalError> {
        super::unload_resolution::validate_entry_using::<E>(entry)?;
        let intent = &entry.intent;
        let binding = &intent.request.binding;
        if intent.validate(context).is_err()
            || intent.retention != session.retention
            || binding.session != session.id
            || binding.key.owner != session.owner
            || session.local.is_none() && entry.local_operation.is_some()
            || !matches!(intent.request.operation, LifecycleOperation::Renew { .. })
                && (entry.applied_deadline.is_some()
                    || entry.previous_effective_end_at_ms.is_some())
            || self.count_entry_key(&binding.key) != 1
        {
            return Err(JournalError::CorruptStore);
        }
        match &intent.request.operation {
            LifecycleOperation::Create { create, .. } => {
                // Logical incarnation is an opaque protocol identity; it is
                // not the local VM generation. Lineage below binds its actual
                // original value and each exact successor.
                if create.options.len() > self.config.max_options_bytes as usize {
                    return Err(JournalError::CorruptStore);
                }
                Ok(0)
            }
            LifecycleOperation::Unload {
                source_incarnation, ..
            } => {
                if source_incarnation.get() == 0 || session.local.is_none() {
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
                if incarnation.get() == 0
                    || session.local.is_none()
                    || *end_at_ms < 0
                    || entry.applied_deadline.is_some()
                        != entry.previous_effective_end_at_ms.is_some()
                    || entry
                        .previous_effective_end_at_ms
                        .is_some_and(|end| end < 0)
                    || entry.applied_deadline.is_some_and(|end| {
                        if *extend_only {
                            entry.previous_effective_end_at_ms.is_none_or(|previous| {
                                end != (if previous > *end_at_ms {
                                    previous
                                } else {
                                    *end_at_ms
                                })
                            })
                        } else {
                            end != *end_at_ms
                        }
                    })
                {
                    return Err(JournalError::CorruptStore);
                }
                Ok(3)
            }
            LifecycleOperation::Resume { .. } => Ok(4),
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
        let existing = self.session_index(&request.binding.session);
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
                && session.can_follow_running() => {}
            LifecycleOperation::Resume {
                source_incarnation,
                capture,
                target_incarnation,
                ..
            } if session.local.is_some()
                && session.incarnation == Some(*source_incarnation)
                && source_incarnation.checked_next().ok() == Some(*target_incarnation)
                && session.last_entry().is_some_and(|prior| {
                    matches!(prior.intent.request.operation,
                        LifecycleOperation::Unload { source_incarnation: old, capture: old_capture }
                        if old == *source_incarnation && old_capture == *capture)
                }) => {}
            LifecycleOperation::Destroy {} => {}
            LifecycleOperation::Renew {
                incarnation,
                end_at_ms,
                ..
            } if session.local.is_some()
                && session.incarnation == Some(*incarnation)
                && *end_at_ms >= 0
                && session.can_follow_running() => {}
            _ => return Err(JournalError::UnsupportedInput),
        }
        // A later public row must not hide an earlier unconfirmed SDK renewal.
        // This gate concerns only a NEW SDK renewal; Destroy always retains its
        // reserved path, and exact retained replays were returned above.
        if super::renewal_confirmation::is_sdk_renewal(intent)
            && super::renewal_confirmation::has_unconfirmed_sdk(session)
        {
            return Err(JournalError::UnresolvedAction);
        }
        session.current_version = intent.accepted_version;
        session.terminal = terminal;
        session.entries.push(Entry {
            intent: intent.clone(),
            local_operation: None,
            applied_deadline: None,
            previous_effective_end_at_ms: None,
            unload_resolution: None,
        });
        Ok((s, session.entries.len() - 1))
    }

    #[expect(
        clippy::vec_init_then_push,
        reason = "fixed extraction cannot lower vec! MaybeUninit construction"
    )]
    fn record_fresh(
        &mut self,
        intent: &AcceptedLifecycleIntent,
    ) -> Result<(usize, usize), JournalError> {
        let request = &intent.request;
        if self.recently_retired_identity(intent) {
            return Err(JournalError::Conflict);
        }
        if intent.retention.slot == 0 || intent.retention.slot > self.config.max_sessions {
            return Err(JournalError::Conflict);
        }
        let lane_index = self
            .retention_lane_index(intent.retention.slot)
            .ok_or(JournalError::Conflict)?;
        let lane = &self.lanes[lane_index];
        if lane.current_window != Some(intent.retention.window) {
            return Err(JournalError::Conflict);
        }
        if self.sessions.len() >= usize::from(self.config.max_sessions)
            || self.retention_slot_used(intent.retention.slot)
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
            entries: {
                let mut entries = Vec::with_capacity(1);
                entries.push(Entry {
                    intent: intent.clone(),
                    local_operation: None,
                    applied_deadline: None,
                    previous_effective_end_at_ms: None,
                    unload_resolution: None,
                });
                entries
            },
            confirmed_terminal: None,
            confirmed_renewal: None,
            compacted_sdk_through: None,
            native_retirement: None,
        });
        Ok((self.sessions.len() - 1, 0))
    }
}

impl Session {
    fn find_entry(&self, intent: &AcceptedLifecycleIntent) -> Result<Option<usize>, JournalError> {
        for (e, entry) in self.entries.iter().enumerate() {
            if entry.intent.request.binding.key == intent.request.binding.key {
                if entry.intent != *intent {
                    return Err(JournalError::Conflict);
                }
                return Ok(Some(e));
            }
        }
        Ok(None)
    }

    fn count_entry_key_from(
        &self,
        key: &sandproof_protocol::lifecycle::OperationKey,
        mut count: usize,
    ) -> usize {
        for entry in &self.entries {
            if entry.intent.request.binding.key == *key {
                count += 1;
            }
        }
        count
    }

    fn contains_entry_key(&self, key: &sandproof_protocol::lifecycle::OperationKey) -> bool {
        for entry in &self.entries {
            if entry.intent.request.binding.key == *key {
                return true;
            }
        }
        false
    }

    pub(super) fn current_execution_origin(&self) -> Option<&Entry> {
        let mut index = self.entries.len();
        while index > 0 {
            index -= 1;
            let entry = &self.entries[index];
            if matches!(
                entry.intent.request.operation,
                LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }
            ) {
                return Some(entry);
            }
        }
        None
    }

    pub(super) fn last_entry(&self) -> Option<&Entry> {
        if self.entries.is_empty() {
            None
        } else {
            Some(&self.entries[self.entries.len() - 1])
        }
    }

    fn can_follow_running(&self) -> bool {
        self.last_entry()
            .is_some_and(|entry| match entry.intent.request.operation {
                LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. } => {
                    entry.local_operation.is_some()
                }
                // A newer canonical request may fence an unapplied renewal;
                // receiving Renew did not replace the original running VM.
                // The SDK acknowledgement gate below remains separate.
                LifecycleOperation::Renew { .. } => true,
                LifecycleOperation::Unload { .. } => entry
                    .unload_resolution
                    .as_ref()
                    .is_some_and(|r| r.result().is_some()),
                LifecycleOperation::Destroy { .. } => false,
            })
    }

    /// Historical entries keep their own incarnation. Only a durably attached
    /// Resume changes the native mapping; receiving its DTO does not do so.
    fn validate_execution_lineage(&self) -> Result<(), JournalError> {
        let mut incarnation = None;
        let mut previous: Option<&Entry> = None;
        let mut capture_frontier = 0;
        for entry in &self.entries {
            match entry.intent.request.operation {
                LifecycleOperation::Create {
                    target_incarnation, ..
                } => {
                    if incarnation.is_some() || previous.is_some() {
                        return Err(JournalError::CorruptStore);
                    }
                    incarnation = Some(target_incarnation);
                }
                LifecycleOperation::Unload {
                    source_incarnation,
                    capture,
                } => {
                    if incarnation != Some(source_incarnation) || capture.get() <= capture_frontier
                    {
                        return Err(JournalError::CorruptStore);
                    }
                    capture_frontier = capture.get();
                }
                LifecycleOperation::Resume {
                    source_incarnation,
                    capture,
                    target_incarnation,
                    ..
                } => {
                    if incarnation != Some(source_incarnation)
                        || source_incarnation.checked_next().ok() != Some(target_incarnation)
                        || previous.is_none_or(|prior| !matches!(prior.intent.request.operation,
                            LifecycleOperation::Unload { source_incarnation: old, capture: old_capture }
                            if old == source_incarnation && old_capture == capture)) {
                        return Err(JournalError::CorruptStore);
                    }
                    if entry.local_operation.is_some() {
                        incarnation = Some(target_incarnation);
                    }
                }
                LifecycleOperation::Renew {
                    incarnation: current,
                    ..
                } => {
                    if incarnation != Some(current) {
                        return Err(JournalError::CorruptStore);
                    }
                }
                LifecycleOperation::Destroy {} => {}
            }
            previous = Some(entry);
        }
        if self.incarnation != incarnation {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }
}

fn first_lane_window_matches(inbox: &Inbox, session: &Session) -> bool {
    let mut position = 0;
    while position < inbox.lanes.len() {
        let lane = &inbox.lanes[position];
        if lane.slot == session.retention.slot {
            return lane.current_window == Some(session.retention.window);
        }
        position += 1;
    }
    false
}

fn prior_session_conflict(inbox: &Inbox, index: usize, session: &Session) -> bool {
    let previous = &inbox.sessions[..index];
    let mut position = 0;
    while position < previous.len() {
        let old = &previous[position];
        if old.id == session.id
            || old.retention.slot == session.retention.slot
            || session.local.is_some_and(|local| {
                old.local
                    .is_some_and(|prior| prior.sandbox == local.sandbox)
            })
        {
            return true;
        }
        position += 1;
    }
    false
}

fn refusal_entries_invalid(session: &Session) -> bool {
    let mut position = 0;
    while position < session.entries.len() {
        let entry = &session.entries[position];
        if entry.local_operation.is_some()
            || entry.applied_deadline.is_some()
            || entry.previous_effective_end_at_ms.is_some()
            || !matches!(
                entry.intent.request.operation,
                LifecycleOperation::Create { .. } | LifecycleOperation::Destroy {}
            )
        {
            return true;
        }
        position += 1;
    }
    false
}

fn first_session_entry(session: &Session) -> Option<&Entry> {
    if session.entries.is_empty() {
        None
    } else {
        Some(&session.entries[0])
    }
}

fn terminal_encoded_within_limit<E: TerminalEncoding>(
    value: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
) -> bool {
    let encoded = E::encode(value);
    match &encoded {
        Ok(bytes) => bytes.len() <= 8192,
        Err(_) => false,
    }
}

impl Inbox {
    fn lanes_invalid<T: TerminalEncoding>(&self, context: &AuthenticatedCenterContext) -> bool {
        let mut index = 0;
        while index < self.lanes.len() {
            let lane = &self.lanes[index];
            if usize::from(lane.slot) != index + 1 || !lane.valid_using::<T>(context) {
                return true;
            }
            index += 1;
        }
        false
    }

    fn prior_owner_conflict(&self, index: usize, owner: &Owner) -> bool {
        let previous = &self.owners[..index];
        let mut position = 0;
        while position < previous.len() {
            let old = &previous[position];
            if old.external == owner.external || old.local == owner.local {
                return true;
            }
            position += 1;
        }
        false
    }

    fn has_local_owner(
        &self,
        session: &Session,
        local_owner: sandbox_controller_core::TenantId,
    ) -> bool {
        let mut position = 0;
        while position < self.owners.len() {
            let owner = &self.owners[position];
            if owner.external == session.owner && owner.local == local_owner {
                return true;
            }
            position += 1;
        }
        false
    }
}

fn has_destroy_at_maximum(session: &Session, maximum: u64) -> bool {
    let mut position = 0;
    while position < session.entries.len() {
        let entry = &session.entries[position];
        if matches!(
            entry.intent.request.operation,
            LifecycleOperation::Destroy {}
        ) && entry.intent.accepted_version == maximum
        {
            return true;
        }
        position += 1;
    }
    false
}

impl Inbox {
    fn validate_entry_order_using<U: super::unload_resolution::UnloadEncoding>(
        &self,
        session: &Session,
        context: &AuthenticatedCenterContext,
    ) -> Result<(u64, [u16; 5]), JournalError> {
        let mut maximum = 0;
        let mut kinds = [0_u16; 5];
        for entry in &session.entries {
            let intent = &entry.intent;
            if intent.accepted_version <= maximum
                || !matches!(intent.request.operation, LifecycleOperation::Destroy {})
                    && !super::renewal_confirmation::follows_retained(session, maximum, intent)
            {
                return Err(JournalError::CorruptStore);
            }
            maximum = if maximum > intent.accepted_version {
                maximum
            } else {
                intent.accepted_version
            };
            let kind = self.entry_kind_using::<U>(session, entry, context)?;
            kinds[kind] += 1;
            if (kind == 0 || kind == 2) && kinds[kind] > 1 {
                return Err(JournalError::CorruptStore);
            }
        }
        Ok((maximum, kinds))
    }
}
