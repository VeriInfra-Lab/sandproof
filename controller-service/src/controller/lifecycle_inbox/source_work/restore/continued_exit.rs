//! Two retained failed-Unload qualifications; physical work keeps its own fence.
use super::{codec, validate_record, work};
use crate::capture::{
    OriginalContinueExitObservation, OriginalContinueWorkRequest, RestoreCandidateExitObservation,
    RestoreWorkRequest, SourceProcessIdentity,
};
use crate::controller::lifecycle_inbox::{
    codec as inbox_codec, readiness::snapshot, unload_resolution::UnloadResolution,
};
use crate::controller::{capture, data_credentials};
use crate::{
    ControllerJournal, HostEvidence, InternalLocalBinding, JournalError, NativeCreateSnapshot,
};
use sandbox_controller_core as core;
use sandproof_protocol::{
    WorkAction,
    lifecycle::{
        AcceptedLifecycleIntent, AuthenticatedCenterContext, ContentDigest, CurrentReadyFacts,
        CurrentResumeReadiness, LifecycleOperation, ResumeObservation,
    },
};
use serde::de::DeserializeOwned;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ContinuedAuthority {
    original: AcceptedLifecycleIntent,
    current: AcceptedLifecycleIntent,
    local: InternalLocalBinding,
    native: NativeCreateSnapshot,
    row: core::SandboxRecord,
    resolution: UnloadResolution,
    local_operation: Option<core::OperationId>,
    credential: (core::Generation, [u8; 32]),
}

/// No Pause or Full belongs to this current Unload. Its saved original is the
/// unchanged Resume; this is distinct from the direct Resume exit qualifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BeforeAdmissionExitCandidate {
    pub(crate) work: RestoreWorkRequest,
    pub(crate) action: WorkAction<core::EffectId>,
    authority: ContinuedAuthority,
    work_binding: ContentDigest,
    process: SourceProcessIdentity,
    receipts: Vec<Vec<u8>>,
}
impl BeforeAdmissionExitCandidate {
    pub(crate) fn matches(&self, observed: &RestoreCandidateExitObservation) -> bool {
        observed.action == self.action
            && observed.work_binding == self.work_binding
            && observed.process == self.process
    }
    pub(crate) fn current_request(&self) -> &AcceptedLifecycleIntent {
        &self.authority.current
    }
}

/// A real saved continuation, including when a later unadmitted Unload kept
/// that exact execution. It always uses this work's actual Pause fence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContinuedExitCandidate {
    pub(crate) work: OriginalContinueWorkRequest,
    pub(crate) action: WorkAction<core::EffectId>,
    authority: ContinuedAuthority,
    work_binding: ContentDigest,
    receipt: Vec<u8>,
}
impl ContinuedExitCandidate {
    pub(crate) fn matches(&self, observed: &OriginalContinueExitObservation) -> bool {
        observed.action == self.action
            && observed.work_binding == self.work_binding
            && observed.process == self.work.source().source
    }
    pub(crate) fn current_request(&self) -> &AcceptedLifecycleIntent {
        &self.authority.current
    }
}

pub(crate) type ContinuedExitCandidates = (
    Option<BeforeAdmissionExitCandidate>,
    Option<ContinuedExitCandidate>,
);

fn lifetime_matches(native: &NativeCreateSnapshot, saved: &CurrentReadyFacts, now: i64) -> bool {
    let Some(lifetime) = native.lifetime else {
        return false;
    };
    let Some(started) = lifetime.started_at_ms else {
        return false;
    };
    if now < 0
        || started < 0
        || lifetime.deadline_ms < 0
        || lifetime.max_deadline_ms < 0
        || lifetime.startup_deadline_ms < 0
    {
        return false;
    }
    // All values are nonnegative i64, hence exactly representable as u64.
    #[allow(
        clippy::cast_sign_loss,
        reason = "negative lifetime values rejected above"
    )]
    let matches = saved.started_at_ms == started as u64
        && saved.end_at_ms == lifetime.deadline_ms as u64
        && saved.max_end_at_ms == lifetime.max_deadline_ms as u64
        && saved.startup_deadline_ms == lifetime.startup_deadline_ms as u64
        && saved.observed_at_ms <= now as u64;
    matches
        && started <= now
        && started <= lifetime.startup_deadline_ms
        && now < lifetime.deadline_ms
        && now < lifetime.max_deadline_ms
}

fn authority(
    tx: &rusqlite::Transaction<'_>,
    world: &core::World,
    namespace: &str,
    original: &AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
    now: i64,
) -> Result<Option<ContinuedAuthority>, JournalError> {
    let inbox = inbox_codec::load(tx)?.ok_or(JournalError::UnsupportedStore)?;
    super::super::super::authenticated(&inbox, original, context)?;
    let (s, _) = inbox.find(original)?.ok_or(JournalError::Conflict)?;
    let session = &inbox.sessions[s];
    let Some(e) = session
        .entries
        .iter()
        .position(|entry| entry.intent.accepted_version == session.current_version)
    else {
        return Err(JournalError::CorruptStore);
    };
    let entry = &session.entries[e];
    let state = inbox.state(s, e);
    let observation = ResumeObservation {
        request: original.clone(),
        current_request: entry.intent.clone(),
        readiness: CurrentResumeReadiness::StateLost,
    };
    if !matches!(
        original.request.operation,
        LifecycleOperation::Resume { .. }
    ) || !state.current
        || state.terminal
        || session
            .current_execution_origin()
            .is_none_or(|origin| origin.intent != *original)
        || !observation.matches_request(original)
        || !matches!(
            entry.intent.request.operation,
            LifecycleOperation::Unload { .. }
        )
    {
        return Ok(None);
    }
    let Some(resolution) = entry.unload_resolution.as_ref() else {
        return Ok(None);
    };
    let Some(saved) = resolution.result() else {
        return Ok(None);
    };
    let Some(local) = state.local else {
        return Ok(None);
    };
    let row = world
        .sandbox_record(local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let native = snapshot::native_snapshot(tx, namespace, world, local, row)?;
    if native.terminal
        || native.phase != core::Phase::Running
        || !native.uncertain_actions.is_empty()
        || !lifetime_matches(&native, saved, now)
    {
        return Ok(None);
    }
    let eligible = match resolution {
        UnloadResolution::BeforeAdmission {
            original: saved, ..
        } => state.local_operation.is_none() && **saved == native,
        UnloadResolution::AfterCapture { .. } => {
            native.current_operation.is_some_and(|operation| {
                state.local_operation == Some(operation.id)
                    && operation.kind == core::OperationKind::Pause
                    && matches!(operation.status, core::OperationStatus::Failed(_))
            })
        }
    };
    if !eligible {
        return Ok(None);
    }
    let credential = match data_credentials::snapshot(tx, world, namespace, local.sandbox, true) {
        Ok(Some(value)) if value.0 == native.generation => value,
        Ok(_) | Err(JournalError::CredentialIndexPoisoned) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(ContinuedAuthority {
        original: original.clone(),
        current: entry.intent.clone(),
        local,
        native,
        row: row.clone(),
        resolution: resolution.clone(),
        local_operation: state.local_operation,
        credential,
    }))
}

fn candidates<E: HostEvidence + DeserializeOwned>(
    tx: &rusqlite::Transaction<'_>,
    world: &core::World,
    namespace: &str,
    authority: ContinuedAuthority,
    now: i64,
) -> Result<ContinuedExitCandidates, JournalError> {
    if authority
        .native
        .has_completed_original_execution(authority.local)
    {
        if !matches!(
            &authority.resolution,
            UnloadResolution::BeforeAdmission { .. }
        ) {
            return Ok((None, None));
        }
        let sources = codec::load(tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = sources
            .records
            .iter()
            .find(|record| {
                record
                    .restore
                    .as_ref()
                    .is_some_and(|claim| claim.intent == authority.original)
            })
            .ok_or(JournalError::CorruptStore)?;
        validate_record(record, world)?;
        let claim = record.restore.as_ref().ok_or(JournalError::CorruptStore)?;
        let Some(action) = claim.activation.as_ref() else {
            return Ok((None, None));
        };
        let work = work(record)?;
        if claim.local != authority.local
            || !super::exit::eligible(authority.local, &authority.native, &work, action, now)
        {
            return Ok((None, None));
        }
        let (actions, receipts) =
            snapshot::startup_records::<E>(tx, namespace, world, authority.local)?;
        if !actions.contains(action) {
            return Err(JournalError::CorruptStore);
        }
        let process = capture::original_source::<E>(tx, namespace, world, authority.local.sandbox)?;
        return Ok((
            Some(BeforeAdmissionExitCandidate {
                work_binding: work.binding_digest()?,
                work,
                action: action.clone(),
                authority,
                process,
                receipts,
            }),
            None,
        ));
    }
    let Some((work, action, receipt)) = super::super::full::continued_ready_records::<E>(
        tx,
        world,
        namespace,
        authority.local,
        &authority.native,
    )?
    else {
        return Ok((None, None));
    };
    if let UnloadResolution::AfterCapture { .. } = &authority.resolution {
        let LifecycleOperation::Unload {
            source_incarnation,
            capture,
        } = &authority.current.request.operation
        else {
            return Err(JournalError::CorruptStore);
        };
        if work.full().pause.binding.operation != authority.current.request.binding
            || work.full().pause.binding.source_incarnation != *source_incarnation
            || work.full().pause.binding.capture != *capture
            || authority.local_operation != Some(work.identity().key.operation)
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok((
        None,
        Some(ContinuedExitCandidate {
            work_binding: work.binding_digest()?,
            work,
            action,
            authority,
            receipt,
        }),
    ))
}

impl ControllerJournal {
    /// Reconstruct current continuation qualification without any dispatch.
    pub(crate) fn lifecycle_continued_exit_candidates<E: HostEvidence + DeserializeOwned>(
        &self,
        original: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        now: i64,
    ) -> Result<ContinuedExitCandidates, JournalError> {
        if !self.startup_epoch_ready()? {
            return Ok((None, None));
        }
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        inbox_codec::validate(&tx, &world)?;
        let result = match authority(&tx, &world, &self.namespace, original, context, now)? {
            Some(authority) => candidates::<E>(&tx, &world, &self.namespace, authority, now)?,
            None => (None, None),
        };
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(result)
    }
}
