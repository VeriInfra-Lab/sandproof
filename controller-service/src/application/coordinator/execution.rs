//! Short journal decisions, one physical executor per logical sandbox, and
//! shared host calls outside the application lock. The outbox remains durable.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, DataLease, Inner, NeverCrash, core,
    data_credentials, map_admission, map_journal, same_secret, token,
};
use std::sync::Arc;

pub(super) struct ActiveSandbox<'a, H: ControllerHost> {
    application: &'a ControllerApplication<H>,
    sandbox: core::SandboxId,
}
pub(super) struct ActiveSession<'a, H: ControllerHost> {
    application: &'a ControllerApplication<H>,
    session: String,
}
impl<H: ControllerHost> Drop for ActiveSession<'_, H> {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.application.inner.lock() {
            inner.active_sessions.remove(&self.session);
        }
    }
}
impl<H: ControllerHost> Drop for ActiveSandbox<'_, H> {
    fn drop(&mut self) {
        // A host panic happens outside this mutex. Persistent Dispatched rows
        // remain unresolved even after this process-local occupancy disappears.
        if let Ok(mut inner) = self.application.inner.lock() {
            inner.active_sandboxes.remove(&self.sandbox);
        }
    }
}

impl<H: ControllerHost> ControllerApplication<H> {
    pub(super) fn acquire_lifecycle_session(
        &self,
        intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
        context: &sandproof_protocol::lifecycle::AuthenticatedCenterContext,
    ) -> Result<Option<ActiveSession<'_, H>>, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let state = inner
            .journal
            .lifecycle_work_state(intent, context)
            .map_err(map_journal)?;
        if !state.current || state.terminal {
            return Ok(None);
        }
        let session = intent
            .request
            .binding
            .session
            .as_protocol_id()
            .as_str()
            .to_owned();
        if !inner.active_sessions.insert(session.clone()) {
            return Err(ApplicationError::Busy);
        }
        Ok(Some(ActiveSession {
            application: self,
            session,
        }))
    }
    /// Record deadlines before scheduling host work. No host operation or
    /// semaphore wait occurs while this authority lock is held.
    /// # Errors
    /// Invalid time and uncertain storage stop the scan without dropping work.
    pub fn accept_due_work(&self) -> Result<(), ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let now = self.clock.now_ms()?;
        inner
            .journal
            .abort_due_startups(now, &mut NeverCrash)
            .map_err(map_admission)?;
        for sandbox in inner.journal.due_lifetimes(now).map_err(map_admission)? {
            let operation = super::next_operation(&inner.journal, sandbox)?;
            let result = inner
                .journal
                .accept_expiry(sandbox, operation, now, &mut NeverCrash);
            if result.is_ok()
                || matches!(
                    result,
                    Err(crate::AdmissionError::Journal(
                        crate::JournalError::CommitUncertain(_)
                    ))
                )
            {
                inner.revoke_data(sandbox);
            }
            result.map_err(map_admission)?;
        }
        Ok(())
    }

    /// A finite scan of native responsibility; never a second authoritative
    /// queue. The boolean selects a reserved termination/cleanup work budget.
    /// # Errors
    /// Fenced or invalid authority refuses the complete scan.
    pub fn pending_workflows(&self) -> Result<Vec<(core::SandboxId, bool)>, ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let world = inner.journal.world().map_err(map_journal)?;
        let reclaimed = world
            .sandbox_records()
            .iter()
            .filter_map(|(sandbox, _)| {
                runtime_resources_reclaimed(&world, *sandbox).then_some(*sandbox)
            })
            .collect::<std::collections::BTreeSet<_>>();
        let pending_release = inner
            .journal
            .host_slot_inventory()
            .map_err(map_journal)?
            .into_iter()
            .filter_map(|(context, phase)| {
                (phase != core::maintenance::slots::HostSlotPhase::Free)
                    .then_some(context.binding.sandbox)
            })
            .collect::<std::collections::BTreeSet<_>>();
        Ok(world
            .sandbox_records()
            .iter()
            .filter_map(|(sandbox, record)| {
                (!inner.active_sandboxes.contains(sandbox)
                    && (!reclaimed.contains(sandbox) || pending_release.contains(sandbox))
                    && record.phase != core::Phase::Running)
                    .then_some((*sandbox, record.terminal))
            })
            .collect())
    }

    pub(super) fn acquire_sandbox(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<ActiveSandbox<'_, H>, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        inner.journal.world().map_err(map_journal)?;
        if !inner.active_sandboxes.insert(sandbox) {
            return Err(ApplicationError::Busy);
        }
        Ok(ActiveSandbox {
            application: self,
            sandbox,
        })
    }

    fn prepare_execution(
        &self,
        sandbox: core::SandboxId,
        host: &H,
    ) -> Result<(), ApplicationError> {
        let initial = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let world = inner.journal.world().map_err(map_journal)?;
            let record = world
                .sandbox_record(sandbox)
                .ok_or(ApplicationError::NotFound)?;
            if record.phase == core::Phase::Starting
                && !inner.journal.startup_epoch_ready().map_err(map_journal)?
            {
                return Err(ApplicationError::Busy);
            }
            if let Some((context, _)) = inner
                .journal
                .host_slot_inventory()
                .map_err(map_journal)?
                .into_iter()
                .find(|(context, phase)| {
                    context.binding.sandbox == sandbox
                        && *phase == core::maintenance::slots::HostSlotPhase::Occupied
                })
            {
                // Slot binding has a small cross-position identity reservation;
                // keep it serialized, never put launch/capture behind this lock.
                host.bind_host_slot(
                    &context,
                    super::recovery::can_install_host_slot_binding(&world, sandbox),
                )?;
            }
            if record.phase != core::Phase::Starting || record.terminal {
                return Ok(());
            }
            let options = inner
                .journal
                .create_options(sandbox)
                .map_err(map_admission)?;
            let registered = inner
                .journal
                .data_credential(sandbox)
                .map_err(map_journal)?;
            let key = core::OperationKey {
                sandbox,
                operation: record
                    .current_operation
                    .ok_or(ApplicationError::OutcomeUnknown)?
                    .id,
            };
            let unclaimed = world.effect_ids().iter().all(|effect| {
                world
                    .logical_effect(&effect.logical)
                    .is_none_or(|logical| logical.origin != core::EffectOrigin::Operation(key))
                    || world.effect_status(effect) == Some(core::EffectStatus::Committed)
            });
            (options, registered, unclaimed, record.generation)
        };
        let (options, registered, unclaimed, generation) = initial;
        if registered.is_none() && !unclaimed {
            return Err(ApplicationError::OutcomeUnknown);
        }
        if !host.has_bound_launch_config(sandbox, generation, &options)? {
            if registered.is_some() || !unclaimed {
                return Err(ApplicationError::OutcomeUnknown);
            }
            host.put_bound_launch_config(sandbox, generation, &token()?, &options)?;
        }
        let secret = host.token_for(sandbox, generation)?;
        let digest = data_credentials::digest(&secret)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        if inner
            .journal
            .create_options(sandbox)
            .map_err(map_admission)?
            != options
        {
            return Err(ApplicationError::OutcomeUnknown);
        }
        let world = inner.journal.world().map_err(map_journal)?;
        if world
            .sandbox_record(sandbox)
            .is_none_or(|record| record.generation != generation)
        {
            return Err(ApplicationError::OutcomeUnknown);
        }
        match inner
            .journal
            .data_credential(sandbox)
            .map_err(map_journal)?
        {
            Some((stored_generation, stored))
                if stored_generation == generation && same_secret(&stored, &digest) =>
            {
                Ok(())
            }
            None if unclaimed => inner
                .journal
                .register_data_credential(sandbox, generation, digest, &mut NeverCrash)
                .map_err(map_journal),
            Some(_) | None => Err(ApplicationError::OutcomeUnknown),
        }
    }

    /// One bounded turn for an original sandbox. Host I/O uses the same shared
    /// host; no global management or host mutex is held during execution.
    /// # Errors
    /// Active, uncertain and conflicting responsibility stays durable; no
    /// absent in-process worker is interpreted as absence of physical effects.
    pub fn advance_current_workflow(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<bool, ApplicationError> {
        let _active = self.acquire_sandbox(sandbox)?;
        let host = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            Arc::clone(&inner.host)
        };
        self.prepare_unload_drain(sandbox, host.as_ref())?;
        self.prepare_execution(sandbox, host.as_ref())?;
        self.reconcile_current_actions(sandbox, host.as_ref())?;
        self.prepare_original_continue_if_ended(sandbox, host.as_ref())?;
        for _ in 0..128 {
            // The failed Pause can have no reserved snapshot. Publish its exact
            // acknowledged continuation before the ordinary Pause scheduler
            // considers reserving one on the already closed plan.
            if let Some(published) =
                self.publish_original_continue_if_ready(sandbox, host.as_ref())?
            {
                return Ok(published);
            }
            let action = {
                let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
                inner
                    .journal
                    .abort_due_startups(self.clock.now_ms()?, &mut NeverCrash)
                    .map_err(map_admission)?;
                let world = inner.journal.world().map_err(map_journal)?;
                match core::workflow::next_step(&world, sandbox) {
                    core::workflow::WorkflowStep::Apply(input) => {
                        inner
                            .journal
                            .apply(*input, &mut NeverCrash)
                            .map_err(map_journal)?;
                        continue;
                    }
                    core::workflow::WorkflowStep::Claim => {
                        let Some(action) = inner
                            .journal
                            .claim_next(Some(sandbox), &mut NeverCrash)
                            .map_err(map_journal)?
                        else {
                            // Epoch/unresolved guards in the authoritative claim
                            // may veto the preview. Yield instead of busy-spinning.
                            return Ok(false);
                        };
                        action
                    }
                    core::workflow::WorkflowStep::PublishRunning => {
                        inner
                            .journal
                            .publish_running_at(
                                sandbox,
                                self.clock.publication_ms().map_err(map_journal)?,
                                &mut NeverCrash,
                            )
                            .map_err(map_journal)?;
                        continue;
                    }
                    core::workflow::WorkflowStep::Complete => {
                        drop(inner);
                        return self.release_current_slot(sandbox, host.as_ref());
                    }
                    core::workflow::WorkflowStep::Blocked => {
                        drop(inner);
                        if self.prepare_original_continue_if_ended(sandbox, host.as_ref())? {
                            continue;
                        }
                        return Ok(self
                            .publish_original_continue_if_ready(sandbox, host.as_ref())?
                            .unwrap_or(false));
                    }
                }
            };
            let result = match self.execute_capture_action(host.as_ref(), &action) {
                Ok(result) => result,
                Err(error) => {
                    if self.prepare_original_continue_if_ended(sandbox, host.as_ref())? {
                        continue;
                    }
                    return Err(error);
                }
            };
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            if let Ok(receipt) = result {
                inner
                    .journal
                    .complete(&receipt, &mut NeverCrash)
                    .map_err(map_journal)?;
            } else {
                drop(inner);
                if self.prepare_original_continue_if_ended(sandbox, host.as_ref())? {
                    continue;
                }
                return Ok(false);
            }
        }
        // A finite turn is a fairness bound, not a failure or a new job queue.
        Ok(false)
    }

    // The caller owns ActiveSandbox; host observations remain outside the lock.
    fn reconcile_current_actions(
        &self,
        sandbox: core::SandboxId,
        host: &H,
    ) -> Result<(), ApplicationError> {
        // Reconciliation observes only original Dispatched/Unknown identities.
        // It cannot race an in-process worker: this turn owns the session gate.
        let uncertain = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .uncertain_actions()
                .map_err(map_journal)?
                .into_iter()
                .filter(|action| action.command.command().sandbox() == sandbox)
                .collect::<Vec<_>>()
        };
        for action in uncertain {
            let receipt = match self.observe_capture_action(host, &action)? {
                Some(receipt) => Some(receipt),
                None => match self.continue_original_activation(host, &action)? {
                    Some(receipt) => Some(receipt),
                    None => self.continue_restore_activation(host, &action)?,
                },
            };
            if let Some(receipt) = receipt {
                let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
                // complete loads the latest World and validates the full action.
                // A terminal result can add physical facts, never grant access.
                inner
                    .journal
                    .reconcile_observed(&action, &receipt)
                    .map_err(map_journal)?;
            }
        }
        Ok(())
    }

    // The caller owns ActiveSandbox. All checks remain against original state;
    // no local boolean is accepted as proof of physical release.
    fn release_current_slot(
        &self,
        sandbox: core::SandboxId,
        host: &H,
    ) -> Result<bool, ApplicationError> {
        let candidate = {
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let world = inner.journal.world().map_err(map_journal)?;
            if !runtime_resources_reclaimed(&world, sandbox) {
                return Ok(world
                    .sandbox_record(sandbox)
                    .is_some_and(|r| r.phase == core::Phase::Running));
            }
            let Some((context, _)) = inner
                .journal
                .host_slot_inventory()
                .map_err(map_journal)?
                .into_iter()
                .find(|(c, p)| {
                    c.binding.sandbox == sandbox
                        && *p != core::maintenance::slots::HostSlotPhase::Free
                })
            else {
                return Ok(true);
            };
            inner.revoke_data(sandbox);
            if !local_routes_dropped(&inner, sandbox)
                || inner
                    .journal
                    .host_slot_release_pinned(sandbox)
                    .map_err(map_journal)?
            {
                return Ok(false);
            }
            let known_exit = inner
                .data_drains
                .get(&sandbox.0)
                .is_none_or(DataLease::drained);
            (context, known_exit)
        };
        let (context, known_exit) = candidate;
        if !known_exit && !super::recovery::host_routes_drained(host, &context) {
            return Ok(false);
        }
        {
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let world = inner.journal.world().map_err(map_journal)?;
            if !runtime_resources_reclaimed(&world, sandbox)
                || !local_routes_dropped(&inner, sandbox)
            {
                return Ok(false);
            }
            // This validates the full current context, terminal resource facts
            // and durable pins and commits Releasing before touching the host.
            inner
                .journal
                .begin_host_slot_release(context, &mut NeverCrash)
                .map_err(map_journal)?;
        }
        let unloaded = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let world = inner.journal.world().map_err(map_journal)?;
            if world
                .sandbox_record(sandbox)
                .is_some_and(|r| r.phase == core::Phase::Paused && !r.terminal)
            {
                let record = inner
                    .journal
                    .lifecycle_capture_record(sandbox)
                    .map_err(map_journal)?
                    .ok_or(ApplicationError::OutcomeUnknown)?;
                let expected = inner
                    .journal
                    .committed_capture_expectation(&record.work)
                    .map_err(map_journal)?
                    .ok_or(ApplicationError::OutcomeUnknown)?;
                Some((record, expected))
            } else {
                None
            }
        };
        if let Some((record, expected)) = &unloaded {
            host.release_lifecycle_host_slot(
                &context,
                record
                    .full_capture()
                    .ok_or(ApplicationError::OutcomeUnknown)?,
                record.work.requirement(),
                expected,
            )?;
        } else {
            host.release_host_slot(&context)?;
        }
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        if !local_routes_dropped(&inner, sandbox) {
            return Err(ApplicationError::OutcomeUnknown);
        }
        inner
            .journal
            .confirm_host_slot_release(context, &mut NeverCrash)
            .map_err(map_journal)?;
        inner.data_revocations.remove(&sandbox.0);
        inner.data_drains.remove(&sandbox.0);
        Ok(true)
    }
}

pub(super) fn local_routes_dropped<H: ControllerHost>(
    inner: &Inner<H>,
    sandbox: core::SandboxId,
) -> bool {
    inner
        .data_revocations
        .get(&sandbox.0)
        .is_none_or(|signals| {
            signals
                .iter()
                .all(|sender| *sender.borrow() && sender.receiver_count() == 0)
        })
        && inner
            .data_drains
            .get(&sandbox.0)
            .is_none_or(DataLease::routes_dropped)
}

fn runtime_resources_reclaimed(world: &core::World, sandbox: core::SandboxId) -> bool {
    if super::recovery::reclaimed_sandboxes(world).contains(&sandbox) {
        return true;
    }
    let Some(row) = world.sandbox_record(sandbox) else {
        return false;
    };
    if row.phase != core::Phase::Paused || row.terminal || row.durable_snapshot.is_none() {
        return false;
    }
    let resources = world.resource_index(sandbox);
    resources.vms.iter().all(|vm| {
        world
            .vm_record(*vm)
            .is_some_and(|v| v.status == core::VmStatus::Reclaimed)
    }) && resources.slots.iter().all(|slot| {
        world
            .slot_record(*slot)
            .is_some_and(|s| s.reclaimed && !s.active)
    })
}
