//! Historical Unload failure completion on the original inbox entry.
//! A saved result never substitutes for a new physical Ready observation.
use super::{
    ControllerJournal, JournalError, NativeCreateSnapshot, codec, model::Entry, source_work,
};
use crate::controller::transaction::WorldTransaction;
use crate::{CrashInjector, CrashPoint};
use sandbox_controller_core as core;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CurrentReadyFacts, LifecycleOperation,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum UnloadResolution {
    BeforeAdmission {
        original: Box<NativeCreateSnapshot>,
        result: Option<CurrentReadyFacts>,
    },
    AfterCapture {
        facts: CurrentReadyFacts,
    },
}
impl UnloadResolution {
    pub(super) fn result(&self) -> Option<&CurrentReadyFacts> {
        match self {
            Self::BeforeAdmission { result, .. } => result.as_ref(),
            Self::AfterCapture { facts } => Some(facts),
        }
    }
    pub(super) fn before_admission(&self) -> bool {
        matches!(self, Self::BeforeAdmission { .. })
    }
}

pub(super) fn validate_entry(entry: &Entry) -> Result<(), JournalError> {
    let Some(resolution) = &entry.unload_resolution else {
        return Ok(());
    };
    if !matches!(
        entry.intent.request.operation,
        LifecycleOperation::Unload { .. }
    ) || serde_json::to_vec(resolution)?.len() > 4096
    {
        return Err(JournalError::CorruptStore);
    }
    if let UnloadResolution::BeforeAdmission { original, .. } = resolution {
        if entry.local_operation.is_some()
            || original.phase != core::Phase::Running
            || original.terminal
            || original.current_vm.is_none()
            || !original.uncertain_actions.is_empty()
            || original.lifetime.is_none()
        {
            return Err(JournalError::CorruptStore);
        }
    } else if entry.local_operation.is_none() {
        return Err(JournalError::CorruptStore);
    }
    if let Some(result) = resolution.result()
        && (result.started_at_ms > result.observed_at_ms
            || result.observed_at_ms >= result.end_at_ms
            || result.end_at_ms > result.max_end_at_ms
            || result.started_at_ms > result.startup_deadline_ms)
        {
            return Err(JournalError::CorruptStore);
        }
    Ok(())
}

pub(super) fn ready_facts(
    lifetime: crate::Lifetime,
    now_ms: i64,
) -> Result<CurrentReadyFacts, JournalError> {
    let convert = |v| u64::try_from(v).map_err(|_| JournalError::CorruptStore);
    if now_ms < 0
        || now_ms >= lifetime.deadline_ms
        || now_ms >= lifetime.max_deadline_ms
        || lifetime
            .started_at_ms
            .is_none_or(|start| start > now_ms || start > lifetime.startup_deadline_ms)
    {
        return Err(JournalError::Conflict);
    }
    Ok(CurrentReadyFacts {
        started_at_ms: convert(lifetime.started_at_ms.ok_or(JournalError::CorruptStore)?)?,
        end_at_ms: convert(lifetime.deadline_ms)?,
        max_end_at_ms: convert(lifetime.max_deadline_ms)?,
        startup_deadline_ms: convert(lifetime.startup_deadline_ms)?,
        observed_at_ms: convert(now_ms)?,
    })
}

impl ControllerJournal {
    pub(crate) fn lifecycle_unload_before_admission_origin(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<Option<AcceptedLifecycleIntent>, JournalError> {
        self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::authenticated(&inbox, intent, context)?;
        let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        let state = inbox.state(s, e);
        let result = if state.current
            && !state.terminal
            && inbox.sessions[s].entries[e]
                .unload_resolution
                .as_ref()
                .is_some_and(UnloadResolution::before_admission)
        {
            Some(
                inbox.sessions[s]
                    .current_execution_origin()
                    .ok_or(JournalError::CorruptStore)?
                    .intent
                    .clone(),
            )
        } else {
            None
        };
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }
    /// Permanently close an Unload which never admitted a native Pause or Full.
    /// This retires only readonly source metadata in the same original commit;
    /// no VM is stopped or continued, and no successful result is published.
    /// # Errors
    /// Supersession, expiry, any native/capture responsibility or changed original refuses.
    pub fn close_lifecycle_unload_before_admission(
        &mut self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<(), JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::authenticated(&inbox, intent, context)?;
        let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        let state = inbox.state(s, e);
        if !state.current
            || state.terminal
            || !matches!(intent.request.operation, LifecycleOperation::Unload { .. })
            || state.local_operation.is_some()
        {
            return Err(JournalError::Conflict);
        }
        let local = state.local.ok_or(JournalError::UnresolvedAction)?;
        let row = world
            .sandbox_record(local.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        let original =
            super::readiness::snapshot::native_snapshot(&tx, &self.namespace, &world, local, row)?;
        if !original.has_completed_original_execution(local)
            && source_work::full::continued_identity(&world, local, &original).is_none()
        {
            return Err(JournalError::UnresolvedAction);
        }
        ready_facts(original.lifetime.ok_or(JournalError::CorruptStore)?, now_ms)?;
        match &inbox.sessions[s].entries[e].unload_resolution {
            Some(UnloadResolution::BeforeAdmission {
                original: saved, ..
            }) if **saved == original => {}
            Some(_) => return Err(JournalError::Conflict),
            None => {
                inbox.sessions[s].entries[e].unload_resolution =
                    Some(UnloadResolution::BeforeAdmission {
                        original: Box::new(original.clone()),
                        result: None,
                    });
            }
        }
        source_work::retire_unadmitted(&tx, &world, intent, &original)?;
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(())
    }

    /// Return the fixed historical result for this exact Unload. Current Ready
    /// is independently reprobed through the execution-origin query.
    /// # Errors
    /// Foreign or unrecorded requests cannot observe another result.
    pub fn lifecycle_original_continued_result(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<Option<CurrentReadyFacts>, JournalError> {
        self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::authenticated(&inbox, intent, context)?;
        let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        let result = inbox.sessions[s].entries[e]
            .unload_resolution
            .as_ref()
            .and_then(UnloadResolution::result)
            .cloned();
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }

    pub(crate) fn publish_pre_admission_original_continued(
        &mut self,
        view: &crate::LifecycleCreateObservation,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<Option<CurrentReadyFacts>, JournalError> {
        let crate::CreateReadiness::Observed(ready) = &view.readiness else {
            return Ok(None);
        };
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let world = tx.world();
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        super::authenticated(&inbox, &view.current_request, context)?;
        let (s, e) = inbox
            .find(&view.current_request)?
            .ok_or(JournalError::Conflict)?;
        let state = inbox.state(s, e);
        if !state.current
            || state.terminal
            || view.local != state.local
            || inbox.sessions[s]
                .current_execution_origin()
                .is_none_or(|origin| origin.intent != view.request)
            || ready.basis.native_input_high_water
                != crate::controller::allocators::input_high_water(&tx)?
        {
            return Err(JournalError::Conflict);
        }
        let local = state.local.ok_or(JournalError::Conflict)?;
        let row = world
            .sandbox_record(local.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        let current =
            super::readiness::snapshot::native_snapshot(&tx, &self.namespace, &world, local, row)?;
        let Some(UnloadResolution::BeforeAdmission { original, result }) =
            &mut inbox.sessions[s].entries[e].unload_resolution
        else {
            return Err(JournalError::Conflict);
        };
        if **original != current
            || view.native.as_ref() != Some(&current)
            || ready.observed_at_ms > u64::try_from(now_ms).map_err(|_| JournalError::Conflict)?
        {
            return Err(JournalError::Conflict);
        }
        let facts = ready_facts(current.lifetime.ok_or(JournalError::CorruptStore)?, now_ms)?;
        if let Some(old) = result {
            return Ok(Some(old.clone()));
        }
        *result = Some(facts.clone());
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(Some(facts))
    }
}
