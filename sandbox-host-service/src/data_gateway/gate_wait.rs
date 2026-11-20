//! Only definite helper-gate contention before relay launch permits a retry.
use super::{
    DataAccessPort, DataAuthorization, DataKind, PortError, RequestLease, drain, until_revoked,
};
use axum::http::StatusCode;
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

const GATE_POLL: Duration = Duration::from_millis(10);

pub(super) struct GateWait {
    idle: Option<Duration>,
    deadline: Option<Instant>,
}

impl GateWait {
    pub(super) const fn new(idle: Option<Duration>) -> Self {
        Self {
            idle,
            deadline: None,
        }
    }

    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    async fn retry(&mut self) -> bool {
        let now = Instant::now();
        let Some(deadline) = self.deadline.or_else(|| now.checked_add(self.idle?)) else {
            return false;
        };
        self.deadline = Some(deadline);
        if now >= deadline {
            return false;
        }
        tokio::time::sleep_until((now + GATE_POLL).min(deadline)).await;
        !self.expired()
    }
}

type Grant<P> = (P, Option<String>);

pub(super) async fn authorize_data<P, F>(
    lease: &Arc<RequestLease>,
    kind: DataKind,
    authorize: &Arc<F>,
    wait: &mut GateWait,
) -> Result<Grant<P>, StatusCode>
where
    P: DataAuthorization,
    F: Fn(DataKind) -> Result<Grant<P>, PortError> + Send + Sync + 'static,
{
    loop {
        let authorize = Arc::clone(authorize);
        let lease = Arc::clone(lease);
        let result = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            authorize(kind)
        })
        .await;
        let error = match result {
            Ok(Ok(grant)) if !wait.expired() => return Ok(grant),
            Ok(Ok(_)) => return Err(StatusCode::SERVICE_UNAVAILABLE),
            Ok(Err(PortError::NotFound | PortError::Invalid)) => {
                return Err(StatusCode::UNAUTHORIZED);
            }
            Ok(Err(error)) => error,
            Err(_) => {
                log(super::FaultStage::Join, PortError::Busy);
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
        };
        if error != PortError::DataGateBusy || !wait.retry().await {
            log(super::FaultStage::HostDataAuthorization, error);
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
    }
}

pub(super) async fn launch_authorized<P, F>(
    port: &Arc<P>,
    mut permit: P::Permit,
    (kind, control, guest_credential): (DataKind, bool, Option<&str>),
    lease: &Arc<RequestLease>,
    authorize: &Arc<F>,
    mut wait: GateWait,
) -> Result<(P::Permit, drain::RelayProcess), StatusCode>
where
    P: DataAccessPort,
    F: Fn(DataKind) -> Result<Grant<P::Permit>, PortError> + Send + Sync + 'static,
{
    let destination = permit.target();
    let mut original_revocation = permit.revocation().clone();
    loop {
        match try_launch(
            Arc::clone(port),
            permit,
            control,
            Arc::clone(lease),
            wait.deadline,
        )
        .await
        {
            Ok(launched) => return Ok(launched),
            Err(LaunchFailure::Status(status)) => return Err(status),
            Err(LaunchFailure::GateBusy) => {}
        }
        // try_launch has dropped the permit and its shared gate. Keeping it
        // while waiting could block exclusive recovery of the earlier helper.
        let grant = until_revoked(&mut original_revocation, async {
            if !wait.retry().await {
                log(super::FaultStage::Relay, PortError::DataGateBusy);
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
            authorize_data(lease, kind, authorize, &mut wait).await
        })
        .await
        .ok_or(StatusCode::UNAUTHORIZED)??;
        let (mut candidate, credential) = grant;
        if candidate.target() != destination
            || credential.as_deref() != guest_credential
            || !candidate.revocation().same_channel(&original_revocation)
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
        permit = candidate;
    }
}

enum LaunchFailure {
    GateBusy,
    Status(StatusCode),
}

async fn try_launch<P: DataAccessPort>(
    port: Arc<P>,
    permit: P::Permit,
    control: bool,
    request_lease: Arc<RequestLease>,
    deadline: Option<Instant>,
) -> Result<(P::Permit, drain::RelayProcess), LaunchFailure> {
    use LaunchFailure::Status;
    let (permit, launched) = tokio::task::spawn_blocking(move || {
        let _lease = request_lease;
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(Status(StatusCode::SERVICE_UNAVAILABLE));
        }
        let result = port.spawn_data_relay(&permit, control);
        Ok((permit, result))
    })
    .await
    .map_err(|_| Status(StatusCode::BAD_GATEWAY))??;
    match launched {
        Ok(child) => Ok((permit, drain::RelayProcess::new(child))),
        Err(PortError::DataGateBusy) => Err(LaunchFailure::GateBusy),
        Err(PortError::NotFound | PortError::Invalid) => Err(Status(StatusCode::UNAUTHORIZED)),
        Err(PortError::QuotaExceeded) => Err(Status(StatusCode::TOO_MANY_REQUESTS)),
        Err(error) => {
            log(super::FaultStage::Relay, error);
            Err(Status(StatusCode::SERVICE_UNAVAILABLE))
        }
    }
}

#[cfg(test)]
pub(super) async fn launch_data_route<P: DataAccessPort>(
    port: Arc<P>,
    permit: P::Permit,
    control: bool,
    lease: Arc<RequestLease>,
) -> Result<(P::Permit, drain::RelayProcess), StatusCode> {
    try_launch(port, permit, control, lease, None)
        .await
        .map_err(|error| match error {
            LaunchFailure::GateBusy => StatusCode::SERVICE_UNAVAILABLE,
            LaunchFailure::Status(status) => status,
        })
}

fn log(stage: super::FaultStage, error: PortError) {
    super::FaultRecord::application(stage, &error, super::FaultContext::default()).emit();
}
