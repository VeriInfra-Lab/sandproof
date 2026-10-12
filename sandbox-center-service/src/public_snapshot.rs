//! Public visibility and lifecycle facts from ONE confirmed authority revision.
//! Internal validation objects retain ordinary lifecycle/cleanup responsibility,
//! but never grant a public token or appear in SDK queries/management.
use crate::PublicTemplate;
use crate::application::find;
use sandproof_api::PortError;
use sandproof_center_core::{CenterState, Sandbox};
use sandproof_protocol::{ResourceId, SandboxId, TenantId};

#[derive(Clone, Debug)]
pub struct PublicSnapshot {
    pub state: CenterState,
    pub hidden_sandboxes: Vec<SandboxId>,
    /// Immutable bindings projected from the SAME confirmed envelope, not
    /// resolved again through a provider or a mutable alias during a query.
    pub templates: Vec<PublicTemplate>,
}
impl PublicSnapshot {
    /// # Errors
    /// Missing or changed original bindings never fall back to current aliases.
    pub fn bound_template(
        &self,
        revision: &ResourceId,
        digest: &[u8; 32],
    ) -> Result<&PublicTemplate, PortError> {
        self.templates
            .iter()
            .find(|template| template.revision() == revision && template.digest() == *digest)
            .ok_or(PortError::NotFound)
    }
    #[must_use]
    pub fn visible(&self, record: &Sandbox) -> bool {
        record.operations.first().is_some_and(|operation| {
            !self
                .hidden_sandboxes
                .contains(&operation.request.identity.sandbox_id)
        })
    }
    /// # Errors
    /// Private validation objects and foreign owners are indistinguishable from
    /// absent public sandboxes, even when their original VM is already Ready.
    pub fn find(&self, tenant: &TenantId, sandbox: &SandboxId) -> Result<&Sandbox, PortError> {
        let record = find(&self.state, tenant, sandbox)?;
        if self.visible(record) {
            Ok(record)
        } else {
            Err(PortError::NotFound)
        }
    }
}
