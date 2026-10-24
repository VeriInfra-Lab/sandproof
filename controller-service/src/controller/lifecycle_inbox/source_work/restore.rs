//! Unique restore responsibility in the same original source-work row.
//! A recorded activation is irreversible even if its physical result is unknown.
use super::{SourceWorkRecord, codec, required_option};
use crate::{
    ControllerJournal, InternalLocalBinding, JournalError,
    capture::{CommittedCaptureExpectation, RestoreActivationPermit, RestoreWorkRequest},
};
use sandbox_controller_core as core;
use sandproof_protocol::{
    WorkAction,
    lifecycle::{AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RestoreClaim {
    pub(super) intent: AcceptedLifecycleIntent,
    pub(super) local: InternalLocalBinding,
    pub(super) generation: core::Generation,
    /// Registered before Load; retained through the same VMM's lifetime.
    pub(super) material_use: bool,
    /// Some is the durable `MayHaveExecuted` marker and exact original action.
    #[serde(deserialize_with = "required_option")]
    pub(super) activation: Option<WorkAction<core::EffectId>>,
}

fn work(record: &SourceWorkRecord) -> Result<RestoreWorkRequest, JournalError> {
    let claim = record.restore.as_ref().ok_or(JournalError::Conflict)?;
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    if !claim.material_use {
        return Err(JournalError::UnresolvedAction);
    }
    let committed = CommittedCaptureExpectation::after_commit(
        capture.full.clone(),
        capture.completed.ok_or(JournalError::CorruptStore)?,
        capture.action.clone().ok_or(JournalError::CorruptStore)?,
    );
    Ok(RestoreWorkRequest::from_claim(
        claim.intent.clone(),
        record.work.requirement.clone(),
        committed,
        claim.local.operation,
        claim.generation,
    ))
}

pub(super) fn validate_record(
    record: &SourceWorkRecord,
    world: &core::World,
) -> Result<(), JournalError> {
    let Some(claim) = &record.restore else {
        return Ok(());
    };
    let capture = record.capture.as_ref().ok_or(JournalError::CorruptStore)?;
    let LifecycleOperation::Resume {
        source_incarnation,
        capture: id,
        target_incarnation,
        manifest_digest,
    } = claim.intent.request.operation
    else {
        return Err(JournalError::CorruptStore);
    };
    let original = record.work.request();
    if !matches!(original.request.operation, LifecycleOperation::Unload { source_incarnation: old, capture: old_id } if old == source_incarnation && old_id == id)
        || capture.completed != Some(manifest_digest)
        || source_incarnation.checked_next().ok() != Some(target_incarnation)
        || claim.intent.request.binding.session != original.request.binding.session
        || claim.intent.request.binding.node != original.request.binding.node
        || claim.intent.request.binding.key.owner != original.request.binding.key.owner
        || claim.intent.center != original.center
        || claim.intent.retention != original.retention
        || claim.local.sandbox != record.work.anchor.local.sandbox
        || claim.local.owner != record.work.anchor.local.owner
        || record.work.anchor.native.generation.checked_add(1) != Some(claim.generation)
    {
        return Err(JournalError::CorruptStore);
    }
    let key = core::OperationKey {
        sandbox: claim.local.sandbox,
        operation: claim.local.operation,
    };
    let plan = world
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    let origin = world
        .operation_plan(core::OperationKey {
            sandbox: claim.local.sandbox,
            operation: capture.operation,
        })
        .ok_or(JournalError::CorruptStore)?;
    if plan.generation != claim.generation
        || plan.restore_from != origin.target_snapshot
        || plan.restore_from.is_none()
        || world
            .operation_record(key)
            .is_none_or(|row| row.operation.kind != core::OperationKind::Resume)
    {
        return Err(JournalError::CorruptStore);
    }
    if let Some(action) = &claim.activation
        && (!matches!(action.command.command(), core::Command::ActivateVm { sandbox, generation, vm, .. }
            if *sandbox == key.sandbox && *generation == claim.generation && plan.vm == Some(*vm))
            || world
                .logical_effect(&action.command.logical)
                .is_none_or(|effect| effect.origin != core::EffectOrigin::Operation(key))
            || !matches!(
                world.effect_status(&action.command),
                Some(
                    core::EffectStatus::Dispatched
                        | core::EffectStatus::OutcomeUnknown
                        | core::EffectStatus::Acknowledged(_)
                )
            ))
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

/// Runs inside the original ready-to-dispatched transaction. A commit failure
/// can leave the marker saved; it never clears it or grants a different action.
pub(in crate::controller) fn claim_activation(
    connection: &rusqlite::Connection,
    world: &core::World,
    action: &WorkAction<core::EffectId>,
) -> Result<bool, JournalError> {
    let Some(mut sources) = codec::load(connection)? else {
        return Ok(true);
    };
    let Some(index) = sources.records.iter().position(|record| {
        record.restore.as_ref().is_some_and(|claim| {
            world
                .logical_effect(&action.command.logical)
                .is_some_and(|effect| {
                    effect.origin
                        == core::EffectOrigin::Operation(core::OperationKey {
                            sandbox: claim.local.sandbox,
                            operation: claim.local.operation,
                        })
                })
        })
    }) else {
        return Ok(true);
    };
    let record = &mut sources.records[index];
    let claim = record.restore.as_mut().ok_or(JournalError::CorruptStore)?;
    if !claim.material_use {
        return Ok(false);
    }
    if !matches!(action.command.command(), core::Command::ActivateVm { .. }) {
        return Ok(true);
    }
    if let Some(original) = &claim.activation {
        return Ok(original == action);
    }
    claim.activation = Some(action.clone());
    codec::save(connection, &sources, sources.max_sessions)?;
    Ok(true)
}

fn admitted_source_generation(
    connection: &rusqlite::Connection,
    namespace: &str,
    world: &core::World,
    source: &SourceWorkRecord,
    local: InternalLocalBinding,
    manifest_digest: sandproof_protocol::lifecycle::ContentDigest,
    now_ms: i64,
) -> Result<core::Generation, JournalError> {
    let capture = source
        .capture
        .as_ref()
        .ok_or(JournalError::UnresolvedAction)?;
    let source_key = core::OperationKey {
        sandbox: local.sandbox,
        operation: capture.operation,
    };
    let source_plan = world
        .operation_plan(source_key)
        .ok_or(JournalError::CorruptStore)?;
    let row = world
        .sandbox_record(local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    if row.terminal
        || row.phase != core::Phase::Paused
        || row.current_vm.is_some()
        || row.net_slot.is_some()
        || capture.completed != Some(manifest_digest)
        || capture.material_bytes.is_none()
        || row.durable_snapshot != source_plan.target_snapshot
        || source_plan.target_snapshot.is_none()
        || !source_plan
            .capture
            .is_some_and(core::capture::CaptureHandoff::may_stop_original)
    {
        return Err(JournalError::UnresolvedAction);
    }
    let witnesses = crate::controller::checkpoint::release_witnesses(connection, world)?;
    if !witnesses.iter().any(|w| {
        w.context.binding.sandbox == local.sandbox
            && Some(w.context.binding.vm) == source.work.anchor.native.current_vm
            && w.context.binding.owner == local.owner
            && w.context.binding.generation == row.generation
    }) {
        return Err(JournalError::UnresolvedAction);
    }
    let lifetime = crate::controller::admission::read_lifetime(
        connection,
        namespace,
        local.sandbox,
        local.owner,
    )
    .map_err(|_| JournalError::CorruptStore)?
    .ok_or(JournalError::CorruptStore)?;
    if now_ms < 0
        || now_ms >= lifetime.deadline_ms
        || now_ms >= lifetime.max_deadline_ms
        || Some(lifetime) != source.work.anchor.native.lifetime
    {
        return Err(JournalError::Conflict);
    }
    row.generation
        .checked_add(1)
        .ok_or(JournalError::IdentifierExhausted)
}

fn check_runtime_capacity(
    connection: &rusqlite::Transaction<'_>,
    namespace: &str,
    world: &core::World,
    local: InternalLocalBinding,
    limits: crate::AdmissionLimits,
    compute: crate::templates::ComputeLimits,
) -> Result<(), JournalError> {
    if !compute.is_valid() {
        return Err(JournalError::UnsupportedInput);
    }
    let options =
        crate::controller::admission::read_create_options(connection, namespace, local.sandbox)
            .map_err(|_| JournalError::CorruptStore)?;
    let template = options.template.ok_or(JournalError::CorruptStore)?;
    if template.owner.is_some_and(|owner| owner != local.owner.0) {
        return Err(JournalError::CorruptStore);
    }
    let (global, tenant) =
        crate::controller::admission_usage::usage(connection, namespace, world, local.owner, true)
            .map_err(|error| match error {
                crate::AdmissionError::Journal(error) => error,
                _ => JournalError::CorruptStore,
            })?;
    let shape = template.resources;
    if global.instances >= limits.max_active_global
        || tenant.instances >= limits.max_active_per_tenant
        || global
            .cpu
            .checked_add(u64::from(shape.cpu_count))
            .is_none_or(|total| total > compute.max_vcpus_global)
        || tenant
            .cpu
            .checked_add(u64::from(shape.cpu_count))
            .is_none_or(|total| total > compute.max_vcpus_per_tenant)
        || global
            .memory_bytes
            .checked_add(shape.host_memory_bytes)
            .is_none_or(|total| total > compute.max_memory_bytes_global)
        || tenant
            .memory_bytes
            .checked_add(shape.host_memory_bytes)
            .is_none_or(|total| total > compute.max_memory_bytes_per_tenant)
    {
        return Err(JournalError::MaintenanceCapacity);
    }
    Ok(())
}

impl ControllerJournal {
    /// Acquire the source and install the original core Resume in one `SQLite`
    /// commit. No load can start from an uncommitted claim or another key.
    /// # Errors
    /// Expired, incomplete, changed or already claimed sources refuse admission.
    pub fn admit_lifecycle_resume(
        &mut self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
        limits: crate::AdmissionLimits,
        compute: crate::templates::ComputeLimits,
        crashes: &mut impl crate::CrashInjector,
    ) -> Result<crate::LifecycleWorkState, JournalError> {
        let state = self.lifecycle_work_state(intent, context)?;
        let local = state.local.ok_or(JournalError::UnresolvedAction)?;
        let tx = crate::controller::transaction::WorldTransaction::begin(
            &mut self.journal.connection,
            &self.committed_world,
        )?;
        let world = tx.world();
        let mut inbox = super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::super::authenticated(&inbox, intent, context)?;
        let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let LifecycleOperation::Resume {
            source_incarnation,
            capture: id,
            target_incarnation,
            manifest_digest,
        } = intent.request.operation
        else {
            return Err(JournalError::Conflict);
        };
        let index = sources
            .records
            .iter()
            .position(|record| {
                record.work.request().request.binding.session == intent.request.binding.session
                    && matches!(record.work.request().request.operation,
                    LifecycleOperation::Unload { source_incarnation: old, capture: old_id }
                    if old == source_incarnation && old_id == id)
            })
            .ok_or(JournalError::Conflict)?;
        if let Some(original) = &sources.records[index].restore {
            return if original.intent == *intent {
                Ok(inbox.state(s, e))
            } else {
                Err(JournalError::Conflict)
            };
        }
        if !state.current
            || state.terminal
            || inbox.sessions[s].incarnation != Some(source_incarnation)
            || source_incarnation.checked_next().ok() != Some(target_incarnation)
        {
            return Err(JournalError::Conflict);
        }
        let generation = admitted_source_generation(
            &tx,
            &self.namespace,
            &world,
            &sources.records[index],
            local,
            manifest_digest,
            now_ms,
        )?;
        check_runtime_capacity(&tx, &self.namespace, &world, local, limits, compute)?;
        // Existing claims replay above even after the allocator is exhausted.
        // A new identity is read and checked under this same write transaction.
        let operation =
            crate::controller::allocators::read(&tx, "operation", &local.sandbox.0.to_string())?
                .checked_add(1)
                .map(core::OperationId)
                .ok_or(JournalError::IdentifierExhausted)?;
        let target = InternalLocalBinding { operation, ..local };
        sources.records[index].restore = Some(RestoreClaim {
            intent: intent.clone(),
            local: target,
            generation,
            material_use: true,
            activation: None,
        });
        // The original core profile gate reads this exact saved claim inside
        // this transaction before accepting Resume. Neither write is visible
        // alone, including when SQLite's commit result is unknown.
        codec::save(&tx, &sources, sources.max_sessions)?;
        crate::controller::storage::apply(
            &tx,
            &self.namespace,
            core::ControllerInput::Request(core::Request::Resume(core::ResumeRequest {
                actor: core::Actor::Tenant(local.owner),
                sandbox: local.sandbox,
                operation,
            })),
        )?;
        inbox.sessions[s].entries[e].local_operation = Some(operation);
        inbox.sessions[s].local = Some(target);
        inbox.sessions[s].incarnation = Some(target_incarnation);
        super::super::codec::save(&tx, &inbox)?;
        let result = inbox.state(s, e);
        crashes.hit(crate::CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(crate::CrashPoint::AfterCommit)?;
        Ok(result)
    }

    /// Return only the original persisted source and successor binding.
    /// # Errors
    /// No unconfirmed source, stale key, missing pin or foreign context can mint work.
    pub fn lifecycle_restore_work(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<RestoreWorkRequest, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::super::authenticated(&inbox, intent, context)?;
        inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        // This is readonly responsibility, including after Destroy. It grants
        // neither a new load nor activation; those recheck current eligibility.
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = sources
            .records
            .iter()
            .find(|r| r.restore.as_ref().is_some_and(|c| c.intent == *intent))
            .ok_or(JournalError::UnresolvedAction)?;
        validate_record(record, &world)?;
        let request = work(record)?;
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(request)
    }

    /// Re-issue only the same permit for original unknown activation. The host
    /// may finish its same candidate's guest initialization, never load again.
    /// # Errors
    /// Terminal/superseded work, another candidate or an uncommitted marker refuse.
    pub fn lifecycle_restore_activation_permit(
        &self,
        request: &RestoreWorkRequest,
        action: &WorkAction<core::EffectId>,
        context: &AuthenticatedCenterContext,
    ) -> Result<RestoreActivationPermit, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::super::authenticated(&inbox, request.intent(), context)?;
        let (s, e) = inbox
            .find(request.intent())?
            .ok_or(JournalError::Conflict)?;
        let current = inbox.state(s, e);
        if !current.current || current.terminal {
            return Err(JournalError::Conflict);
        }
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = sources
            .records
            .iter()
            .find(|record| {
                record
                    .restore
                    .as_ref()
                    .is_some_and(|claim| claim.intent == *request.intent())
            })
            .ok_or(JournalError::UnresolvedAction)?;
        validate_record(record, &world)?;
        if work(record)? != *request {
            return Err(JournalError::Conflict);
        }
        let claim = record.restore.as_ref().ok_or(JournalError::CorruptStore)?;
        if claim.activation.as_ref() != Some(action)
            || !matches!(
                world.effect_status(&action.command),
                Some(core::EffectStatus::Dispatched | core::EffectStatus::OutcomeUnknown)
            )
        {
            return Err(JournalError::UnresolvedAction);
        }
        let permit = RestoreActivationPermit::after_dispatch_commit(
            action.clone(),
            request.binding_digest()?,
        );
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(permit)
    }

    /// Locate the exact original work for a dispatched restore action,
    /// including readonly reconciliation after its request was superseded.
    /// # Errors
    /// Changed action origin, corrupt claims or missing material uses refuse.
    pub fn lifecycle_restore_action_work(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Option<RestoreWorkRequest>, JournalError> {
        if !matches!(
            action.command.command(),
            core::Command::PrepareVm {
                source: Some(_),
                ..
            } | core::Command::ConfigureNetwork { .. }
                | core::Command::ApplyFilePolicy { .. }
                | core::Command::ActivateVm { .. }
        ) {
            return Ok(None);
        }
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let sources = codec::load(&tx)?;
        let record = sources.as_ref().and_then(|sources| {
            sources.records.iter().find(|r| {
                r.restore.as_ref().is_some_and(|claim| {
                    world
                        .logical_effect(&action.command.logical)
                        .is_some_and(|effect| {
                            effect.origin
                                == core::EffectOrigin::Operation(core::OperationKey {
                                    sandbox: claim.local.sandbox,
                                    operation: claim.local.operation,
                                })
                        })
                })
            })
        });
        let result = match record {
            Some(record) => {
                validate_record(record, &world)?;
                Some(work(record)?)
            }
            None => None,
        };
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }
}

/// The generic local request entry cannot manufacture a restore source. Its
/// exact durable claim must already be part of this same SQL transaction.
pub(in crate::controller) fn permits_resume_input(
    connection: &rusqlite::Connection,
    world: &core::World,
    request: &core::ResumeRequest,
) -> Result<bool, JournalError> {
    let Some(sources) = codec::load(connection)? else {
        return Ok(false);
    };
    let Some(row) = world.sandbox_record(request.sandbox) else {
        return Ok(false);
    };
    Ok(sources.records.iter().any(|record| {
        record.restore.as_ref().is_some_and(|claim| {
            claim.local.sandbox == request.sandbox
                && claim.local.operation == request.operation
                && request.actor == core::Actor::Tenant(claim.local.owner)
                && row.owner == claim.local.owner
                && !row.terminal
                && row.generation.checked_add(1) == Some(claim.generation)
                && row.phase == core::Phase::Paused
                && claim.material_use
                && claim.activation.is_none()
                && record.capture.as_ref().is_some_and(|capture| {
                    world
                        .operation_plan(core::OperationKey {
                            sandbox: request.sandbox,
                            operation: capture.operation,
                        })
                        .is_some_and(|plan| {
                            plan.target_snapshot.is_some()
                                && plan.target_snapshot == row.durable_snapshot
                        })
                })
        })
    }))
}
