//! V3 identity/order on the original controller connection and writer lease.
//! The original reducer/admission/outbox remains the local phase authority.
mod access;
pub(in crate::controller) mod audit;
mod codec;
mod renewal;
mod renewal_confirmation;
mod validation;
mod window;
pub use audit::{LifecycleAudit, LifecycleAuditOperation};
mod model;
mod pending;
mod readiness;
mod refusal;
pub(in crate::controller) mod source_work;
#[cfg(test)]
mod tests;
mod unload_resolution;

use super::{ControllerJournal, identity, internal_inbox, transaction::WorldTransaction};
use crate::{
    AdmissionError, CrashInjector, CrashPoint, CreateAdmission, InternalAdmission,
    InternalLocalBinding, JournalError,
};
pub(super) use codec::{charged_bytes, validate};
use model::{Inbox, Owner};
pub use model::{LifecycleInboxConfig, LifecycleRetentionLane, LifecycleWorkState};
pub use readiness::{
    CreateReadiness, CreateReadinessPending, LifecycleCreateObservation, LifecycleReadyBasis,
    LifecycleReadyObservation, NativeCreateSnapshot,
};
use rusqlite::params;
use sandbox_controller_core as core;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation,
};
use sha2::{Digest, Sha256};
pub use source_work::{MaterialRetirementQuestion, SourceWork, SourceWorkRecord};

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_WORK_STATE_READ: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) struct CreateAttachment {
    pub intent: AcceptedLifecycleIntent,
    pub local: InternalLocalBinding,
    pub registered_template: crate::templates::TemplateBinding,
}

pub(in crate::controller) fn pinned(
    connection: &rusqlite::Connection,
    sandbox: core::SandboxId,
) -> Result<bool, JournalError> {
    Ok(codec::load(connection)?.is_some_and(|inbox| {
        inbox
            .sessions
            .iter()
            .any(|session| session.local.is_some_and(|local| local.sandbox == sandbox))
    }))
}

fn authenticated(
    inbox: &Inbox,
    intent: &AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
) -> Result<(), JournalError> {
    inbox.verify_context(context)?;
    intent.validate(context).map_err(|_| JournalError::Conflict)
}

impl ControllerJournal {
    #[cfg(test)]
    pub(crate) fn fail_next_lifecycle_commit(durable: bool) {
        super::transaction::fail_next_commit(durable);
    }

    #[cfg(test)]
    pub(crate) fn fail_next_lifecycle_work_state_read() {
        FAIL_NEXT_WORK_STATE_READ.set(true);
    }

    pub(crate) fn startup_epoch_ready(&self) -> Result<bool, JournalError> {
        self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let ready = codec::load(&tx)?.is_none_or(|inbox| {
            self.active_host_epoch.is_some()
                && self.active_host_epoch.as_ref() == inbox.host_epoch_observation.as_ref()
        });
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(ready)
    }

    /// Saved protected configuration; no provisioning or permission is inferred.
    /// # Errors
    /// Changed writers and missing/corrupt feature tables fail closed.
    pub fn lifecycle_inbox_configuration(
        &self,
    ) -> Result<Option<LifecycleInboxConfig>, JournalError> {
        self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let config = codec::load(&tx)?.map(|i| i.config);
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(config)
    }

    /// Original protected requirement; reading this does not validate hardware.
    /// # Errors
    /// Missing/foreign configuration cannot become a fabricated registration.
    pub fn host_epoch_requirement(
        &self,
    ) -> Result<crate::capture::HostEpochRequirement, JournalError> {
        let config = self
            .lifecycle_inbox_configuration()?
            .ok_or(JournalError::UnsupportedStore)?;
        Ok(crate::capture::HostEpochRequirement::configured(
            config.node,
            config.host_epoch.ok_or(JournalError::UnsupportedStore)?,
        ))
    }

    /// Bind the actual protected bootstrap observation BEFORE native admission.
    /// The provider runs without a SQL transaction; its physical truth is an
    /// explicit dependency, while identity and commit order are checked here.
    /// # Errors
    /// Missing/incorrect observations or uncertain commit grant no new work.
    pub fn establish_host_epoch(
        &mut self,
        provider: &mut impl FnMut(
            &crate::capture::HostEpochRequirement,
        ) -> Result<
            crate::capture::HostEpochObservation,
            crate::WorkerTransportError,
        >,
        crashes: &mut impl CrashInjector,
    ) -> Result<crate::capture::HostEpochObservation, JournalError> {
        self.active_host_epoch = None;
        let requirement = self.host_epoch_requirement()?;
        let observation = provider(&requirement).map_err(|_| JournalError::UnresolvedAction)?;
        if observation.node != *requirement.node()
            || observation.registration != *requirement.registration()
        {
            return Err(JournalError::InvalidHostEvidence);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if inbox.config.node != observation.node
            || inbox.config.host_epoch.as_ref() != Some(&observation.registration)
        {
            return Err(JournalError::Conflict);
        }
        inbox.host_epoch_observation = Some(observation.clone());
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        self.active_host_epoch = Some(observation.clone());
        Ok(observation)
    }

    /// Explicit protected provisioning of the current complete empty store.
    /// Inbox and source limits commit together; old formats are not migrated.
    /// # Errors
    /// Refuses changed identity, live objects, unsupported profile or reserve.
    pub fn initialize_lifecycle_inbox(
        &mut self,
        config: LifecycleInboxConfig,
        source: crate::capture::SourceObservationConfig,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        if !config.valid() || !source.valid() || !self.world()?.sandbox_records().is_empty() {
            return Err(JournalError::UnsupportedStore);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        if codec::load(&tx)?.is_some()
            || super::internal_inbox::configuration_snapshot(&tx)?.is_some()
        {
            return Err(JournalError::UnsupportedStore);
        }
        let cap = super::capacity::config(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let maintenance = super::checkpoint::config(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        if u64::from(config.max_operations_per_session) > maintenance.max_operations_per_sandbox {
            return Err(JournalError::UnsupportedStore);
        }
        if config.required_bytes() > cap.max_sql_value_bytes {
            return Err(JournalError::MaintenanceCapacity);
        }
        let reserve = config.required_bytes();
        let lanes = (1..=config.max_sessions)
            .map(model::LifecycleRetentionLane::fresh)
            .collect();
        let inbox = Inbox {
            config,
            host_epoch_observation: None,
            owner_high_water: 0,
            owners: vec![],
            sessions: vec![],
            lanes,
        };
        inbox.validate()?;
        let bytes = serde_json::to_vec(&inbox)?;
        tx.execute_batch("CREATE TABLE controller_lifecycle_inbox(id INTEGER PRIMARY KEY CHECK(id=1),state_json BLOB NOT NULL,sha256 BLOB NOT NULL CHECK(length(sha256)=32),reserve_bytes INTEGER NOT NULL CHECK(reserve_bytes>0));")?;
        tx.execute(
            "INSERT INTO controller_lifecycle_inbox VALUES(1,?1,?2,?3)",
            params![
                bytes,
                Sha256::digest(&bytes).as_slice(),
                i64::try_from(reserve).map_err(|_| JournalError::MaintenanceCapacity)?
            ],
        )?;
        source_work::initialize(&tx, source, &inbox.config)?;
        tx.execute(
            "UPDATE controller_meta SET version=?1 WHERE id=1",
            [crate::LIFECYCLE_STORE_FORMAT_VERSION],
        )?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)
    }

    /// TLS/registered sender facts are independently supplied by the original
    /// protected receiver. Recorded is not native admission or execution.
    /// # Errors
    /// Refuses changed replay content, stale order, foreign owner or capacity.
    pub fn record_lifecycle_intent(
        &mut self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        crashes: &mut impl CrashInjector,
    ) -> Result<LifecycleWorkState, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, intent, context)?;
        if let Some((s, e)) = inbox.find(intent)? {
            return Ok(inbox.state(s, e));
        }
        let (s, e) = inbox.record(intent)?;
        if matches!(intent.request.operation, LifecycleOperation::Destroy {})
            && let Some(local) = inbox.sessions[s].local
        {
            internal_inbox::terminate_local(&tx, &self.namespace, local)?;
            let operation = tx
                .world()
                .sandbox_record(local.sandbox)
                .ok_or(JournalError::CorruptStore)?
                .current_operation
                .ok_or(JournalError::CorruptStore)?
                .id;
            inbox.sessions[s].entries[e].local_operation = Some(operation);
        }
        codec::save(&tx, &inbox)?;
        let state = inbox.state(s, e);
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(state)
    }

    /// Exact original canonical request and current order. Returned local IDs
    /// are data; the dispatcher must still claim its original durable action.
    /// # Errors
    /// No missing or conflicting key is manufactured as a fresh request.
    pub fn lifecycle_work_state(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<LifecycleWorkState, JournalError> {
        #[cfg(test)]
        if FAIL_NEXT_WORK_STATE_READ.replace(false) {
            return Err(JournalError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                None,
            )));
        }
        self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, intent, context)?;
        let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        let state = inbox.state(s, e);
        tx.commit()?;
        Ok(state)
    }

    /// Reuse the original native quota/template/lifetime/slot admission. Full
    /// canonical mapping attaches inside that ONE transaction before commit.
    /// # Errors
    /// Unknown commits require querying this same key, never a new native ID.
    pub fn admit_lifecycle_create(
        &mut self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        admission: InternalAdmission,
        crashes: &mut impl CrashInjector,
    ) -> Result<LifecycleWorkState, AdmissionError> {
        let state = self.lifecycle_work_state(intent, context)?;
        let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
            return Err(JournalError::UnsupportedInput.into());
        };
        if !state.current || state.terminal || state.refusal.is_some() || state.local.is_some() {
            return Ok(state);
        }
        internal_inbox::validate_binding_and_time(create, &admission)?;
        internal_inbox::validate_owner_id(&intent.request.binding.key.owner, &admission.template)?;
        let (mut options, policy) = internal_inbox::checked_binding_options(create)?;
        let inbox = codec::load(&self.journal.connection)?.ok_or(JournalError::UnsupportedStore)?;
        if inbox.host_epoch_observation.is_none()
            || self.active_host_epoch.as_ref() != inbox.host_epoch_observation.as_ref()
        {
            return Err(JournalError::UnsupportedStore.into());
        }
        let owner = owner_for(&inbox, &self.world()?, &intent.request.binding.key.owner)?;
        let InternalAdmission {
            now_ms,
            limits,
            compute,
            mut template,
            file_policy_digest,
        } = admission;
        let registered_template = template.registered;
        if template.local.owner.is_some() {
            template.local.owner = Some(owner.0);
        }
        let shape = template.local.resources;
        options.template = Some(template.local);
        let local = InternalLocalBinding {
            sandbox: self.next_sandbox_id()?,
            owner,
            operation: core::OperationId(1),
        };
        let admitted = self.admit_with_compute(
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
                timeout_secs: create.timeout_ms / 1000,
                disk_bytes: shape.disk_bytes,
                limits: crate::AdmissionLimits {
                    max_lifetime_secs: create.max_lifetime_ms / 1000,
                    ..limits
                },
                options,
            },
            Some(compute),
            None,
            Some(&internal_inbox::AdmissionAttachment::Lifecycle(
                CreateAttachment {
                    intent: intent.clone(),
                    local,
                    registered_template,
                },
            )),
            crashes,
        );
        match admitted {
            // This is a definite original admission failure before effects.
            // The failed original transaction has rolled back. Persist the
            // refusal only after rechecking the same current full Create in
            // the original writer transaction. Unknown commits never enter it.
            Err(AdmissionError::QuotaExceeded) => {
                return self
                    .refuse_lifecycle_create_capacity(intent, context, crashes)
                    .map_err(AdmissionError::from);
            }
            other => {
                other?;
            }
        }
        self.lifecycle_work_state(intent, context)
            .map_err(AdmissionError::from)
    }
}

fn owner_for(
    inbox: &Inbox,
    world: &core::World,
    owner: &sandproof_protocol::TenantId,
) -> Result<core::TenantId, JournalError> {
    if let Some(row) = inbox.owners.iter().find(|r| &r.external == owner) {
        return Ok(row.local);
    }
    let highest = world
        .sandbox_records()
        .iter()
        .map(|(_, s)| s.owner.0)
        .max()
        .unwrap_or(0)
        .max(inbox.owner_high_water);
    Ok(core::TenantId(
        highest
            .checked_add(1)
            .ok_or(JournalError::IdentifierExhausted)?,
    ))
}

pub(super) fn attach_create(
    tx: &WorldTransaction<'_>,
    namespace: &str,
    a: &CreateAttachment,
) -> Result<(), JournalError> {
    let mut inbox = codec::load(tx)?.ok_or(JournalError::UnsupportedStore)?;
    let (s, e) = inbox.find(&a.intent)?.ok_or(JournalError::Conflict)?;
    if inbox.sessions[s].terminal
        || inbox.sessions[s].refusal.is_some()
        || inbox.sessions[s].current_version != a.intent.accepted_version
        || inbox.sessions[s].local.is_some()
    {
        return Err(JournalError::Conflict);
    }
    let LifecycleOperation::Create { create, .. } = &a.intent.request.operation else {
        return Err(JournalError::Conflict);
    };
    let owner = &inbox.sessions[s].owner;
    let expected = if let Some(row) = inbox.owners.iter().find(|r| &r.external == owner) {
        row.local
    } else {
        let maximum = tx
            .world()
            .sandbox_records()
            .iter()
            .filter(|(id, _)| *id != a.local.sandbox)
            .map(|(_, r)| r.owner.0)
            .max()
            .unwrap_or(0)
            .max(inbox.owner_high_water);
        core::TenantId(
            maximum
                .checked_add(1)
                .ok_or(JournalError::IdentifierExhausted)?,
        )
    };
    if expected != a.local.owner {
        return Err(JournalError::Conflict);
    }
    if !inbox
        .owners
        .iter()
        .any(|r| r.external == inbox.sessions[s].owner)
    {
        inbox.owner_high_water = a.local.owner.0;
        inbox.owners.push(Owner {
            external: inbox.sessions[s].owner.clone(),
            local: a.local.owner,
        });
    }
    inbox.sessions[s].local = Some(a.local);
    inbox.sessions[s].entries[e].local_operation = Some(a.local.operation);
    tx.execute(
        "UPDATE sandbox_lifetimes SET startup_deadline_ms=?1 WHERE sandbox_id=?2",
        params![
            i64::try_from(create.startup_deadline_ms)
                .map_err(|_| JournalError::UnsupportedInput)?,
            identity::sandbox_id(namespace, a.local.sandbox)?.as_str()
        ],
    )?;
    codec::save(tx, &inbox)
}

pub(super) fn check_admission(
    tx: &rusqlite::Connection,
    a: Option<&internal_inbox::AdmissionAttachment>,
) -> Result<(), JournalError> {
    if let Some(inbox) = codec::load(tx)? {
        let Some(internal_inbox::AdmissionAttachment::Lifecycle(attachment)) = a else {
            return Err(JournalError::UnsupportedInput);
        };
        let (s, e) = inbox
            .find(&attachment.intent)?
            .ok_or(JournalError::Conflict)?;
        let state = inbox.state(s, e);
        if !state.current || state.terminal || state.refusal.is_some() || state.local.is_some() {
            return Err(JournalError::Conflict);
        }
    }
    Ok(())
}

pub(super) fn configured(connection: &rusqlite::Connection) -> Result<bool, JournalError> {
    Ok(codec::load(connection)?.is_some())
}

pub(super) fn dispatch_epoch_allows(
    connection: &rusqlite::Connection,
    active: Option<&crate::capture::HostEpochObservation>,
    command: &core::Command,
) -> Result<bool, JournalError> {
    if !matches!(
        command,
        core::Command::InstallFence {
            terminal: false,
            ..
        } | core::Command::PrepareVm { .. }
            | core::Command::ConfigureNetwork { .. }
            | core::Command::ActivateVm { .. }
            | core::Command::ApplyFilePolicy { .. }
    ) {
        return Ok(true);
    }
    let Some(inbox) = codec::load(connection)? else {
        return Ok(true);
    };
    Ok(active.is_some() && active == inbox.host_epoch_observation.as_ref())
}
