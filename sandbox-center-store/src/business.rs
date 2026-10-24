//! One Redis block for coupled lifecycle and engineering business metadata.
//! This module adds no key, queue, storage engine or recovery algorithm.
use crate::{ControlState, StoreError, authority::AuthorityState};
use sandproof_center_core::CenterState;
use serde::{Deserialize, Serialize};

/// Metadata adapters validate cross-references against the same center state
/// and reserve the final bounded record shape, not just today's JSON length.
pub trait BusinessMetadata: Clone + Serialize + serde::de::DeserializeOwned {
    /// Pure same-CAS retirement projection after the original core transition.
    /// It must not erase unknown responsibilities merely because a row is absent.
    #[must_use]
    fn with_center(&self, center: &CenterState) -> Self;
    fn is_valid_for(&self, center: &CenterState) -> bool;
    fn required_block_bytes(&self) -> usize;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(
        serialize = "M: Serialize",
        deserialize = "M: serde::de::DeserializeOwned"
    )
)]
pub struct BusinessEnvelope<M> {
    pub center: CenterState,
    pub metadata: M,
}

impl<M: BusinessMetadata> ControlState for BusinessEnvelope<M> {
    // No deployment migration: previous center-only blocks are rejected.
    const FORMAT_VERSION: u32 = sandproof_protocol::internal::CENTER_STORE_FORMAT_VERSION;
    fn check_supported_profiles(&self) -> Result<(), StoreError> {
        self.center.check_supported_profiles()
    }
    fn is_valid(&self) -> bool {
        self.center.is_valid()
            && self.metadata.is_valid_for(&self.center)
            && self.required_block_bytes() != usize::MAX
    }
}
impl<M: BusinessMetadata> AuthorityState for BusinessEnvelope<M> {
    fn center(&self) -> &CenterState {
        &self.center
    }
    fn with_center(&self, center: CenterState) -> Self {
        let metadata = self.metadata.with_center(&center);
        Self { center, metadata }
    }
    fn required_block_bytes(&self) -> usize {
        self.center
            .required_authority_bytes()
            .saturating_add(self.metadata.required_block_bytes())
    }
}
