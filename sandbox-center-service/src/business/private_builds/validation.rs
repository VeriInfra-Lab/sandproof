//! Cross-reference checking and retirement use exact original global records.
use super::{PrivateBuilds, owner};
use crate::business::{PrivateBuildConfig, RegistryMetadata};
use sandproof_center_core::{CenterState, LanePhase, NodeRole};
use sandproof_controller_service::{CreateOptions, template_registry::valid_candidate_id};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, LifecycleOperation};

impl PrivateBuilds {
    fn configuration_valid(&self, metadata: &RegistryMetadata, center: &CenterState) -> bool {
        if !self.config.valid()
            || !center
                .nodes()
                .iter()
                .any(|node| node.role == NodeRole::Builder)
            || self.windows.open_windows().iter().any(|window| {
                !window
                    .tenant()
                    .0
                    .to_string()
                    .try_into()
                    .is_ok_and(|tenant| center.tenant_is_registered(&tenant))
            })
            || self.windows.validate().is_err()
            || self.windows.limits() != self.config.windows
            || self.anchors.len() > self.config.max_anchors
            || !metadata.registrations.iter().any(|known| {
                known.revision == self.config.builder_revision
                    && metadata.catalog.templates.iter().any(|binding| {
                        binding.template_id == known.template_id
                            && binding.owner.is_none()
                            && binding
                                .canonical_digest()
                                .is_ok_and(|digest| digest == self.config.builder_digest)
                    })
            })
        {
            return false;
        }
        true
    }
    pub(super) fn valid_for(&self, metadata: &RegistryMetadata, center: &CenterState) -> bool {
        if !self.configuration_valid(metadata, center) {
            return false;
        }
        self.anchors.iter().enumerate().all(|(index, anchor)| {
            let create = &anchor.original_create;
            if owner(&anchor.tenant).is_err()
                || !valid_candidate_id(&anchor.build_id)
                || self.anchors[..index].iter().any(|old| {
                    old.original_create.request.binding.session == create.request.binding.session
                        || (old.tenant == anchor.tenant
                            && old.window == anchor.window
                            && old.build_id == anchor.build_id)
                })
                || self
                    .windows
                    .issued_through()
                    .is_none_or(|last| anchor.window.value() > last)
                || self
                    .windows
                    .window(anchor.window)
                    .is_some_and(|window| owner(&anchor.tenant).ok() != Some(window.tenant()))
                || create.request.binding.key.owner != anchor.tenant
                || create.request.binding.node.deployment_id != *center.deployment()
                || create.accepted_version != 1
                || create.accepted_version_for_request() != Some(1)
                || !matches!(create.request.operation, LifecycleOperation::Create { .. })
                || create
                    .request
                    .validate(&create.request.binding.node)
                    .is_err()
                || &create.center != center.center_identity()
                || !RegistryMetadata::request_fits(create)
                || !self.matches_parameters(metadata, create, &anchor.parameters)
                || !center.nodes().iter().any(|node| {
                    node.identity == create.request.binding.node && node.role == NodeRole::Builder
                })
            {
                return false;
            }
            let Some(lane) = center.lane(&create.request.binding.node, create.retention.slot)
            else {
                return false;
            };
            let live = center.sandboxes().iter().find(|sandbox| {
                sandbox.operations.first().is_some_and(|first| {
                    first.intent().request.binding.session == create.request.binding.session
                })
            });
            if anchor.settlement.is_none()
                && (lane.phase() == LanePhase::ClosePending || live.is_none())
            {
                return false;
            }
            if let Some(cleaned) = &anchor.settlement {
                if self.windows.window(anchor.window).is_some()
                    || crate::business::publication::terminal_request(create, cleaned).is_none()
                {
                    return false;
                }
                if live.is_none() && lane.retired_through() < create.retention.window.get() {
                    return false;
                }
            }
            live.is_none_or(|record| {
                record
                    .operations
                    .first()
                    .is_some_and(|first| first.intent() == create)
                    && anchor.settlement.as_ref().is_none_or(|cleaned| {
                        record.operations.last().is_some_and(|last| {
                            crate::business::publication::terminal_request(create, cleaned).as_ref()
                                == Some(last.intent())
                                && last.terminal() == Some(cleaned)
                        })
                    })
            })
        })
    }
    pub(super) fn matches_parameters(
        &self,
        metadata: &RegistryMetadata,
        intent: &AcceptedLifecycleIntent,
        parameters: &super::BuildParameters,
    ) -> bool {
        let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
            return false;
        };
        let Ok(options) = serde_json::from_slice::<CreateOptions>(&create.options) else {
            return false;
        };
        let Ok(binding) =
            metadata.artifact_binding(&self.config.builder_revision, &self.config.builder_digest)
        else {
            return false;
        };
        let resources = sandproof_protocol::internal::lifecycle::Resources {
            instances: 1,
            cpu: u64::from(binding.resources.cpu_count),
            memory_bytes: binding.resources.host_memory_bytes,
            disk_bytes: binding.resources.disk_bytes,
        };
        create.template_revision == self.config.builder_revision
            && create.template_digest == self.config.builder_digest
            && binding.owner.is_none()
            && create.resources == resources
            && parameters.timeout_secs.checked_mul(1000) == Some(create.timeout_ms)
            && parameters.timeout_secs > 0
            && parameters.timeout_secs <= self.config.policy.max_timeout_secs
            && self.config.policy.max_lifetime_secs.checked_mul(1000)
                == Some(create.max_lifetime_ms)
            && (!parameters.allow_internet_access || self.config.policy.public_ipv4_enabled)
            && options.validate().is_ok()
            && options.template.is_none()
            && options.env_vars.is_empty()
            && options.metadata.is_empty()
            && options.networking.as_ref().is_some_and(|network| {
                network.network.is_none()
                    && network.allow_internet_access == Some(parameters.allow_internet_access)
            })
    }
    pub(in crate::business) fn retire(&self, center: &CenterState) -> Self {
        let mut next = self.clone();
        next.anchors.retain(|anchor| {
            anchor.settlement.is_none()
                || center.sandboxes().iter().any(|record| {
                    record.operations.first().is_some_and(|first| {
                        first.intent().request.binding.session
                            == anchor.original_create.request.binding.session
                    })
                })
                || center
                    .lane(
                        &anchor.original_create.request.binding.node,
                        anchor.original_create.retention.slot,
                    )
                    .is_none_or(|lane| {
                        lane.retired_through() < anchor.original_create.retention.window.get()
                    })
        });
        next
    }
}
impl RegistryMetadata {
    /// Internal finite authority view; never serialize through the public API.
    #[must_use]
    pub fn private_build_anchors(&self) -> &[super::GlobalBuildAnchor] {
        self.private_builds
            .as_ref()
            .map_or(&[], |builds| &builds.anchors)
    }
    pub(in crate::business) fn private_valid_for(&self, center: &CenterState) -> bool {
        self.private_builds
            .as_ref()
            .is_none_or(|builds| builds.valid_for(self, center))
    }
    /// Public template selection never exposes the private builder base.
    #[must_use]
    pub fn builder_revision(&self) -> Option<&sandproof_protocol::ResourceId> {
        self.private_builds
            .as_ref()
            .map(|builds| &builds.config.builder_revision)
    }
    #[must_use]
    pub fn private_build_config(&self) -> Option<&PrivateBuildConfig> {
        self.private_builds.as_ref().map(|builds| &builds.config)
    }
    #[must_use]
    pub fn artifact_role(&self, revision: &sandproof_protocol::ResourceId) -> NodeRole {
        if self.builder_revision() == Some(revision) {
            NodeRole::Builder
        } else {
            NodeRole::Runtime
        }
    }
}
