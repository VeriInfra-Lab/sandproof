//! Bounded allowlisted view of the already validated ORIGINAL control block.
//! Not an authorizing port or another recovery/lifecycle implementation.
use crate::entry::Envelope;
use sandproof_center_core::{AcceptanceTime, NodeRegistration, RetentionLane, SessionPhase};
use sandproof_center_store::Revision;
use sandproof_controller_service::CreateOptions;
use sandproof_protocol::{
    DeploymentId, ResourceId,
    internal::lifecycle::Resources,
    lifecycle::{
        AcceptedLifecycleIntent, CaptureId, CenterIdentity, ContentDigest, CreateRefusalReason,
        CurrentReadyFacts, IncarnationId, LifecycleOperation, OperationBinding, RetentionBinding,
        TerminalLifecycleObservation,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

#[derive(Serialize)]
pub struct AuditView<'a> {
    schema_version: u16,
    versions: sandproof_protocol::internal::ContractVersions,
    deployment: &'a DeploymentId,
    revision: Revision,
    sandbox_highwater: u64,
    operation_highwater: u64,
    nodes: &'a [NodeRegistration],
    retention_lanes: &'a [RetentionLane],
    sandboxes: Vec<SandboxAudit<'a>>,
}

#[derive(Serialize)]
struct CreateAudit<'a> {
    binding: &'a OperationBinding,
    template_revision: &'a ResourceId,
    template_digest: &'a [u8; 32],
    resources: Resources,
    options_sha256: String,
    startup_deadline_ms: u64,
    timeout_ms: u64,
    max_lifetime_ms: u64,
    /// Diagnostic correlation only. Even known metadata keys contain user data;
    /// none of their original values is exported or used as authorization.
    metadata_sha256: String,
}
#[derive(Serialize)]
struct OperationAudit<'a> {
    center: &'a CenterIdentity,
    binding: &'a OperationBinding,
    retention: RetentionBinding,
    accepted_version: u64,
    acceptance_time: AcceptanceTime,
    operation: OperationKind,
    running: Option<&'a CurrentReadyFacts>,
    create_refusal: Option<CreateRefusalReason>,
    terminal: Option<&'a TerminalLifecycleObservation>,
}
/// The complete operation parameters except user Create options. Those are
/// represented only by the allowlisted CreateAudit fields and content hashes.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OperationKind {
    Create { target_incarnation: IncarnationId },
    Unload { source_incarnation: IncarnationId, capture: CaptureId },
    Resume { source_incarnation: IncarnationId, capture: CaptureId, target_incarnation: IncarnationId, manifest_digest: ContentDigest },
    Renew { incarnation: IncarnationId, end_at_ms: i64, extend_only: bool },
    Destroy,
}
impl From<&LifecycleOperation> for OperationKind {
    fn from(operation: &LifecycleOperation) -> Self {
        match operation {
            LifecycleOperation::Create { target_incarnation, .. } => Self::Create { target_incarnation: *target_incarnation },
            LifecycleOperation::Unload { source_incarnation, capture } => Self::Unload { source_incarnation: *source_incarnation, capture: *capture },
            LifecycleOperation::Resume { source_incarnation, capture, target_incarnation, manifest_digest } => Self::Resume {
                source_incarnation: *source_incarnation, capture: *capture, target_incarnation: *target_incarnation, manifest_digest: *manifest_digest,
            },
            LifecycleOperation::Renew { incarnation, end_at_ms, extend_only } => Self::Renew { incarnation: *incarnation, end_at_ms: *end_at_ms, extend_only: *extend_only },
            LifecycleOperation::Destroy {} => Self::Destroy,
        }
    }
}
#[derive(Serialize)]
struct SandboxAudit<'a> {
    create: CreateAudit<'a>,
    private: bool,
    operations: Vec<OperationAudit<'a>>,
    terminating: bool,
    released: bool,
    phase: SessionPhase,
    /// A historical committed lifetime, never evidence of current readiness.
    last_running: Option<&'a CurrentReadyFacts>,
    terminal_confirmation: Option<&'a TerminalLifecycleObservation>,
}

pub fn view(envelope: &Envelope, revision: Revision) -> Result<AuditView<'_>, &'static str> {
    let hidden = envelope.metadata.private_sandboxes();
    let sandboxes = envelope
        .center
        .sandboxes()
        .iter()
        .map(|record| {
            let create = record
                .operations()
                .first()
                .ok_or("invalid original center operation history")?
                .intent();
            Ok(SandboxAudit {
                create: create_view(create)?,
                private: hidden.contains(create.request.binding.session.as_protocol_id()),
                operations: record
                    .operations()
                    .iter()
                    .map(|operation| OperationAudit {
                        center: &operation.intent().center,
                        binding: &operation.intent().request.binding,
                        retention: operation.intent().retention,
                        accepted_version: operation.intent().accepted_version,
                        acceptance_time: operation.acceptance_time(),
                        operation: OperationKind::from(&operation.intent().request.operation),
                        running: operation.running(),
                        create_refusal: operation.create_refusal(),
                        terminal: operation.terminal(),
                    })
                    .collect(),
                terminating: record.terminating(),
                released: record.released(),
                phase: record.phase(),
                last_running: record.last_running(),
                terminal_confirmation: record.terminal_confirmation.as_ref(),
            })
        })
        .collect::<Result<_, &'static str>>()?;
    let (sandbox_highwater, operation_highwater) = envelope.center.identity_frontiers();
    Ok(AuditView {
        schema_version: 3,
        versions: sandproof_protocol::internal::ContractVersions::current(),
        deployment: envelope.center.deployment(),
        revision,
        sandbox_highwater,
        operation_highwater,
        nodes: envelope.center.nodes(),
        retention_lanes: envelope.center.retention_lanes(),
        sandboxes,
    })
}

fn create_view(
    intent: &AcceptedLifecycleIntent,
) -> Result<CreateAudit<'_>, &'static str> {
    let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
        return Err("invalid original center create operation");
    };
    let options: CreateOptions = serde_json::from_slice(&create.options)
        .map_err(|_| "invalid original center options")?;
    options
        .validate()
        .map_err(|_| "invalid original center options")?;
    let metadata =
        serde_json::to_vec(&options.metadata).map_err(|_| "invalid original center metadata")?;
    Ok(CreateAudit {
        binding: &intent.request.binding,
        template_revision: &create.template_revision,
        template_digest: &create.template_digest,
        resources: create.resources,
        options_sha256: format!("{:x}", Sha256::digest(&create.options)),
        metadata_sha256: format!("{:x}", Sha256::digest(&metadata)),
        startup_deadline_ms: create.startup_deadline_ms,
        timeout_ms: create.timeout_ms,
        max_lifetime_ms: create.max_lifetime_ms,
    })
}
