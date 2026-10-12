//! Original protected candidate verification/sealing plus exact cache facts.
use sandproof_api::PortError;
use sandproof_center_service::{
    RegisteredHosts,
    business::{CandidatePort, PreparationError, ProtectedCandidate, SharedBusinessAuthority},
};
use sandproof_controller_service::{template_registry::CandidateStore, templates::TemplateBinding};
use sandproof_internal::cache::CacheRequest;
use sandproof_protocol::{ResourceId, TenantId};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct ProtectedCandidates {
    authority: SharedBusinessAuthority,
    hosts: RegisteredHosts,
    store: Arc<CandidateStore>,
    /// Scheduling hints only: one bounded cursor per currently registered
    /// immutable artifact, shared by clones. Never an availability authority.
    cache_starts: Arc<Mutex<Vec<CacheStart>>>,
}
struct CacheStart {
    revision: ResourceId,
    digest: [u8; 32],
    next_host: usize,
}

fn rotate_start(
    cursors: &mut Vec<CacheStart>,
    revision: &ResourceId,
    digest: [u8; 32],
    nodes: std::num::NonZeroUsize,
) -> usize {
    let index = cursors
        .iter()
        .position(|cursor| cursor.revision == *revision && cursor.digest == digest)
        .unwrap_or_else(|| {
            cursors.push(CacheStart {
                revision: revision.clone(),
                digest,
                next_host: 0,
            });
            cursors.len() - 1
        });
    let cursor = &mut cursors[index];
    let start = cursor.next_host % nodes.get();
    cursor.next_host = (start + 1) % nodes.get();
    start
}
impl ProtectedCandidates {
    pub(super) fn new(
        authority: SharedBusinessAuthority,
        hosts: RegisteredHosts,
        store: Arc<CandidateStore>,
    ) -> Self {
        Self {
            authority,
            hosts,
            store,
            cache_starts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// This finite background work is derived from the SAME protected catalog;
    /// it is not a queue, synthetic availability or alternate default template.
    /// Individual unavailable artifacts/hosts are retried later, while shared
    /// authority faults stop the whole scan. Running budgets remain owned by
    /// the host; a failed material check never creates a cache fact.
    pub fn prepare_static(&self, deadline: Duration) -> Result<(), PortError> {
        let original = self.authority.snapshot()?;
        let templates = original.metadata.static_artifacts()?;
        for (revision, binding) in templates {
            let digest = binding
                .canonical_digest()
                .map_err(|_| PortError::OutcomeUnknown)?;
            if original
                .center
                .nodes()
                .iter()
                .filter(|node| node.role == original.metadata.artifact_role(&revision))
                .all(|node| {
                    node.templates
                        .iter()
                        .any(|known| known.revision == revision && known.digest == digest)
                })
            {
                continue;
            }
            if self.store.verify_binding(&binding).is_ok() {
                let _ = self.cache(&revision, &binding, deadline);
            }
            // A local material failure must not block another protected
            // revision, but it must not hide a simultaneous shared-store fault.
            self.authority.snapshot()?;
        }
        Ok(())
    }

    fn cache(
        &self,
        revision: &ResourceId,
        binding: &TemplateBinding,
        deadline: Duration,
    ) -> Result<(), PortError> {
        let until = Instant::now()
            .checked_add(deadline)
            .ok_or(PortError::Invalid)?;
        let current = self.authority.snapshot()?;
        let digest = binding.canonical_digest().map_err(|_| PortError::Invalid)?;
        if current.metadata.artifact_binding(revision, &digest)? != *binding {
            return Err(PortError::Conflict);
        }
        let nodes = current.center.nodes();
        let node_count =
            std::num::NonZeroUsize::new(nodes.len()).ok_or(PortError::OutcomeUnknown)?;
        let start = {
            let mut cursors = self
                .cache_starts
                .lock()
                .map_err(|_| PortError::OutcomeUnknown)?;
            // Registry/catalog limits bound these hints. Retired artifacts do not
            // leave an ever-growing history, and new revisions start afresh.
            cursors.retain(|cursor| {
                current
                    .metadata
                    .artifact_binding(&cursor.revision, &cursor.digest)
                    .is_ok()
            });
            rotate_start(&mut cursors, revision, digest, node_count)
        };
        let mut available = false;
        // Keep the original total deadline and serial transport. Even if the
        // first RPC consumes it, this artifact starts at another node next
        // time; independent artifacts cannot reset each other's progress.
        for offset in 0..nodes.len() {
            let node = &nodes[(start + offset) % nodes.len()];
            if node.role != current.metadata.artifact_role(revision) {
                continue;
            }
            if node
                .templates
                .iter()
                .any(|known| known.revision == *revision && known.digest == digest)
            {
                available = true;
                continue;
            }
            let Some(remaining) = until.checked_duration_since(Instant::now()) else {
                break;
            };
            let request = CacheRequest {
                node: node.identity.clone(),
                revision: revision.clone(),
                digest,
                descriptor: binding.clone(),
            };
            match self.hosts.cache_template(&request, remaining) {
                Ok(evidence) => {
                    self.authority.register_cache(&request, &evidence)?;
                    available = true;
                }
                Err(_) => {
                    self.authority.snapshot()?;
                }
            }
        }
        if available {
            Ok(())
        } else {
            Err(PortError::OutcomeUnknown)
        }
    }
}

impl CandidatePort for ProtectedCandidates {
    fn verified(
        &self,
        tenant: &TenantId,
        candidate: &str,
    ) -> Result<ProtectedCandidate, PortError> {
        self.authority.snapshot()?;
        let owner = tenant.as_str().parse().map_err(|_| PortError::Invalid)?;
        let (publication, _) = self
            .store
            .verified_candidate(owner, candidate)
            .map_err(|_| PortError::Invalid)?;
        let revision = format!("candidate-{candidate}-r1")
            .try_into()
            .map_err(|_| PortError::Invalid)?;
        ProtectedCandidate::new(publication, revision)
    }
    fn prepare(
        &self,
        candidate: &ProtectedCandidate,
        deadline: Duration,
    ) -> Result<(), PreparationError> {
        let current = self
            .authority
            .snapshot()
            .map_err(|_| PreparationError::Unknown)?;
        let digest = candidate
            .publication
            .binding
            .canonical_digest()
            .map_err(|_| PreparationError::Rejected)?;
        if current
            .metadata
            .artifact_binding(&candidate.revision, &digest)
            .map_err(|_| PreparationError::Unknown)?
            != candidate.publication.binding
        {
            return Err(PreparationError::Unknown);
        }
        self.store
            .prepare_sealed(&candidate.publication)
            .map_err(|_| PreparationError::Unknown)?;
        self.cache(
            &candidate.revision,
            &candidate.publication.binding,
            deadline,
        )
        .map_err(|_| PreparationError::Unknown)
    }
}

#[cfg(test)]
#[path = "tests/cursors.rs"]
mod cursor_tests;
