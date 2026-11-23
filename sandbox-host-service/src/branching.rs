//! Original branch control through the protected native parent.
use super::{ApplicationError, ControllerApplication, ControllerHost, HostLifecycle, ReceiveError};
use sandproof_controller_service::application::branching::{
    BranchAccessIdentity, BranchAccessSelector, BranchControlAction, BranchControlReply,
    BranchControlRequest, BranchControlStatus, BranchStartFailure, BranchTarget,
};
use sandproof_controller_service::application::{
    HostBranchBatchKey, HostBranchBatchPhase, HostBranchClose, HostBranchParent, HostGuestBranch,
};
use sandproof_protocol::{ResourceId, SandboxId, lifecycle::AuthenticatedCenterContext};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// The guest framing limit is separate from the center-to-host HTTP envelope.
const GUEST_CONTROL_BODY_BYTES: usize = 16 * 1024;

#[derive(Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum GuestCommand<'a> {
    Start {
        plan: GuestPlan<'a>,
    },
    Observe {
        batch_id: &'a ResourceId,
    },
    Release {
        batch_id: &'a ResourceId,
        #[serde(skip_serializing_if = "Option::is_none")]
        branch_id: Option<&'a SandboxId>,
    },
}

#[derive(Serialize)]
struct GuestPlan<'a> {
    batch_id: &'a ResourceId,
    checkpoint_id: &'a ResourceId,
    branches: Vec<GuestBranch<'a>>,
}

#[derive(Serialize)]
struct GuestBranch<'a> {
    branch_id: &'a SandboxId,
    port: u16,
    access_token: &'a str,
}

impl<'a> GuestPlan<'a> {
    fn new(request: &'a BranchControlRequest, branches: &'a [HostGuestBranch]) -> Self {
        Self {
            batch_id: &request.batch.batch,
            checkpoint_id: &request.batch.checkpoint,
            branches: branches
                .iter()
                .map(|branch| GuestBranch {
                    branch_id: &branch.target().sandbox,
                    port: branch.target().port.get(),
                    access_token: branch.token(),
                })
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum GuestReply {
    Pending {
        batch_id: ResourceId,
    },
    Lost {
        batch_id: ResourceId,
    },
    Ready {
        batch_id: ResourceId,
        branches: Vec<GuestReadyBranch>,
        released: Vec<SandboxId>,
    },
    FailedClean {
        batch_id: ResourceId,
    },
    NeverStarted {
        batch_id: ResourceId,
    },
    Released {
        batch_id: ResourceId,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestReadyBranch {
    branch_id: SandboxId,
    port: std::num::NonZeroU16,
}

enum GuestObservation {
    Inconclusive,
    Ready {
        active: Vec<BranchTarget>,
        released: Vec<SandboxId>,
    },
    FailedClean,
    NeverStarted,
    Released,
}

impl GuestObservation {
    fn from_wire(bytes: &[u8], request: &BranchControlRequest) -> Option<Self> {
        if bytes.len() > GUEST_CONTROL_BODY_BYTES {
            return None;
        }
        let reply: GuestReply = serde_json::from_slice(bytes).ok()?;
        let batch = &request.batch.batch;
        Some(match reply {
            GuestReply::Pending { batch_id } | GuestReply::Lost { batch_id }
                if &batch_id == batch =>
            {
                Self::Inconclusive
            }
            GuestReply::FailedClean { batch_id } if &batch_id == batch => Self::FailedClean,
            GuestReply::NeverStarted { batch_id } if &batch_id == batch => Self::NeverStarted,
            GuestReply::Released { batch_id } if &batch_id == batch => Self::Released,
            GuestReply::Ready {
                batch_id,
                branches,
                released,
            } if &batch_id == batch && !branches.is_empty() => {
                let retired: HashSet<_> = released.iter().collect();
                if retired.len() != released.len()
                    || request.targets.len() != branches.len() + retired.len()
                {
                    return None;
                }
                let active: Vec<_> = branches
                    .into_iter()
                    .map(|entry| BranchTarget {
                        sandbox: entry.branch_id,
                        port: entry.port,
                    })
                    .collect();
                if !request
                    .targets
                    .iter()
                    .filter(|target| !retired.contains(&target.sandbox))
                    .eq(active.iter())
                {
                    return None;
                }
                Self::Ready { active, released }
            }
            _ => return None,
        })
    }

    fn proves_gone(&self, branch: Option<&SandboxId>) -> bool {
        match self {
            Self::Released | Self::FailedClean => true,
            Self::Ready { released, .. } => branch.is_some_and(|branch| released.contains(branch)),
            Self::Inconclusive | Self::NeverStarted => false,
        }
    }
}

impl<H: ControllerHost + Send + 'static> HostLifecycle<H> {
    /// Exact native control for the authenticated host work queue. Only the
    /// original protected driver can exchange with the guest.
    /// # Errors
    /// Invalid identity and overload refuse work. Unknown results retain the
    /// original batch and do not prove absence.
    pub async fn drive_branch_control<B: Send + 'static>(
        &self,
        request: BranchControlRequest,
        context: AuthenticatedCenterContext,
        prepare: impl FnOnce(&H, &HostBranchParent) -> Result<Option<B>, ApplicationError>
        + Send
        + 'static,
        exchange: impl FnOnce(&B, &[u8]) -> Result<Vec<u8>, ApplicationError> + Send + 'static,
    ) -> Result<BranchControlReply, ReceiveError> {
        if !request.valid_for(&context)
            || !guest_identifier(request.batch.batch.as_str())
            || request
                .targets
                .iter()
                .any(|target| !guest_identifier(target.sandbox.as_str()))
        {
            return Err(ReceiveError::Conflict);
        }
        let cleanup = matches!(request.action, BranchControlAction::Release { .. });
        let permit = self.work_permit(cleanup).map_err(receive_error)?;
        self.execute_blocking(permit, move |app| {
            let status = drive(app, &request, &context, prepare, exchange)?;
            Ok(BranchControlReply {
                original: request,
                status,
            })
        })
        .await
        .map_err(receive_error)
    }
}

fn guest_identifier(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

fn receive_error(error: ApplicationError) -> ReceiveError {
    match error {
        ApplicationError::QuotaExceeded => ReceiveError::Capacity,
        _ => ReceiveError::Unavailable,
    }
}

fn drive<H: ControllerHost, B>(
    app: &ControllerApplication<H>,
    request: &BranchControlRequest,
    context: &AuthenticatedCenterContext,
    prepare: impl FnOnce(&H, &HostBranchParent) -> Result<Option<B>, ApplicationError>,
    exchange: impl FnOnce(&B, &[u8]) -> Result<Vec<u8>, ApplicationError>,
) -> Result<BranchControlStatus, ApplicationError> {
    let key = HostBranchBatchKey {
        operation: request.operation.clone(),
        batch: request.batch.clone(),
    };
    let parent = &request.parent;
    let batch_id = &request.batch.batch;
    let reserve = matches!(
        request.action,
        BranchControlAction::Start {} | BranchControlAction::Release { branch: None }
    );
    let Some((control, phase)) =
        app.prepare_host_branch_control(parent, context, &key, &request.targets, reserve, prepare)?
    else {
        return if matches!(request.action, BranchControlAction::Start {}) {
            Ok(BranchControlStatus::Failed {
                reason: BranchStartFailure::CheckpointUnavailable,
            })
        } else {
            Err(ApplicationError::OutcomeUnknown)
        };
    };
    let private_plan;
    let command = match &request.action {
        BranchControlAction::Start {} if phase == HostBranchBatchPhase::Pending => {
            private_plan = app.pending_host_branch_guest_plan(parent, context, &key)?;
            GuestCommand::Start {
                plan: GuestPlan::new(request, &private_plan),
            }
        }
        BranchControlAction::Release { branch } => {
            let branch = branch.as_ref();
            app.change_host_branch_close(parent, context, &key, HostBranchClose::Begin(branch))?;
            GuestCommand::Release {
                batch_id,
                branch_id: branch,
            }
        }
        _ => GuestCommand::Observe { batch_id },
    };
    let encoded = sandproof_controller_service::template_registry::evidence::encode_bounded(
        &command,
        GUEST_CONTROL_BODY_BYTES,
    )
    .map_err(|_| ApplicationError::Invalid)?;
    let response = exchange(control.driver(), &encoded);
    let observation = response
        .ok()
        .and_then(|bytes| GuestObservation::from_wire(&bytes, request))
        .unwrap_or(GuestObservation::Inconclusive);
    let failure = match (&request.action, &observation) {
        (
            BranchControlAction::Start {} | BranchControlAction::Observe {},
            GuestObservation::FailedClean,
        ) => Some(BranchStartFailure::GuestSetupFailed),
        (BranchControlAction::Start {}, GuestObservation::NeverStarted)
            if matches!(
                phase,
                HostBranchBatchPhase::Pending | HostBranchBatchPhase::Released
            ) =>
        {
            Some(BranchStartFailure::GuestCapacity)
        }
        _ => None,
    };
    if let Some(reason) = failure {
        app.change_host_branch_close(parent, context, &key, HostBranchClose::Begin(None))?;
        drop(control);
        app.change_host_branch_close(parent, context, &key, HostBranchClose::ConfirmGone(None))?;
        return Ok(BranchControlStatus::Failed { reason });
    }
    match (&request.action, observation) {
        (BranchControlAction::Release { branch }, observation) => {
            if !observation.proves_gone(branch.as_ref()) {
                return Ok(BranchControlStatus::CleaningUp {});
            }
            drop(control);
            app.change_host_branch_close(
                parent,
                context,
                &key,
                HostBranchClose::ConfirmGone(branch.as_ref()),
            )?;
            Ok(BranchControlStatus::Released {})
        }
        (BranchControlAction::Start {}, GuestObservation::Ready { active, released })
            if released.is_empty() =>
        {
            app.publish_host_branch_batch(parent, context, &key, &active)?;
            arm_observed_routes(app, request, context, &active)?;
            Ok(BranchControlStatus::Ready {})
        }
        (BranchControlAction::Observe {}, GuestObservation::Ready { active, .. })
            if phase == HostBranchBatchPhase::Ready =>
        {
            arm_observed_routes(app, request, context, &active)?;
            // The center already has this Ready batch. The observation only
            // refreshes local permission after host management restart.
            Ok(BranchControlStatus::Pending {})
        }
        _ => Ok(BranchControlStatus::Pending {}),
    }
}

fn arm_observed_routes<H: ControllerHost>(
    app: &ControllerApplication<H>,
    request: &BranchControlRequest,
    context: &AuthenticatedCenterContext,
    active: &[BranchTarget],
) -> Result<(), ApplicationError> {
    for target in active {
        let identity = BranchAccessIdentity {
            parent: request.parent.clone(),
            selector: BranchAccessSelector {
                operation: request.operation.clone(),
                batch: request.batch.clone(),
                branch: target.sandbox.clone(),
            },
        };
        match app.observe_host_branch_data_ready(&identity, context, target) {
            // A concurrent release or parent expiry wins over an old ready
            // observation. Other active branches may still be rearmed.
            Ok(()) | Err(ApplicationError::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
