//! High-level identity attachment to the ONE existing local admission algorithm.
use super::super::{identity, transaction::WorldTransaction};
use super::{
    codec,
    model::{InternalLocalBinding, TenantBinding},
};
use crate::{
    AdmissionError, AdmissionLimits, ControllerJournal, CrashInjector, CreateAdmission,
    CreateOptions, JournalError,
    templates::{ComputeLimits, TemplateBinding},
};
use rusqlite::params;
use sandbox_controller_core as core;
use sandproof_protocol::{
    ResourceId,
    internal::lifecycle::{Intent, LifecycleFact, LifecycleRequest, LifecycleResponse},
};

/// Protected host cache registration, never deserialized from an HTTP request.
/// The descriptor digest is checked against the registered center binding;
/// artifact verification remains the original host/template responsibility.
#[derive(Clone, Debug)]
pub struct InternalTemplate {
    pub revision: ResourceId,
    pub digest: [u8; 32],
    /// Exact GLOBAL immutable registration. Its canonical digest is the wire
    /// binding; global paths are metadata, never local execution targets.
    pub registered: TemplateBinding,
    /// Before admission, owner is the registered GLOBAL numeric user identity.
    /// Admission authenticates that exact decimal ID, then projects it into
    /// the host's private tenant mapping for the original local algorithm.
    pub local: TemplateBinding,
}

/// Trusted local policy and explicitly sampled host clock. No production defaults.
#[derive(Clone, Debug)]
pub struct InternalAdmission {
    pub now_ms: i64,
    pub limits: AdmissionLimits,
    pub compute: ComputeLimits,
    pub template: InternalTemplate,
    pub file_policy_digest: core::PolicyDigest,
}

/// None means replay/fence, not permission to allocate or execute another VM.
/// Some identifies an already committed original local create. Execution must
/// retain the original application lock and recheck the original local workflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternalCreateResult {
    pub response: LifecycleResponse,
    pub local: Option<InternalLocalBinding>,
}

pub(in crate::controller) struct AdmissionAttachment {
    pub request: LifecycleRequest,
    pub local: InternalLocalBinding,
    /// Original protected cache descriptor before GLOBAL-to-LOCAL owner mapping.
    pub registered_template: TemplateBinding,
}

fn policy(options: &CreateOptions) -> Result<core::NetworkPolicy, AdmissionError> {
    let network = options.networking.as_ref();
    crate::application::selected_policy(&crate::application::CreateCommand {
        template_id: String::new(),
        timeout: None,
        allow_internet_access: network.and_then(|v| v.allow_internet_access),
        network: network.and_then(|v| v.network.clone()),
        metadata: None,
        env_vars: None,
    })
    .map_err(|_| AdmissionError::InvalidCreateOptions)
}

fn validate_template_and_time(
    request: &LifecycleRequest,
    admission: &InternalAdmission,
) -> Result<(), AdmissionError> {
    let InternalAdmission {
        now_ms,
        limits,
        template,
        ..
    } = admission;
    let now = u64::try_from(*now_ms).map_err(|_| AdmissionError::InvalidClock)?;
    let binding = &request.create;
    let shape = template.local.resources;
    let mut projected = template.registered.clone();
    projected
        .kernel
        .path
        .clone_from(&template.local.kernel.path);
    projected
        .rootfs
        .path
        .clone_from(&template.local.rootfs.path);
    if !template.local.is_valid()
        || projected != template.local
        || template.registered.canonical_digest().ok() != Some(template.digest)
        || template.revision != binding.template_revision
        || template.digest != binding.template_digest
        || binding.resources.instances != 1
        || binding.resources.cpu != u64::from(shape.cpu_count)
        || binding.resources.memory_bytes != shape.host_memory_bytes
        || binding.resources.disk_bytes != shape.disk_bytes
        || !binding.timeout_ms.is_multiple_of(1000)
        || !binding.max_lifetime_ms.is_multiple_of(1000)
        || binding.max_lifetime_ms / 1000 > limits.max_lifetime_secs
        || now >= binding.startup_deadline_ms
        || binding.startup_deadline_ms
            > now
                .checked_add(
                    limits
                        .startup_timeout_secs
                        .checked_mul(1000)
                        .ok_or(AdmissionError::TimeOverflow)?,
                )
                .ok_or(AdmissionError::TimeOverflow)?
    {
        return Err(AdmissionError::InvalidCreateOptions);
    }
    Ok(())
}

fn validate_owner(
    request: &LifecycleRequest,
    template: &InternalTemplate,
) -> Result<(), AdmissionError> {
    if template
        .local
        .owner
        .is_some_and(|expected| expected.to_string() != request.identity.tenant_id.as_str())
    {
        return Err(AdmissionError::Unauthorized);
    }
    Ok(())
}

fn checked_options(
    request: &LifecycleRequest,
) -> Result<(CreateOptions, core::NetworkPolicy), AdmissionError> {
    let options: CreateOptions = serde_json::from_slice(&request.create.options)
        .map_err(|_| AdmissionError::InvalidCreateOptions)?;
    if options.template.is_some()
        || options
            .metadata
            .keys()
            .any(|key| key.starts_with("_sandproof_"))
    {
        return Err(AdmissionError::InvalidCreateOptions);
    }
    let policy = policy(&options)?;
    Ok((options, policy))
}

impl InternalAdmission {
    /// The original pure binding checks, before a numeric physical-capacity
    /// veto may become definitive. No mapping, allocation or host I/O occurs.
    pub(crate) fn validate_attachment(
        &self,
        request: &LifecycleRequest,
    ) -> Result<(), AdmissionError> {
        validate_template_and_time(request, self)?;
        validate_owner(request, &self.template)?;
        checked_options(request)?;
        Ok(())
    }
}

fn local_owner(
    journal: &ControllerJournal,
    inbox: &super::model::Inbox,
    tenant: &sandproof_protocol::TenantId,
) -> Result<core::TenantId, JournalError> {
    if let Some(binding) = inbox.tenants.iter().find(|v| &v.tenant == tenant) {
        return Ok(binding.local);
    }
    let highest = journal
        .world()?
        .sandbox_records()
        .iter()
        .map(|(_, r)| r.owner.0)
        .max()
        .unwrap_or(0)
        .max(inbox.tenant_highwater);
    Ok(core::TenantId(
        highest
            .checked_add(1)
            .ok_or(JournalError::IdentifierExhausted)?,
    ))
}

impl ControllerJournal {
    /// Attach a previously authenticated, durably recorded create to the
    /// original local quota/lifetime/slot/outbox transaction. No worker I/O.
    /// # Errors
    /// Fails closed on conflicts, invalid cache/time/shape, quota or unknown commit.
    pub fn admit_internal_create(
        &mut self,
        request: &LifecycleRequest,
        admission: InternalAdmission,
        crashes: &mut impl CrashInjector,
    ) -> Result<InternalCreateResult, AdmissionError> {
        self.world()?;
        let inbox = codec::load(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        let entry = inbox
            .entries
            .iter()
            .find(|entry| entry.request == *request)
            .ok_or(JournalError::Conflict)?;
        if request.intent != Intent::Create {
            return Err(JournalError::Conflict.into());
        }
        let superseded = inbox.entries.iter().any(|entry| {
            entry.request.identity.sandbox_id == request.identity.sandbox_id
                && entry.request.identity.intent_version.get()
                    > request.identity.intent_version.get()
        });
        // Replay/fence checks precede clock/cache checks: a retry never resolves
        // a new alias, re-times the admission, or allocates a replacement.
        if superseded || entry.local.is_some() || entry.response.fact != LifecycleFact::Recorded {
            return Ok(InternalCreateResult {
                response: entry.response.clone(),
                local: None,
            });
        }
        validate_template_and_time(request, &admission)?;
        let InternalAdmission {
            now_ms,
            limits,
            compute,
            mut template,
            file_policy_digest,
        } = admission;
        let binding = &request.create;
        let shape = template.local.resources;
        validate_owner(request, &template)?;
        let owner = local_owner(self, &inbox, &request.identity.tenant_id)?;
        let registered_template = template.registered;
        // Do not confuse global identity with allocation order on this host.
        // None stays explicitly shared; private descriptions retain the SAME
        // authenticated owner through the original local admission checks.
        if template.local.owner.is_some() {
            template.local.owner = Some(owner.0);
        }
        let local = InternalLocalBinding {
            sandbox: self.next_sandbox_id()?,
            owner,
            operation: core::OperationId(1),
        };
        let (mut options, policy) = checked_options(request)?;
        options.template = Some(template.local);
        let response = entry.response.clone();
        self.admit_with_compute(
            core::CreateRequest {
                actor: core::Actor::Tenant(owner),
                owner,
                sandbox: local.sandbox,
                workspace: core::FsId(local.sandbox.0),
                file_policy: core::FilePolicyRef {
                    digest: file_policy_digest,
                },
                policy,
                operation: local.operation,
            },
            CreateAdmission {
                now_ms,
                timeout_secs: binding.timeout_ms / 1000,
                disk_bytes: shape.disk_bytes,
                limits: AdmissionLimits {
                    max_lifetime_secs: binding.max_lifetime_ms / 1000,
                    ..limits
                },
                options,
            },
            Some(compute),
            None,
            Some(&AdmissionAttachment {
                request: request.clone(),
                local,
                registered_template,
            }),
            crashes,
        )?;
        Ok(InternalCreateResult {
            response,
            local: Some(local),
        })
    }
}

/// Only the original admission calls this, after all native rows have been
/// prepared and before its single commit. Rollback removes BOTH responsibilities.
pub(in crate::controller) fn attach_admission(
    tx: &WorldTransaction<'_>,
    namespace: &str,
    attachment: &AdmissionAttachment,
) -> Result<(), JournalError> {
    let mut inbox = codec::load(tx)?.ok_or(JournalError::UnsupportedStore)?;
    let entry = inbox
        .entries
        .iter_mut()
        .find(|e| e.request == attachment.request)
        .ok_or(JournalError::Conflict)?;
    if entry.local.is_some() || attachment.request.intent != Intent::Create {
        return Err(JournalError::Conflict);
    }
    entry.local = Some(attachment.local);
    let tenant = attachment.request.identity.tenant_id.clone();
    if let Some(previous) = inbox.tenants.iter().find(|v| v.tenant == tenant) {
        if previous.local != attachment.local.owner {
            return Err(JournalError::Conflict);
        }
    } else {
        if attachment.local.owner.0 <= inbox.tenant_highwater {
            return Err(JournalError::Conflict);
        }
        inbox.tenant_highwater = attachment.local.owner.0;
        inbox.tenants.push(TenantBinding {
            tenant,
            local: attachment.local.owner,
        });
    }
    tx.execute(
        "UPDATE sandbox_lifetimes SET startup_deadline_ms=?1 WHERE sandbox_id=?2",
        params![
            i64::try_from(attachment.request.create.startup_deadline_ms)
                .map_err(|_| JournalError::UnsupportedInput)?,
            identity::sandbox_id(namespace, attachment.local.sandbox)?.as_str()
        ],
    )?;
    codec::save(tx, &inbox)
}

/// Persist the higher-level destroy memory and the native terminal intent in
/// ONE transaction. Native recovery cannot continue startup after accepting it.
pub(super) fn attach_terminal(
    tx: &WorldTransaction<'_>,
    namespace: &str,
    inbox: &super::model::Inbox,
    request: &LifecycleRequest,
) -> Result<(), JournalError> {
    if request.intent != Intent::Destroy {
        return Ok(());
    }
    let local = inbox
        .entries
        .iter()
        .find(|e| {
            e.request.identity.sandbox_id == request.identity.sandbox_id
                && e.request.intent == Intent::Create
        })
        .and_then(|e| e.local);
    let Some(local) = local else {
        return Ok(());
    };
    let world = tx.world();
    let record = world
        .sandbox_record(local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if record.owner != local.owner {
        return Err(JournalError::CorruptStore);
    }
    // Expiry/compensation or a previously accepted destroy already owns
    // termination. Reuse it; no second kill operation or stop is admitted.
    if record.terminal || record.phase == core::Phase::Destroyed {
        return Ok(());
    }
    let operation = super::super::allocators::read(tx, "operation", &local.sandbox.0.to_string())?
        .checked_add(1)
        .map(core::OperationId)
        .ok_or(JournalError::IdentifierExhausted)?;
    super::super::storage::apply(
        tx,
        namespace,
        core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
            actor: core::Actor::Tenant(local.owner),
            sandbox: local.sandbox,
            operation,
        })),
    )?;
    Ok(())
}
