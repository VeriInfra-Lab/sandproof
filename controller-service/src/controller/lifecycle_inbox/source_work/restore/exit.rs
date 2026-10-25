//! Readonly authority for observing loss of the original possibly-run candidate.
use super::{codec, validate_record, work};
use crate::capture::{RestoreCandidateExitObservation, RestoreWorkRequest};
use crate::controller::lifecycle_inbox::{
    codec as inbox_codec, readiness::snapshot::base_snapshot,
};
use crate::{
    ControllerJournal, CreateReadiness, InternalLocalBinding, JournalError, NativeCreateSnapshot,
};
use sandbox_controller_core as core;
use sandproof_protocol::{
    WorkAction,
    lifecycle::{AcceptedLifecycleIntent, AuthenticatedCenterContext, ContentDigest},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RestoreExitCandidate {
    pub(crate) work: RestoreWorkRequest,
    pub(crate) action: WorkAction<core::EffectId>,
    pub(crate) current_request: AcceptedLifecycleIntent,
    local: InternalLocalBinding,
    native: NativeCreateSnapshot,
    work_binding: ContentDigest,
}

impl RestoreExitCandidate {
    pub(crate) fn matches(&self, observed: &RestoreCandidateExitObservation) -> bool {
        observed.action == self.action
            && observed.work_binding == self.work_binding
            && observed.process.pid > 0
            && observed.process.start_time_ticks > 0
            && observed.process.boot_id == self.work.source().host_bootstrap.boot_id
    }
}

fn eligible(
    local: InternalLocalBinding,
    native: &NativeCreateSnapshot,
    request: &RestoreWorkRequest,
    action: &WorkAction<core::EffectId>,
    now_ms: i64,
) -> bool {
    let core::Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = action.command.command()
    else {
        return false;
    };
    let Some(lifetime) = native.lifetime else {
        return false;
    };
    *sandbox == local.sandbox
        && local.operation == request.operation()
        && native.owner == local.owner
        && !native.terminal
        && matches!(native.phase, core::Phase::Starting | core::Phase::Running)
        && native.current_vm == Some(*vm)
        && native.epoch == *epoch
        && native.generation == *generation
        && request.generation() == *generation
        && action.identity.epoch == *epoch
        && action.identity.generation == *generation
        && native.current_operation.is_some_and(|operation| {
            operation.id == local.operation
                && operation.kind == core::OperationKind::Resume
                && operation.epoch == *epoch
                && matches!(
                    operation.status,
                    core::OperationStatus::Pending
                        | core::OperationStatus::OutcomeUnknown
                        | core::OperationStatus::Succeeded
                )
        })
        && now_ms >= 0
        && lifetime
            .started_at_ms
            .is_some_and(|started| started <= now_ms && started <= lifetime.startup_deadline_ms)
        && now_ms < lifetime.deadline_ms
        && now_ms < lifetime.max_deadline_ms
}

impl ControllerJournal {
    /// No physical fact or new dispatch authority is created. In particular,
    /// an absent MayHaveExecuted marker never produces an exit question.
    pub(crate) fn lifecycle_restore_exit_candidate(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
    ) -> Result<Option<RestoreExitCandidate>, JournalError> {
        if !self.startup_epoch_ready()? {
            return Ok(None);
        }
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        inbox_codec::validate(&tx, &world)?;
        let view = base_snapshot(&tx, &self.namespace, &world, intent, context)?.view;
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = sources.records.iter().find(|record| {
            record
                .restore
                .as_ref()
                .is_some_and(|claim| claim.intent == *intent)
        });
        let result =
            if let (Some(record), Some(local), Some(native)) = (record, view.local, view.native) {
                validate_record(record, &world)?;
                let claim = record.restore.as_ref().ok_or(JournalError::CorruptStore)?;
                // A failed Unload continued under its own qualification is a
                // different observation path. Never borrow an old Resume fence.
                if view.readiness == CreateReadiness::NotCurrent
                    || !matches!(
                        view.current_request.request.operation,
                        sandproof_protocol::lifecycle::LifecycleOperation::Resume { .. }
                            | sandproof_protocol::lifecycle::LifecycleOperation::Renew { .. }
                    )
                    || claim.activation.is_none()
                {
                    None
                } else {
                    let request = work(record)?;
                    let action = claim
                        .activation
                        .as_ref()
                        .ok_or(JournalError::CorruptStore)?;
                    if claim.local == local && eligible(local, &native, &request, action, now_ms) {
                        Some(RestoreExitCandidate {
                            work_binding: request.binding_digest()?,
                            work: request,
                            action: action.clone(),
                            current_request: view.current_request,
                            local,
                            native,
                        })
                    } else {
                        None
                    }
                }
            } else {
                None
            };
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }
}
