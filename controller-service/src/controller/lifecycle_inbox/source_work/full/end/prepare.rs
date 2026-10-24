//! The sole admitted original-continue event shares the source/end SQL commit.
use super::super::super::SourceWork;
use super::{ControllerJournal, JournalError, codec, core, key, set_end};
use crate::controller::{admission, lifecycle_inbox, storage, transaction::WorldTransaction};
use crate::{
    CrashInjector, CrashPoint,
    capture::{CaptureApiEndObservation, OriginalContinueWorkRequest},
};
use core::capture::{CaptureInput, OriginalContinueIdentity};
use sandproof_protocol::lifecycle::AuthenticatedCenterContext;

pub(in crate::controller) fn permits_input(
    connection: &rusqlite::Connection,
    world: &core::World,
    input: &CaptureInput,
) -> Result<bool, JournalError> {
    let (identity, supplied) = match input {
        // The terminal SQL producer is not connected in this batch. Native
        // type support cannot bypass the saved original observation contract.
        CaptureInput::RetireTerminatedCapture(_) => return Ok(false),
        CaptureInput::PrepareOriginalContinue {
            identity,
            end_receipt,
        } => (*identity, Some(*end_receipt)),
        CaptureInput::PublishOriginalContinued { identity } => (*identity, None),
    };
    let Some(sources) = codec::load(connection)? else {
        return Ok(false);
    };
    let Some(record) = sources
        .records
        .iter()
        .find(|r| key(r) == Some(identity.key))
    else {
        return Ok(false);
    };
    super::validate_record(record)?;
    let saved = super::receipt(record)?;
    if saved.is_none()
        || supplied.is_some_and(|end| Some(end) != saved)
        || record.work.anchor.native.generation != identity.generation
        || record.work.anchor.native.current_vm != Some(identity.vm)
    {
        return Ok(false);
    }
    Ok(world.operation_plan(identity.key).is_some_and(|plan| {
        plan.epoch == identity.epoch
            && plan.generation == identity.generation
            && (supplied.is_some()
                || plan
                    .capture
                    .and_then(core::capture::CaptureHandoff::original_continuation)
                    .is_some_and(|c| c.vm == identity.vm && Some(c.end_receipt) == saved))
    }))
}

pub(in crate::controller) fn current_identity(
    connection: &rusqlite::Connection,
    namespace: &str,
    source: &SourceWork,
    context: &AuthenticatedCenterContext,
    now_ms: i64,
    world: &core::World,
) -> Result<OriginalContinueIdentity, JournalError> {
    let inbox = lifecycle_inbox::codec::load(connection)?.ok_or(JournalError::UnsupportedStore)?;
    lifecycle_inbox::authenticated(&inbox, source.request(), context)?;
    let (s, e) = inbox
        .find(source.request())?
        .ok_or(JournalError::Conflict)?;
    let state = inbox.state(s, e);
    if !state.current || state.terminal {
        return Err(JournalError::Conflict);
    }
    let operation = inbox.sessions[s].entries[e]
        .local_operation
        .ok_or(JournalError::UnresolvedAction)?;
    let local = source.anchor.local;
    if state
        .local
        .is_none_or(|binding| binding.owner != local.owner || binding.sandbox != local.sandbox)
        || world
            .sandbox_record(local.sandbox)
            .is_none_or(|row| row.owner != local.owner)
    {
        return Err(JournalError::Conflict);
    }
    let lifetime = admission::read_lifetime(connection, namespace, local.sandbox, local.owner)
        .map_err(|_| JournalError::CorruptStore)?
        .ok_or(JournalError::CorruptStore)?;
    if now_ms < 0
        || lifetime
            .started_at_ms
            .is_none_or(|start| now_ms < start || start > lifetime.startup_deadline_ms)
        || now_ms >= lifetime.deadline_ms
        || now_ms >= lifetime.max_deadline_ms
        || Some(lifetime) != source.anchor.native.lifetime
    {
        return Err(JournalError::Conflict);
    }
    let key = core::OperationKey {
        sandbox: local.sandbox,
        operation,
    };
    let plan = world
        .operation_plan(key)
        .ok_or(JournalError::UnresolvedAction)?;
    Ok(OriginalContinueIdentity {
        key,
        epoch: plan.epoch,
        generation: source.anchor.native.generation,
        vm: source
            .anchor
            .native
            .current_vm
            .ok_or(JournalError::CorruptStore)?,
    })
}

impl ControllerJournal {
    /// Close the original Full launch and prepare the same VM's continuation
    /// in the original source/native transaction. `None` means no Full worker
    /// was ever granted its SQL launch permit; an authorized worker requires
    /// its complete API-end and actual exit observation. No execute permit is
    /// returned, and all material obligations remain held.
    /// # Errors
    /// Wrong current source/end, expiry, terminal state or native uncertainty
    /// refuses atomically. An unknown commit returns no continuation question.
    pub fn prepare_lifecycle_original_continue(
        &mut self,
        source: &SourceWork,
        context: &AuthenticatedCenterContext,
        api_end: Option<CaptureApiEndObservation>,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<OriginalContinueWorkRequest, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let identity = current_identity(&tx, &self.namespace, source, context, now_ms, &world)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let index = sources
            .records
            .iter()
            .position(|r| r.work == *source)
            .ok_or(JournalError::Conflict)?;
        let record = &mut sources.records[index];
        super::super::validate::validate_record(record, &world)?;
        if key(record) != Some(identity.key) {
            return Err(JournalError::Conflict);
        }
        let end_receipt = set_end(record, api_end)?;
        // Saving the canonical question first lets the original storage profile
        // compare this exact input to SQL. Both saves roll back on any rejection.
        super::super::save(&tx, &sources)?;
        let decision = storage::apply(
            &tx,
            &self.namespace,
            core::ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
                identity,
                end_receipt,
            }),
        )?;
        super::super::validate::validate_record(&sources.records[index], &decision.next)?;
        let row = decision
            .next
            .sandbox_record(identity.key.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        let work = OriginalContinueWorkRequest::from_ended_source(
            identity,
            sources.records[index]
                .full_capture()
                .ok_or(JournalError::CorruptStore)?
                .clone(),
            source.requirement().clone(),
            end_receipt,
            row.policy.clone(),
            row.file_policy,
        );
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(work)
    }
}
