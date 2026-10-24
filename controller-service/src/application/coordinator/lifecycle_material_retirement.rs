//! Final terminal material cleanup uses the original application and Host port.
//! The original saved plan is committed before any physical delete; unknown
//! effects keep its full charge and are retried with those same bytes.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, Inner, NeverCrash, core, map_journal,
};
use crate::MaterialRetirementQuestion;
use core::maintenance::retirement::{QuiescenceCertificate, QuiescenceStatus};
use std::sync::Arc;

struct MaterialRetirementGuard<'a, H: ControllerHost> {
    application: &'a ControllerApplication<H>,
    sandbox: core::SandboxId,
    session: String,
}
impl<H: ControllerHost> Drop for MaterialRetirementGuard<'_, H> {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.application.inner.lock() {
            inner.active_sandboxes.remove(&self.sandbox);
            inner.active_sessions.remove(&self.session);
        }
    }
}

fn routes_drained<H: ControllerHost>(inner: &Inner<H>, sandbox: core::SandboxId) -> bool {
    inner
        .data_revocations
        .get(&sandbox.0)
        .is_none_or(|signals| {
            signals
                .iter()
                .all(|signal| *signal.borrow() && signal.receiver_count() == 0)
        })
        && inner
            .data_drains
            .get(&sandbox.0)
            .is_none_or(super::DataLease::drained)
}

fn certificate<H: ControllerHost>(
    inner: &Inner<H>,
    question: &MaterialRetirementQuestion,
) -> Result<QuiescenceCertificate, ApplicationError> {
    let sandbox = question.request.sandbox();
    if !routes_drained(inner, sandbox) {
        return Err(ApplicationError::Busy);
    }
    let world = inner.journal.world().map_err(map_journal)?;
    core::maintenance::retirement::validate_terminal_cleanup(&world, sandbox)
        .map_err(|_| ApplicationError::Busy)?;
    let row = world
        .sandbox_record(sandbox)
        .ok_or(ApplicationError::Busy)?;
    // The journal separately verifies real writer-end, all native uses, exact
    // release witness and external pins before accepting this certificate.
    Ok(QuiescenceCertificate {
        sandbox,
        owner: row.owner,
        epoch: row.epoch,
        operation: row.current_operation.ok_or(ApplicationError::Busy)?.id,
        workspace: row.workspace,
        worker_drained: QuiescenceStatus::Confirmed,
        data_connections_drained: QuiescenceStatus::Confirmed,
        host_resources_reclaimed: QuiescenceStatus::Confirmed,
        external_pins_released: QuiescenceStatus::Confirmed,
    })
}

impl<H: ControllerHost> ControllerApplication<H> {
    /// Advance only final terminal material obligations. This is called by the
    /// existing bounded maintenance driver, never a second GC controller.
    /// # Errors
    /// Busy users, unknown deletion or mismatched original evidence stay charged.
    pub fn advance_terminal_lifecycle_material(
        &self,
        sandbox: core::SandboxId,
    ) -> Result<usize, ApplicationError> {
        let (questions, host, session) = {
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let questions = inner
                .journal
                .lifecycle_material_retirement_questions(sandbox)
                .map_err(map_journal)?;
            let Some(first) = questions.first() else {
                return Ok(0);
            };
            let session = first
                .source
                .request()
                .request
                .binding
                .session
                .as_protocol_id()
                .as_str()
                .to_owned();
            if inner.active_sandboxes.contains(&sandbox)
                || inner.active_sessions.contains(&session)
                || questions.iter().any(|question| {
                    question.source.request().request.binding.session
                        != first.source.request().request.binding.session
                })
            {
                return Err(ApplicationError::Busy);
            }
            inner.revoke_data(sandbox);
            if !routes_drained(&inner, sandbox) {
                return Err(ApplicationError::Busy);
            }
            inner.active_sandboxes.insert(sandbox);
            inner.active_sessions.insert(session.clone());
            (questions, Arc::clone(&inner.host), session)
        };
        let _active = MaterialRetirementGuard {
            application: self,
            sandbox,
            session,
        };
        let count = questions.len();
        for question in questions {
            self.retire_one_material(&question, &host)?;
        }
        Ok(count)
    }

    fn retire_one_material(
        &self,
        question: &MaterialRetirementQuestion,
        host: &H,
    ) -> Result<(), ApplicationError> {
        let saved = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .saved_lifecycle_material_plan(question)
                .map_err(map_journal)?
        };
        let bytes = match saved {
            Some(bytes) => bytes,
            None => host.prepare_lifecycle_material_retirement(&question.request)?,
        };
        let permit = {
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let closure = certificate(&inner, question)?;
            inner
                .journal
                .prepare_lifecycle_material_retirement(question, &bytes, closure, &mut NeverCrash)
                .map_err(map_journal)?
        };
        // Outside SQL and Inner: errors return without changing Retiring or its
        // reservation. In particular, do not replace a partly deleted plan.
        let observation = host.retire_lifecycle_material(&question.request, &bytes, &permit)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let closure = certificate(&inner, question)?;
        inner
            .journal
            .settle_lifecycle_material_retirement(question, &observation, closure, &mut NeverCrash)
            .map_err(map_journal)
    }
}
