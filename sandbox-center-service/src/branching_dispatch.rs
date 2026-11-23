//! Dispatch original plans without holding the center authority across host I/O.
use crate::{
    CenterApplication, RegisteredHosts, TemplateProvider, business::SharedBusinessAuthority,
};
use sandproof_api::{
    PortError,
    admission::{RequestAdmission, RequestClass},
};
use std::sync::Arc;
use tokio::task::{JoinSet, spawn_blocking};

pub const MAX_BRANCH_DISPATCHES_PER_ROUND: usize = 4;

impl<T: TemplateProvider> CenterApplication<SharedBusinessAuthority, RegisteredHosts, T> {
    /// Project current bounded work from the original center ledger. Each
    /// detached worker retains its admission lease until its real effects end.
    /// # Errors
    /// Authority failure or a failed worker prevents claiming a successful round.
    pub async fn branch_background_round(
        self: &Arc<Self>,
        admission: &Arc<RequestAdmission>,
        cursor: &mut usize,
    ) -> Result<(), PortError> {
        let Some(lease) = admission.try_acquire(RequestClass::Background) else {
            return Ok(());
        };
        let app = Arc::clone(self);
        let mut work = spawn_blocking(move || {
            let _lease = lease;
            app.authority
                .lock()
                .map_err(|_| PortError::OutcomeUnknown)?
                .pending_branch_work(app.now()?)
        })
        .await
        .map_err(|_| PortError::OutcomeUnknown)??;
        if work.is_empty() {
            *cursor = 0;
            return Ok(());
        }
        let available = work.len();
        work.rotate_left(*cursor % available);
        let mut tasks = JoinSet::new();
        for request in work.into_iter().take(MAX_BRANCH_DISPATCHES_PER_ROUND) {
            let Some(lease) = admission.try_acquire(RequestClass::Background) else {
                break;
            };
            *cursor = cursor.wrapping_add(1);
            let app = Arc::clone(self);
            tasks.spawn(async move {
                let Ok(client) = app.hosts.client(&request.parent.node) else {
                    return Ok(());
                };
                let Ok(reply) = client.branch_control(&request).await else {
                    return Ok(());
                };
                spawn_blocking(move || {
                    let _lease = lease;
                    app.authority
                        .lock()
                        .map_err(|_| PortError::OutcomeUnknown)?
                        .observe_branch_work(&request, &reply, app.now()?)
                })
                .await
                .map_err(|_| PortError::OutcomeUnknown)?
            });
        }
        while let Some(result) = tasks.join_next().await {
            match result.map_err(|_| PortError::OutcomeUnknown)? {
                Ok(()) | Err(PortError::Conflict | PortError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}
