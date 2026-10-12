//! Pure bounded inbox transitions and canonical parameter checks; no `SQLite` I/O.
use crate::{CreateOptions, JournalError};
use sandbox_controller_core as core;
use sandproof_protocol::internal::lifecycle::{
    Intent, LifecycleFact, LifecycleRequest, LifecycleResponse, RESULT_CONFIRMATION_BODY_BYTES,
};
use sandproof_protocol::internal::window::{RETENTION_LANE_BYTES, WindowCloseRequest};
use sandproof_protocol::internal::{NodeIdentity, PositiveCounter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub(super) const CONFIRMATION_FIELD_BYTES: usize =
    RESULT_CONFIRMATION_BODY_BYTES + b",\"confirmed\":".len();

/// Permanent local identities, allocated only with the original create admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InternalLocalBinding {
    pub sandbox: core::SandboxId,
    pub owner: core::TenantId,
    pub operation: core::OperationId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TenantBinding {
    pub tenant: sandproof_protocol::TenantId,
    pub local: core::TenantId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InternalInboxConfig {
    pub node: NodeIdentity,
    pub max_sandboxes: u16,
    pub operations_per_sandbox: u16,
    pub max_options_bytes: u32,
}
impl InternalInboxConfig {
    /// Pure deployment validation for the original bounded inbox; it neither
    /// opens storage nor grants a lifecycle capability.
    #[must_use]
    pub fn valid(&self) -> bool {
        (1..=4096).contains(&self.max_sandboxes)
            && (2..=64).contains(&self.operations_per_sandbox)
            && (1..=65_536).contains(&self.max_options_bytes)
            && self.required_bytes() <= 16 * 1024 * 1024
    }
    /// Conservative reserved row envelope, including every retained request
    /// and response. Protected entrypoints use this SAME capacity formula.
    #[must_use]
    pub fn required_bytes(&self) -> usize {
        usize::from(self.max_sandboxes)
            .saturating_mul(usize::from(self.operations_per_sandbox))
            .saturating_mul(
                (self.max_options_bytes as usize)
                    .saturating_mul(4)
                    .saturating_add(8192)
                    // Every future entry may retain its EXACT center confirmation.
                    .saturating_add(CONFIRMATION_FIELD_BYTES),
            )
            .saturating_add(8192)
            // The rejection frontier is future storage, not guessed old slack.
            .saturating_add(usize::from(self.max_sandboxes).saturating_mul(RETENTION_LANE_BYTES))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub(super) request: LifecycleRequest,
    pub(super) digest: [u8; 32],
    pub(super) response: LifecycleResponse,
    pub(super) confirmed: Option<LifecycleResponse>,
    pub(super) local: Option<InternalLocalBinding>,
}
impl Entry {
    fn valid_confirmation(&self) -> bool {
        self.confirmed.as_ref().is_none_or(|confirmed| {
            confirmed == &self.response
                && !matches!(
                    confirmed.fact,
                    LifecycleFact::Recorded | LifecycleFact::Stopped
                )
                && serde_json::to_vec(confirmed)
                    .is_ok_and(|bytes| bytes.len() <= RESULT_CONFIRMATION_BODY_BYTES)
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inbox {
    pub(super) config: InternalInboxConfig,
    pub(super) observation: PositiveCounter,
    pub(super) entries: Vec<Entry>,
    pub(super) tenant_highwater: u64,
    pub(super) tenants: Vec<TenantBinding>,
    /// Fixed protected positions. A retired incarnation never becomes absent.
    pub(super) lanes: Vec<InternalRetentionLane>,
}

/// Original durable per-position rejection memory. It contains no guest secret
/// or create payload; a stamp is an exact confirmed fact, not permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InternalRetentionLane {
    pub retention_slot: u16,
    pub retired_through: u64,
    pub current_window: Option<PositiveCounter>,
    pub last_retired: Option<LifecycleResponse>,
}
impl InternalRetentionLane {
    pub(super) fn fresh(retention_slot: u16) -> Self {
        Self {
            retention_slot,
            retired_through: 0,
            current_window: Some(1.try_into().expect("positive literal")),
            last_retired: None,
        }
    }
    fn valid(&self, node: &NodeIdentity) -> bool {
        let successor = self
            .retired_through
            .checked_add(1)
            .and_then(|v| v.try_into().ok());
        if self.current_window != successor {
            return false;
        }
        match (&self.last_retired, self.retired_through) {
            (None, 0) => true,
            (Some(last), floor) if floor > 0 => {
                last.identity.retention_slot == self.retention_slot
                    && last.identity.request_window.get() == floor
                    && WindowCloseRequest::new(last.clone()).validate(node).is_ok()
            }
            _ => false,
        }
    }
}

fn encoded_request(
    request: &LifecycleRequest,
    config: &InternalInboxConfig,
    current: Option<PositiveCounter>,
) -> Result<Vec<u8>, JournalError> {
    request
        .validate(&config.node)
        .map_err(|_| JournalError::Conflict)?;
    if Some(request.identity.request_window) != current
        || request.create.options.len() > config.max_options_bytes as usize
    {
        return Err(JournalError::Conflict);
    }
    // SAME production normalizer, not a second JSON/option semantics implementation.
    let options: CreateOptions =
        serde_json::from_slice(&request.create.options).map_err(|_| JournalError::Conflict)?;
    if options.encoded().map_err(|_| JournalError::Conflict)? != request.create.options
        // Local resolved paths MUST NOT come from a remote center. The later host
        // template adapter resolves the immutable revision from protected cache.
        || options.template.is_some()
    {
        return Err(JournalError::Conflict);
    }
    Ok(serde_json::to_vec(request)?)
}
impl Inbox {
    pub(super) fn lane(&self, slot: u16) -> Option<&InternalRetentionLane> {
        self.lanes.iter().find(|lane| lane.retention_slot == slot)
    }
    pub(super) fn current_window(&self, slot: u16) -> Option<PositiveCounter> {
        self.lane(slot)?.current_window
    }
    fn validate_lanes(&self) -> Result<(), JournalError> {
        let mut slots = BTreeSet::new();
        if self.lanes.len() != usize::from(self.config.max_sandboxes)
            || self.lanes.iter().any(|lane| {
                lane.retention_slot == 0
                    || lane.retention_slot > self.config.max_sandboxes
                    || !slots.insert(lane.retention_slot)
                    || !lane.valid(&self.config.node)
            })
        {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }
    fn validate_tenants(&self) -> Result<(), JournalError> {
        let mut tenants = BTreeSet::new();
        let mut local_tenants = BTreeSet::new();
        if self.tenants.len() > usize::from(self.config.max_sandboxes)
            || self.tenant_highwater
                < self
                    .tenants
                    .iter()
                    .map(|binding| binding.local.0)
                    .max()
                    .unwrap_or(0)
            || self.tenants.iter().any(|binding| {
                binding.local.0 == 0
                    || binding.local.0 > self.tenant_highwater
                    || !tenants.insert(binding.tenant.as_str())
                    || !local_tenants.insert(binding.local.0)
                    || !self.entries.iter().any(|entry| {
                        entry
                            .local
                            .is_some_and(|local| local.owner == binding.local)
                            && entry.request.identity.tenant_id == binding.tenant
                    })
            })
        {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }
    fn validate_history(&self, index: usize, entry: &Entry) -> Result<(), JournalError> {
        let request = &entry.request;
        let prior = self.entries[..index]
            .iter()
            .rev()
            .find(|e| e.request.identity.sandbox_id == request.identity.sandbox_id);
        if let Some(prior) = prior {
            if prior.request.identity.tenant_id != request.identity.tenant_id
                || prior.request.identity.retention_slot != request.identity.retention_slot
                || prior.request.identity.request_window != request.identity.request_window
                || prior.request.create != request.create
                || prior.request.intent == Intent::Destroy
                || request.intent == Intent::Create
                || request.identity.intent_version.get()
                    <= prior.request.identity.intent_version.get()
                || entry.response.observation_version.get()
                    <= prior.response.observation_version.get()
            {
                return Err(JournalError::CorruptStore);
            }
        } else if self.entries[..index]
            .iter()
            .any(|other| other.request.identity.retention_slot == request.identity.retention_slot)
            || (request.intent != Intent::Destroy
                && (request.intent != Intent::Create || request.identity.intent_version.get() != 1))
        {
            return Err(JournalError::CorruptStore);
        }
        let count = self
            .entries
            .iter()
            .filter(|e| e.request.identity.sandbox_id == request.identity.sandbox_id)
            .count();
        if count > usize::from(self.config.operations_per_sandbox)
            || (count == usize::from(self.config.operations_per_sandbox)
                && !self.entries.iter().any(|e| {
                    e.request.identity.sandbox_id == request.identity.sandbox_id
                        && e.request.intent == Intent::Destroy
                }))
        {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }
    pub(super) fn validate(&self) -> Result<(), JournalError> {
        self.validate_lanes()?;
        self.validate_tenants()?;
        if self.observation.get() < self.entries.len() as u64 + 1
            || !self.config.valid()
            || self.entries.len()
                > usize::from(self.config.max_sandboxes)
                    * usize::from(self.config.operations_per_sandbox)
        {
            return Err(JournalError::CorruptStore);
        }
        let mut operations = BTreeSet::new();
        let mut sandboxes = BTreeSet::new();
        let mut observations = BTreeSet::new();
        let mut local_sandboxes = BTreeSet::new();
        for stamp in self
            .lanes
            .iter()
            .filter_map(|lane| lane.last_retired.as_ref())
        {
            if stamp.observation_version.get() > self.observation.get()
                || !observations.insert(stamp.observation_version.get())
                || !operations.insert(stamp.identity.operation_id.as_str())
                || !sandboxes.insert(stamp.identity.sandbox_id.as_str())
            {
                return Err(JournalError::CorruptStore);
            }
        }
        for (index, entry) in self.entries.iter().enumerate() {
            let request = &entry.request;
            let encoded = encoded_request(
                request,
                &self.config,
                self.current_window(request.identity.retention_slot),
            )
            .map_err(|_| JournalError::CorruptStore)?;
            if Sha256::digest(encoded).as_slice() != entry.digest
                || !operations.insert(request.identity.operation_id.as_str())
                || entry.response.validate_for(request).is_err()
                || entry.response.observation_version.get() > self.observation.get()
                || !observations.insert(entry.response.observation_version.get())
                || !entry.valid_confirmation()
            {
                return Err(JournalError::CorruptStore);
            }
            if let Some(local) = entry.local {
                if request.intent != Intent::Create
                    || local.operation != core::OperationId(1)
                    || local.sandbox.0 == 0
                    || !local_sandboxes.insert(local.sandbox.0)
                    || !self.tenants.iter().any(|binding| {
                        binding.tenant == request.identity.tenant_id && binding.local == local.owner
                    })
                    || !matches!(
                        entry.response.fact,
                        LifecycleFact::Recorded | LifecycleFact::Ready { .. }
                    )
                {
                    return Err(JournalError::CorruptStore);
                }
            } else if matches!(entry.response.fact, LifecycleFact::Ready { .. })
                || (matches!(entry.response.fact, LifecycleFact::Renewed { .. })
                    && !self.entries[..index].iter().any(|create| {
                        create.request.identity.sandbox_id == request.identity.sandbox_id
                            && create.local.is_some()
                    }))
            {
                return Err(JournalError::CorruptStore);
            }
            if self
                .lanes
                .iter()
                .filter_map(|lane| lane.last_retired.as_ref())
                .any(|stamp| stamp.identity.sandbox_id == request.identity.sandbox_id)
            {
                return Err(JournalError::CorruptStore);
            }
            sandboxes.insert(request.identity.sandbox_id.as_str());
            self.validate_history(index, entry)?;
        }
        // Recent stamps and live incarnations share the uniqueness set but not
        // the active-instance bound: each position can retain one of each.
        if self
            .entries
            .iter()
            .map(|entry| entry.request.identity.sandbox_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
            > usize::from(self.config.max_sandboxes)
        {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }
    pub(super) fn record(
        &mut self,
        request: &LifecycleRequest,
    ) -> Result<LifecycleResponse, JournalError> {
        let encoded = encoded_request(
            request,
            &self.config,
            self.current_window(request.identity.retention_slot),
        )?;
        if let Some(entry) = self
            .entries
            .iter()
            .find(|e| e.request.identity.operation_id == request.identity.operation_id)
        {
            if entry.request == *request && Sha256::digest(&encoded).as_slice() == entry.digest {
                return Ok(entry.response.clone());
            }
            return Err(JournalError::Conflict);
        }
        let prior = self
            .entries
            .iter()
            .rev()
            .find(|e| e.request.identity.sandbox_id == request.identity.sandbox_id);
        if let Some(prior) = prior
            && (prior.request.identity.tenant_id != request.identity.tenant_id
                || prior.request.identity.retention_slot != request.identity.retention_slot
                || prior.request.identity.request_window != request.identity.request_window
                || prior.request.create != request.create
                || (prior.request.intent == Intent::Destroy && request.intent != Intent::Destroy))
        {
            return Err(JournalError::Conflict);
        }
        let count = self
            .entries
            .iter()
            .filter(|e| e.request.identity.sandbox_id == request.identity.sandbox_id)
            .count();
        if count >= usize::from(self.config.operations_per_sandbox)
            || (request.intent != Intent::Destroy
                && count + 1 >= usize::from(self.config.operations_per_sandbox))
        {
            return Err(JournalError::MaintenanceCapacity);
        }
        if let Some(prior) = prior {
            if request.intent == Intent::Create
                || request.identity.intent_version.get()
                    <= prior.request.identity.intent_version.get()
                || prior.request.intent == Intent::Destroy
            {
                return Err(JournalError::Conflict);
            }
        } else {
            if self.entries.iter().any(|other| {
                other.request.identity.retention_slot == request.identity.retention_slot
            }) {
                return Err(JournalError::Conflict);
            }
            if self
                .entries
                .iter()
                .map(|e| e.request.identity.sandbox_id.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                >= usize::from(self.config.max_sandboxes)
            {
                return Err(JournalError::MaintenanceCapacity);
            }
            if request.intent != Intent::Destroy
                && (request.intent != Intent::Create || request.identity.intent_version.get() != 1)
            {
                return Err(JournalError::Conflict);
            }
        }
        self.observation = self
            .observation
            .checked_next()
            .map_err(|_| JournalError::IdentifierExhausted)?;
        let response = LifecycleResponse {
            identity: request.identity.clone(),
            observation_version: self.observation,
            fact: LifecycleFact::Recorded,
        };
        self.entries.push(Entry {
            request: request.clone(),
            digest: Sha256::digest(encoded).into(),
            response: response.clone(),
            confirmed: None,
            local: None,
        });
        self.validate()?;
        Ok(response)
    }
}
