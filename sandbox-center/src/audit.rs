//! Bounded allowlisted view of the already validated ORIGINAL control block.
//! Not an authorizing port or another recovery/lifecycle implementation.
use crate::entry::Envelope;
use sandproof_center_core::{NodeRegistration, NodeWindow};
use sandproof_center_store::Revision;
use sandproof_controller_service::CreateOptions;
use sandproof_protocol::{
    DeploymentId, ResourceId,
    internal::{
        ControlIdentity,
        lifecycle::{Intent, LifecycleResponse, Resources},
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
    windows: Vec<&'a NodeWindow>,
    sandboxes: Vec<SandboxAudit<'a>>,
}

#[derive(Serialize)]
struct CreateAudit<'a> {
    identity: &'a ControlIdentity,
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
    identity: &'a ControlIdentity,
    intent: Intent,
    response: Option<&'a LifecycleResponse>,
}
#[derive(Serialize)]
struct SandboxAudit<'a> {
    create: CreateAudit<'a>,
    private: bool,
    operations: Vec<OperationAudit<'a>>,
    terminating: bool,
    released: bool,
    started_at_ms: Option<u64>,
    end_at_ms: Option<u64>,
    max_end_at_ms: Option<u64>,
    terminal_confirmation: Option<&'a LifecycleResponse>,
}

pub fn view(envelope: &Envelope, revision: Revision) -> Result<AuditView<'_>, &'static str> {
    let hidden = envelope.metadata.private_sandboxes();
    let sandboxes = envelope
        .center
        .sandboxes()
        .iter()
        .map(|record| {
            let create = &record
                .operations
                .first()
                .ok_or("invalid original center operation history")?
                .request;
            Ok(SandboxAudit {
                create: create_view(create)?,
                private: hidden.contains(&create.identity.sandbox_id),
                operations: record
                    .operations
                    .iter()
                    .map(|operation| OperationAudit {
                        identity: &operation.request.identity,
                        intent: operation.request.intent,
                        response: operation.response.as_ref(),
                    })
                    .collect(),
                terminating: record.terminating,
                released: record.released,
                started_at_ms: record.started_at_ms,
                end_at_ms: record.end_at_ms,
                max_end_at_ms: record.max_end_at_ms,
                terminal_confirmation: record.terminal_confirmation.as_ref(),
            })
        })
        .collect::<Result<_, &'static str>>()?;
    let (sandbox_highwater, operation_highwater) = envelope.center.identity_frontiers();
    let windows = envelope
        .center
        .nodes()
        .iter()
        .map(|node| {
            envelope
                .center
                .window(&node.identity)
                .ok_or("invalid original center window")
        })
        .collect::<Result<_, _>>()?;
    Ok(AuditView {
        schema_version: 1,
        versions: sandproof_protocol::internal::ContractVersions::current(),
        deployment: envelope.center.deployment(),
        revision,
        sandbox_highwater,
        operation_highwater,
        nodes: envelope.center.nodes(),
        windows,
        sandboxes,
    })
}

fn create_view(
    create: &sandproof_protocol::internal::lifecycle::LifecycleRequest,
) -> Result<CreateAudit<'_>, &'static str> {
    let options: CreateOptions = serde_json::from_slice(&create.create.options)
        .map_err(|_| "invalid original center options")?;
    options
        .validate()
        .map_err(|_| "invalid original center options")?;
    let metadata =
        serde_json::to_vec(&options.metadata).map_err(|_| "invalid original center metadata")?;
    Ok(CreateAudit {
        identity: &create.identity,
        template_revision: &create.create.template_revision,
        template_digest: &create.create.template_digest,
        resources: create.create.resources,
        options_sha256: format!("{:x}", Sha256::digest(&create.create.options)),
        metadata_sha256: format!("{:x}", Sha256::digest(&metadata)),
        startup_deadline_ms: create.create.startup_deadline_ms,
        timeout_ms: create.create.timeout_ms,
        max_lifetime_ms: create.create.max_lifetime_ms,
    })
}
