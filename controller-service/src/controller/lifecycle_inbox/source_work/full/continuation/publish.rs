use super::{
    ControllerJournal, HostEvidence, JournalError, OriginalContinueWorkRequest, WorkAction,
    WorkOutcome, WorkReceipt, action_record, codec, core, work,
};
use crate::controller::{lifecycle_inbox, storage, transaction::WorldTransaction};
use crate::{CrashInjector, CrashPoint};
use lifecycle_inbox::unload_resolution::{UnloadResolution, ready_facts};
use sandproof_protocol::lifecycle::{AuthenticatedCenterContext, CurrentReadyFacts};

impl ControllerJournal {
    /// Publish one fixed `OriginalContinued` result only from a fresh trusted
    /// observation of the exact already-acknowledged original activation.
    /// # Errors
    /// Changed identity, source, lifetime, terminal state or native barrier refuses.
    pub fn publish_lifecycle_original_continued<E: HostEvidence>(
        &mut self,
        request: &OriginalContinueWorkRequest,
        action: &WorkAction<core::EffectId>,
        receipt: &WorkReceipt<E>,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<CurrentReadyFacts, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let sources = codec::load(&tx)?.ok_or(JournalError::CorruptStore)?;
        let record = action_record(&sources, &world, &self.namespace, action)?
            .ok_or(JournalError::Conflict)?;
        let current = super::super::end::prepare::current_identity(
            &tx,
            &self.namespace,
            &record.work,
            context,
            now_ms,
            &world,
        )?;
        if current != request.identity() || work(record, &world)?.as_ref() != Some(request) {
            return Err(JournalError::Conflict);
        }
        let mut inbox = lifecycle_inbox::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let (s, e) = inbox
            .find(record.work.request())?
            .ok_or(JournalError::Conflict)?;
        if let Some(UnloadResolution::AfterCapture { facts }) =
            &inbox.sessions[s].entries[e].unload_resolution
        {
            return if world
                .sandbox_record(current.key.sandbox)
                .is_some_and(|row| !row.terminal && row.phase == core::Phase::Running)
            {
                Ok(facts.clone())
            } else {
                Err(JournalError::Conflict)
            };
        }
        if inbox.sessions[s].entries[e].unload_resolution.is_some() {
            return Err(JournalError::Conflict);
        }
        receipt.validate_for(action)?;
        let WorkOutcome::ObservedSuccess(evidence) = &receipt.outcome else {
            return Err(JournalError::InvalidHostEvidence);
        };
        if !evidence.validates(action.command.command())
            || evidence.source_process_identity(action.command.command(), &action.identity.resource)
                != Some(request.source().source.clone())
            || !core::effect::effect_succeeded_check(&world, action.command.command())
        {
            return Err(JournalError::InvalidHostEvidence);
        }
        let lifetime = crate::controller::admission::read_lifetime(
            &tx,
            &self.namespace,
            current.key.sandbox,
            record.work.anchor.local.owner,
        )
        .map_err(|_| JournalError::CorruptStore)?
        .ok_or(JournalError::CorruptStore)?;
        let facts = ready_facts(lifetime, now_ms)?;
        storage::apply(
            &tx,
            &self.namespace,
            core::ControllerInput::Capture(core::capture::CaptureInput::PublishOriginalContinued {
                identity: current,
            }),
        )?;
        inbox.sessions[s].entries[e].unload_resolution = Some(UnloadResolution::AfterCapture {
            facts: facts.clone(),
        });
        lifecycle_inbox::codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(facts)
    }
}
