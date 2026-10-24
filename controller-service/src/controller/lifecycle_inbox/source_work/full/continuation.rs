//! Readonly original work and post-dispatch permission; neither reloads a VM.
use super::{JournalError, SourceWorkRecord, WorkAction, codec, core, key};
use crate::controller::identity;
use crate::{
    ControllerJournal, HostEvidence,
    capture::{OriginalContinuePermit, OriginalContinueWorkRequest},
};
use rusqlite::OptionalExtension;
use sandproof_protocol::{WorkOutcome, WorkReceipt, lifecycle::AuthenticatedCenterContext};
use serde::de::DeserializeOwned;
mod publish;
pub(in crate::controller) type ContinuedEvidence = (
    OriginalContinueWorkRequest,
    WorkAction<core::EffectId>,
    Vec<u8>,
);

pub(in crate::controller) fn historical_identity(
    world: &core::World,
    local: crate::InternalLocalBinding,
    native: &crate::NativeCreateSnapshot,
) -> Option<core::capture::OriginalContinueIdentity> {
    let operation = native.current_operation?;
    if native.owner != local.owner
        || native.phase != core::Phase::Running
        || native.terminal
        || !native.uncertain_actions.is_empty()
        || operation.kind != core::OperationKind::Pause
        || !matches!(operation.status, core::OperationStatus::Failed(_))
        || operation.epoch != native.epoch
    {
        return None;
    }
    let identity = core::capture::OriginalContinueIdentity {
        key: core::OperationKey {
            sandbox: local.sandbox,
            operation: operation.id,
        },
        epoch: native.epoch,
        generation: native.generation,
        vm: native.current_vm?,
    };
    let plan = world.operation_plan(identity.key)?;
    let origin = world.operation_plan(core::OperationKey {
        sandbox: local.sandbox,
        operation: local.operation,
    })?;
    let saved = plan.capture?.original_continuation()?;
    (plan.epoch == identity.epoch
        && plan.generation == identity.generation
        && saved.vm == identity.vm
        && origin.generation == identity.generation
        && origin.vm == Some(identity.vm)
        && world
            .operation_record(identity.key)
            .is_some_and(|r| r.operation == operation)
        && core::effect::effect_succeeded_check(world, &identity.activation_command()))
    .then_some(identity)
}

pub(in crate::controller) fn ready_records<E: HostEvidence + DeserializeOwned>(
    connection: &rusqlite::Connection,
    world: &core::World,
    namespace: &str,
    local: crate::InternalLocalBinding,
    native: &crate::NativeCreateSnapshot,
) -> Result<Option<ContinuedEvidence>, JournalError> {
    let Some(identity) = historical_identity(world, local, native) else {
        return Ok(None);
    };
    let sources = codec::load(connection)?.ok_or(JournalError::CorruptStore)?;
    let record = sources
        .records
        .iter()
        .find(|r| key(r) == Some(identity.key))
        .ok_or(JournalError::CorruptStore)?;
    if record.work.anchor.local != local {
        return Err(JournalError::CorruptStore);
    }
    let question = work(record, world)?.ok_or(JournalError::CorruptStore)?;
    let mut statement=connection.prepare("SELECT action_json,receipt_json FROM outbox WHERE operation_id=?1 AND dispatch_state='receipted' ORDER BY id")?;
    let mut found = None;
    for row in statement.query_map(
        [identity::operation_id(namespace, identity.key)?.as_str()],
        |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)),
    )? {
        let (action, bytes) = row?;
        let action: WorkAction<core::EffectId> = serde_json::from_slice(&action)?;
        if *action.command.command() != identity.activation_command() {
            continue;
        }
        if world
            .logical_effect(&action.command.logical)
            .is_none_or(|l| l.current_attempt != action.command.attempt)
        {
            continue;
        }
        action_record(&sources, world, namespace, &action)?.ok_or(JournalError::CorruptStore)?;
        let receipt: WorkReceipt<E> = serde_json::from_slice(&bytes)?;
        receipt.validate_for(&action)?;
        let WorkOutcome::ObservedSuccess(ref evidence) = receipt.outcome else {
            return Err(JournalError::CorruptStore);
        };
        if !evidence.validates(action.command.command())
            || evidence.source_process_identity(action.command.command(), &action.identity.resource)
                != Some(question.source().source.clone())
            || found.is_some()
        {
            return Err(JournalError::CorruptStore);
        }
        found = Some((question.clone(), action, bytes));
    }
    found.map(Some).ok_or(JournalError::UnresolvedAction)
}

pub(in crate::controller) fn work(
    record: &SourceWorkRecord,
    world: &core::World,
) -> Result<Option<OriginalContinueWorkRequest>, JournalError> {
    let Some(key) = key(record) else {
        return Ok(None);
    };
    let plan = world
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    let Some(saved) = plan
        .capture
        .and_then(core::capture::CaptureHandoff::original_continuation)
    else {
        return Ok(None);
    };
    super::validate_record(record, world)?;
    let end = record
        .capture_work_end_receipt()?
        .ok_or(JournalError::CorruptStore)?;
    if saved.end_receipt != end || Some(saved.vm) != record.work.anchor.native.current_vm {
        return Err(JournalError::CorruptStore);
    }
    let row = world
        .sandbox_record(key.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    Ok(Some(OriginalContinueWorkRequest::from_ended_source(
        core::capture::OriginalContinueIdentity {
            key,
            epoch: plan.epoch,
            generation: plan.generation,
            vm: saved.vm,
        },
        record
            .full_capture()
            .ok_or(JournalError::CorruptStore)?
            .clone(),
        record.work.requirement().clone(),
        end,
        row.policy.clone(),
        row.file_policy,
    )))
}

fn action_record<'a>(
    sources: &'a codec::Sources,
    world: &core::World,
    namespace: &str,
    action: &WorkAction<core::EffectId>,
) -> Result<Option<&'a SourceWorkRecord>, JournalError> {
    if !matches!(action.command.command(), core::Command::ActivateVm { .. }) {
        return Ok(None);
    }
    let Some(logical) = world.logical_effect(&action.command.logical) else {
        return Ok(None);
    };
    let core::EffectOrigin::Operation(origin) = logical.origin else {
        return Ok(None);
    };
    let Some(plan) = world.operation_plan(origin) else {
        return Err(JournalError::CorruptStore);
    };
    if !core::capture::continuation::activation_belongs_to_continuation(
        plan,
        action.command.command(),
    ) {
        return Ok(None);
    }
    let record = sources
        .records
        .iter()
        .find(|r| key(r) == Some(origin))
        .ok_or(JournalError::CorruptStore)?;
    let question = work(record, world)?.ok_or(JournalError::CorruptStore)?;
    if logical.current_attempt != action.command.attempt
        || action.identity.owner != identity::owner_id(record.work.anchor.local.owner)?
        || action.identity.operation != identity::operation_id(namespace, origin)?
        || action.identity.sandbox != identity::sandbox_id(namespace, origin.sandbox)?
        || action.identity.generation != question.identity().generation
        || action.identity.epoch != question.identity().epoch
        || action.identity.resource != question.source().resource
        || action.identity.attempt != action.command.attempt
    {
        return Err(JournalError::Conflict);
    }
    Ok(Some(record))
}

fn stored_action(
    connection: &rusqlite::Connection,
    action: &WorkAction<core::EffectId>,
) -> Result<bool, JournalError> {
    let bytes:Option<Vec<u8>>=connection.query_row(
        "SELECT action_json FROM outbox WHERE sandbox_id=?1 AND logical_action_id=?2 AND attempt=?3 AND dispatch_state!='ready'",
        rusqlite::params![action.identity.sandbox.as_str(),action.identity.logical_action.as_str(),i64::try_from(action.identity.attempt).map_err(|_|JournalError::Conflict)?],|r|r.get(0),
    ).optional()?;
    Ok(bytes.as_deref() == Some(serde_json::to_vec(action)?.as_slice()))
}

pub(in crate::controller) fn validate_receipt<E: HostEvidence>(
    connection: &rusqlite::Connection,
    world: &core::World,
    action: &WorkAction<core::EffectId>,
    receipt: &WorkReceipt<E>,
) -> Result<(), JournalError> {
    let Some(sources) = codec::load(connection)? else {
        return Ok(());
    };
    let Some(record) = action_record(
        &sources,
        world,
        &crate::controller::checkpoint::namespace(connection)?,
        action,
    )?
    else {
        return Ok(());
    };
    if let WorkOutcome::ObservedSuccess(evidence) = &receipt.outcome {
        let question = work(record, world)?.ok_or(JournalError::CorruptStore)?;
        if !stored_action(connection, action)?
            || !evidence.validates(action.command.command())
            || evidence.source_process_identity(action.command.command(), &action.identity.resource)
                != Some(question.source().source.clone())
            || !core::capture::continuation::continuation_activation_pending(
                world,
                action.command.command(),
            )
        {
            return Err(JournalError::InvalidHostEvidence);
        }
    }
    Ok(())
}

impl ControllerJournal {
    pub(crate) fn lifecycle_original_continue_activation(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<Option<(WorkAction<core::EffectId>, OriginalContinueWorkRequest)>, JournalError>
    {
        let world = self.world()?;
        if world.sandbox_record(sandbox).is_none_or(|row| {
            row.terminal
                || row.phase != core::Phase::Pausing
                || row
                    .current_operation
                    .is_none_or(|operation| operation.kind != core::OperationKind::Pause)
        }) {
            return Ok(None);
        }
        let Some(record) = self.lifecycle_capture_record(sandbox)? else {
            return Ok(None);
        };
        let Some(question) = work(&record, &world)? else {
            return Ok(None);
        };
        if !core::capture::continuation::continuation_activation_pending(
            &world,
            &question.identity().activation_command(),
        ) {
            return Ok(None);
        }
        let mut statement=self.journal.connection.prepare("SELECT action_json FROM outbox WHERE operation_id=?1 AND dispatch_state='receipted' ORDER BY id")?;
        for bytes in statement.query_map(
            [identity::operation_id(&self.namespace, question.identity().key)?.as_str()],
            |r| r.get::<_, Vec<u8>>(0),
        )? {
            let action: WorkAction<core::EffectId> = serde_json::from_slice(&bytes?)?;
            if *action.command.command() == question.identity().activation_command()
                && core::effect::effect_succeeded_check(&world, action.command.command())
            {
                self.lifecycle_original_continue_action_work(&action)?
                    .ok_or(JournalError::CorruptStore)?;
                return Ok(Some((action, question)));
            }
        }
        Ok(None)
    }
    /// Locate retained original work, including after a later terminal intent.
    /// No authority to PATCH or reopen access is returned by this read.
    /// # Errors
    /// Missing original responsibility or substituted action refuses the read.
    pub fn lifecycle_original_continue_action_work(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Option<OriginalContinueWorkRequest>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let result = if let Some(sources) = codec::load(&tx)? {
            action_record(&sources, &world, &self.namespace, action)?
                .map(|r| work(r, &world)?.ok_or(JournalError::CorruptStore))
                .transpose()?
        } else {
            None
        };
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    /// Reissue only the same original dispatch permission. The host observes
    /// this exact VM before PATCH and never loads a snapshot or initializes it.
    /// # Errors
    /// Supersession, expiry, missing end, changed work or uncommitted dispatch refuses.
    pub fn lifecycle_original_continue_permit(
        &self,
        request: &OriginalContinueWorkRequest,
        action: &WorkAction<core::EffectId>,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
    ) -> Result<OriginalContinuePermit, JournalError> {
        if !self.startup_epoch_ready()? {
            return Err(JournalError::UnresolvedAction);
        }
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = action_record(&sources, &world, &self.namespace, action)?
            .ok_or(JournalError::Conflict)?;
        let current = super::end::prepare::current_identity(
            &tx,
            &self.namespace,
            &record.work,
            context,
            now_ms,
            &world,
        )?;
        if current != request.identity()
            || work(record, &world)?.as_ref() != Some(request)
            || !stored_action(&tx, action)?
            || !core::capture::continuation::continuation_activation_pending(
                &world,
                action.command.command(),
            )
            || !matches!(
                world.effect_status(&action.command),
                Some(core::EffectStatus::Dispatched | core::EffectStatus::OutcomeUnknown)
            )
        {
            return Err(JournalError::Conflict);
        }
        let permit = OriginalContinuePermit::after_dispatch_commit(
            action.clone(),
            request.binding_digest()?,
        );
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(permit)
    }
}
