use super::super::{ControllerJournal, SourceWork, authenticated, snapshot};
use super::{CaptureWork, FullCaptureWorkRequest, JournalError, SourceWorkRecord, codec, core, reservation, save, validate};
use crate::{CrashInjector, CrashPoint, HostEvidence};
use sandproof_protocol::lifecycle::AuthenticatedCenterContext;
use serde::de::DeserializeOwned;

impl ControllerJournal {
    /// Accept the original Pause and fixed Full responsibility in one transaction.
    /// # Errors
    /// Changed source, request, deadline, profile or bounded capacity refuses.
    pub fn admit_lifecycle_full_capture<E: HostEvidence + DeserializeOwned>(
        &mut self,
        work: &SourceWork,
        context: &AuthenticatedCenterContext,
        full: &FullCaptureWorkRequest,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<SourceWorkRecord, JournalError> {
        let local = work.anchor.local;
        let operation = self.next_operation_id(local.sandbox);
        let active = self.active_host_epoch.clone();
        let tx = crate::controller::transaction::WorldTransaction::begin(
            &mut self.journal.connection,
            &self.committed_world,
        )?;
        let mut inbox =
            super::super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, work.request(), context)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let i = sources
            .records
            .iter()
            .position(|r| r.work == *work)
            .ok_or(JournalError::Conflict)?;
        if let Some(old) = &sources.records[i].capture {
            if old.full != *full {
                return Err(JournalError::Conflict);
            }
            return Ok(sources.records[i].clone());
        }
        let current = snapshot::current::<E>(
            &tx,
            &self.namespace,
            work.request(),
            now_ms,
            snapshot::CurrentSource {
                world: &tx.world(),
                inbox: &inbox,
                limits: &sources.config,
                active: active.as_ref(),
            },
        )?;
        if current != *work || sources.records[i].observation.is_none() || !full.valid() {
            return Err(JournalError::UnresolvedAction);
        }
        validate::full_binding(&sources.records[i], full)?;
        let reserved_material_bytes = reservation(full)?;
        let held = sources.records.iter().try_fold(0_u64, |sum, r| {
            sum.checked_add(r.capture.as_ref().map_or(0, |c| c.reserved_material_bytes))
                .ok_or(JournalError::MaintenanceCapacity)
        })?;
        if held
            .checked_add(reserved_material_bytes)
            .is_none_or(|n| n > sources.config.max_material_bytes)
        {
            return Err(JournalError::MaintenanceCapacity);
        }

        let operation = operation?;
        let (s, e) = inbox.find(work.request())?.ok_or(JournalError::Conflict)?;
        crate::controller::storage::apply(
            &tx,
            &self.namespace,
            core::ControllerInput::Request(core::Request::Pause(core::PauseRequest {
                actor: core::Actor::Tenant(local.owner),
                sandbox: local.sandbox,
                operation,
            })),
        )?;
        inbox.sessions[s].entries[e].local_operation = Some(operation);
        super::super::super::codec::save(&tx, &inbox)?;
        sources.records[i].capture = Some(CaptureWork {
            operation,
            reserved_material_bytes,
            full: full.clone(),
            action: None,
            worker: None,
            completed: None,
            material_bytes: None,
        });
        save(&tx, &sources)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(sources.records[i].clone())
    }
}
