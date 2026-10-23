//! Canonical identities and accepted intent order. Local lifecycle phase stays
//! in the original World; these rows do not invent another phase authority.
use crate::{InternalLocalBinding, JournalError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ExpectedSession,
    IncarnationId, LifecycleOperation, SessionId,
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
    pub max_options_bytes: u32,
    pub max_profile_bytes: u32,
    pub max_manifest_bytes: u32,
}
impl LifecycleInboxConfig {
    #[must_use]
    pub fn required_bytes(&self) -> u64 {
        // Each finite position keeps Create/Unload/Destroy, an explicit native
        // mapping, one capture work and its complete immutable bundle receipt.
        // The final operation position is reserved for terminal responsibility.
        u64::from(self.max_sessions)
            * (4 * u64::from(self.max_options_bytes)
                + 3 * 8192
                + 2 * u64::from(self.max_manifest_bytes)
                + u64::from(self.max_profile_bytes)
                + 8192)
            + 8192
    }
    pub fn valid(&self) -> bool {
        self.center.deployment == self.node.deployment_id
            && self
                .host_epoch
                .as_ref()
                .is_none_or(crate::capture::HostEpochRegistration::valid)
            && (1..=4096).contains(&self.max_sessions)
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
    pub local: Option<InternalLocalBinding>,
    pub local_operation: Option<sandbox_controller_core::OperationId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub intent: AcceptedLifecycleIntent,
    pub local_operation: Option<sandbox_controller_core::OperationId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Session {
    pub id: SessionId,
    pub owner: TenantId,
    pub current_version: u64,
    pub terminal: bool,
    pub incarnation: Option<IncarnationId>,
    pub local: Option<InternalLocalBinding>,
    pub entries: Vec<Entry>,
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
            if session.entries.is_empty()
                || session.local.is_some() && self.host_epoch_observation.is_none()
                || session.entries.len() > 3
                || self.sessions[..i].iter().any(|old| {
                    old.id == session.id
                        || session.local.is_some_and(|local| {
                            old.local
                                .is_some_and(|prior| prior.sandbox == local.sandbox)
                        })
                })
            {
                return Err(JournalError::CorruptStore);
            }
            let mut maximum = 0;
            let mut kinds = [0_u8; 3];
            for entry in &session.entries {
                let intent = &entry.intent;
                maximum = maximum.max(intent.accepted_version);
                let kind = self.entry_kind(session, entry, &context)?;
                kinds[kind] += 1;
                if kinds[kind] > 1 {
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

    fn entry_kind(
        &self,
        session: &Session,
        entry: &Entry,
        context: &AuthenticatedCenterContext,
    ) -> Result<usize, JournalError> {
        let intent = &entry.intent;
        let binding = &intent.request.binding;
        if intent.validate(context).is_err()
            || binding.session != session.id
            || binding.key.owner != session.owner
            || session.local.is_none() && entry.local_operation.is_some()
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
                if session.incarnation != Some(*source_incarnation)
                    || session.local.is_none()
                    || intent.accepted_version != 2
                    || binding.expected != ExpectedSession::Version(1)
                {
                    return Err(JournalError::CorruptStore);
                }
                Ok(1)
            }
            LifecycleOperation::Destroy {} => Ok(2),
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
            let session = &mut self.sessions[s];
            if session.owner != request.binding.key.owner || session.terminal {
                return Err(JournalError::Conflict);
            }
            let terminal = matches!(request.operation, LifecycleOperation::Destroy {});
            if (!terminal
                && request.binding.expected != ExpectedSession::Version(session.current_version))
                || (terminal && intent.accepted_version <= session.current_version)
                || session.entries.len() >= 3
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
                _ => return Err(JournalError::UnsupportedInput),
            }
            session.current_version = intent.accepted_version;
            session.terminal = terminal;
            session.entries.push(Entry {
                intent: intent.clone(),
                local_operation: None,
            });
            return Ok((s, session.entries.len() - 1));
        }
        if self.sessions.len() >= usize::from(self.config.max_sessions) {
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
            owner: request.binding.key.owner.clone(),
            current_version: intent.accepted_version,
            terminal,
            incarnation,
            local: None,
            entries: vec![Entry {
                intent: intent.clone(),
                local_operation: None,
            }],
        });
        Ok((self.sessions.len() - 1, 0))
    }
}
