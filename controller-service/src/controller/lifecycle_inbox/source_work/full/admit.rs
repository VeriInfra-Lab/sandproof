use super::super::{ControllerJournal, SourceWork, authenticated, snapshot};
use super::{
    CaptureWork, FullCaptureWorkRequest, JournalError, SourceWorkRecord, codec, core, reservation,
    save, validate,
};
use crate::{
    CrashInjector, CrashPoint, HostEvidence,
    physical_storage::{CaptureAdmission, CapturePhysicalBytes, PhysicalStorageProfile},
};
use sandproof_protocol::lifecycle::AuthenticatedCenterContext;
use serde::de::DeserializeOwned;

impl ControllerJournal {
    pub(crate) fn lifecycle_capture_profile(
        &self,
    ) -> Result<Option<PhysicalStorageProfile>, JournalError> {
        Ok(self
            .bounded_host_config()?
            .and_then(|config| config.physical_storage)
            .filter(|profile| {
                profile
                    .service_data
                    .iter()
                    .any(|service| service.on_demand_capture)
            }))
    }

    /// Accept the original Pause and fixed Full responsibility in one transaction.
    /// # Errors
    /// Changed source, request, deadline, profile or bounded capacity refuses.
    /// On-demand deployments require the application’s leased volume observation.
    pub fn admit_lifecycle_full_capture<E: HostEvidence + DeserializeOwned>(
        &mut self,
        work: &SourceWork,
        context: &AuthenticatedCenterContext,
        full: &FullCaptureWorkRequest,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<SourceWorkRecord, JournalError> {
        self.admit_lifecycle_full_capture_with_space::<E>(
            work,
            context,
            (full, None),
            now_ms,
            crashes,
        )
    }

    pub(crate) fn admit_lifecycle_full_capture_with_space<E: HostEvidence + DeserializeOwned>(
        &mut self,
        work: &SourceWork,
        context: &AuthenticatedCenterContext,
        proposal: (&FullCaptureWorkRequest, Option<CaptureAdmission>),
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<SourceWorkRecord, JournalError> {
        let (full, physical) = proposal;
        let observation_missing = physical.is_none() && self.lifecycle_capture_profile()?.is_some();
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
        let (session, entry) = inbox.find(work.request())?.ok_or(JournalError::Conflict)?;
        if inbox.sessions[session].entries[entry]
            .unload_resolution
            .is_some()
        {
            return Err(JournalError::Conflict);
        }
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
        if observation_missing {
            return Err(JournalError::ControllerBusy);
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
        let mut held_physical = CapturePhysicalBytes::default();
        let held = sources.records.iter().try_fold(0_u64, |sum, r| {
            let charge = super::super::retirement::material_charge(r)?;
            if physical.is_some() && charge != 0 {
                let prior = r.full_capture().ok_or(JournalError::CorruptStore)?;
                held_physical =
                    held_physical.checked_add(CapturePhysicalBytes::for_capture(prior)?)?;
            }
            sum.checked_add(charge)
                .ok_or(JournalError::MaintenanceCapacity)
        })?;
        let total = held
            .checked_add(reserved_material_bytes)
            .ok_or(JournalError::MaintenanceCapacity)?;
        if total > sources.config.max_material_bytes {
            // Only this same-transaction, definite material refusal carries
            // the pre-admission guarantee. Overflow and later storage/commit
            // failures retain their original classification.
            return Err(JournalError::CaptureMaterialQuotaExceeded);
        }

        if let Some(admission) = physical
            && !admission.fits(held_physical, CapturePhysicalBytes::for_capture(full)?)?
        {
            return Err(JournalError::CaptureMaterialQuotaExceeded);
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
            end: None,
        });
        save(&tx, &sources)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(sources.records[i].clone())
    }
}
