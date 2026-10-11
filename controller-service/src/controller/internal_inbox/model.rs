//! Pure bounded inbox transitions and canonical parameter checks; no `SQLite` I/O.
use crate::{CreateOptions, JournalError};
use sandproof_protocol::internal::lifecycle::{
    Intent, LifecycleFact, LifecycleRequest, LifecycleResponse,
};
use sandproof_protocol::internal::{NodeIdentity, PositiveCounter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InternalInboxConfig {
    pub node: NodeIdentity,
    pub max_sandboxes: u16,
    pub operations_per_sandbox: u16,
    pub max_options_bytes: u32,
}
impl InternalInboxConfig {
    pub(super) fn valid(&self) -> bool {
        (1..=4096).contains(&self.max_sandboxes)
            && (2..=64).contains(&self.operations_per_sandbox)
            && (1..=65_536).contains(&self.max_options_bytes)
            && self.required_bytes() <= 16 * 1024 * 1024
    }
    pub(super) fn required_bytes(&self) -> usize {
        usize::from(self.max_sandboxes)
            .saturating_mul(usize::from(self.operations_per_sandbox))
            .saturating_mul(
                (self.max_options_bytes as usize)
                    .saturating_mul(4)
                    .saturating_add(8192),
            )
            .saturating_add(8192)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub(super) request: LifecycleRequest,
    pub(super) digest: [u8; 32],
    pub(super) response: LifecycleResponse,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inbox {
    pub(super) config: InternalInboxConfig,
    pub(super) observation: PositiveCounter,
    pub(super) entries: Vec<Entry>,
}

fn encoded_request(
    request: &LifecycleRequest,
    config: &InternalInboxConfig,
) -> Result<Vec<u8>, JournalError> {
    request
        .validate(&config.node)
        .map_err(|_| JournalError::Conflict)?;
    if request.identity.request_window.get() != 1
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
    pub(super) fn validate(&self) -> Result<(), JournalError> {
        if self.observation.get() != self.entries.len() as u64 + 1
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
        for (index, entry) in self.entries.iter().enumerate() {
            let request = &entry.request;
            let encoded =
                encoded_request(request, &self.config).map_err(|_| JournalError::CorruptStore)?;
            if Sha256::digest(encoded).as_slice() != entry.digest
                || !operations.insert(request.identity.operation_id.as_str())
                || entry.response.validate_for(request).is_err()
                || entry.response.fact != LifecycleFact::Recorded
                || entry.response.observation_version.get() > self.observation.get()
                || !observations.insert(entry.response.observation_version.get())
            {
                return Err(JournalError::CorruptStore);
            }
            sandboxes.insert(request.identity.sandbox_id.as_str());
            let prior = self.entries[..index]
                .iter()
                .rev()
                .find(|e| e.request.identity.sandbox_id == request.identity.sandbox_id);
            if let Some(prior) = prior {
                if prior.request.identity.tenant_id != request.identity.tenant_id
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
            } else if request.intent != Intent::Destroy
                && (request.intent != Intent::Create || request.identity.intent_version.get() != 1)
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
        }
        if sandboxes.len() > usize::from(self.config.max_sandboxes) {
            return Err(JournalError::CorruptStore);
        }
        Ok(())
    }
    pub(super) fn record(
        &mut self,
        request: &LifecycleRequest,
    ) -> Result<LifecycleResponse, JournalError> {
        let encoded = encoded_request(request, &self.config)?;
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
        });
        self.validate()?;
        Ok(response)
    }
}
