//! Material cleanup uses the original application and Host port.
//! The original saved plan is committed before any physical delete; unknown
//! effects keep its full charge and are retried with those same bytes.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, NeverCrash, core, map_journal,
};
use crate::MaterialRetirementQuestion;
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

impl<H: ControllerHost> ControllerApplication<H> {
    /// Advance exact material obligations. This is called by the
    /// existing bounded maintenance driver, never a second GC controller.
    /// # Errors
    /// Busy users, unknown deletion or mismatched original evidence stay charged.
    pub fn advance_lifecycle_material(
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
            // The original producer/consumer ConfirmRelease already saved
            // their real drain/physical-end facts. Never revoke or wait for
            // unrelated connections of a newer execution here.
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
            inner
                .journal
                .prepare_lifecycle_material_retirement(question, &bytes, &mut NeverCrash)
                .map_err(map_journal)?
        };
        // Outside SQL and Inner: errors return without changing Retiring or its
        // reservation. In particular, do not replace a partly deleted plan.
        let observation = host
            .retire_lifecycle_material(&question.request, &bytes, &permit)
            .map_err(|_| ApplicationError::OutcomeUnknown)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        inner
            .journal
            .settle_lifecycle_material_retirement(question, &observation, &mut NeverCrash)
            .map_err(map_journal)
    }
}
