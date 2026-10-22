//! Recovery checks for the opt-in extension of the original authority state.
use super::{
    CenterState, ExpectedSession, LifecycleFact, LifecycleOperation, LifecycleOperationRecord,
    LifecycleProfile, LifecycleSession, LifecycleSubmission, OperationKey,
};

impl CenterState {
    pub(crate) fn lifecycle_registration_matches(&self, expected: &Self) -> bool {
        match (&self.lifecycle, &expected.lifecycle) {
            (None, None) => true,
            (Some(actual), Some(configured)) => {
                actual.format_version == configured.format_version
                    && actual.center == configured.center
                    && actual.windows == configured.windows
            }
            _ => false,
        }
    }

    pub(crate) fn valid_lifecycle(&self) -> bool {
        let Some(profile) = &self.lifecycle else {
            return true;
        };
        if profile.format_version != 1
            || profile.center.deployment != self.deployment
            || self.bounds.operations_per_sandbox < 3
            || self
                .bounds
                .required_block_bytes()
                .saturating_add(self.lifecycle_required_bytes())
                > 16 * 1024 * 1024
            || profile.windows.len() != self.tenants.len()
            || profile.sessions.len() != self.sandboxes.len()
        {
            return false;
        }
        self.valid_lifecycle_windows(profile) && self.valid_lifecycle_sessions(profile)
    }

    fn valid_lifecycle_windows(&self, profile: &LifecycleProfile) -> bool {
        for (index, window) in profile.windows.iter().enumerate() {
            if !self.tenant_is_registered(&window.owner)
                || profile.windows[..index]
                    .iter()
                    .any(|w| w.owner == window.owner)
            {
                return false;
            }
        }
        true
    }

    fn valid_lifecycle_sessions(&self, profile: &LifecycleProfile) -> bool {
        let mut keys = Vec::new();
        for (index, session) in profile.sessions.iter().enumerate() {
            if profile.sessions[..index]
                .iter()
                .any(|s| s.original_sandbox == session.original_sandbox)
                || !self.valid_lifecycle_session(profile, session, &mut keys)
            {
                return false;
            }
        }
        true
    }

    fn valid_lifecycle_session<'a>(
        &self,
        profile: &LifecycleProfile,
        session: &'a LifecycleSession,
        keys: &mut Vec<&'a OperationKey>,
    ) -> bool {
        let Some(first) = session.operations.first() else {
            return false;
        };
        let owner = &first.intent.request.binding.key.owner;
        let Ok(index) = self.index(owner, &session.original_sandbox) else {
            return false;
        };
        let original = &self.sandboxes[index];
        let origin = &original.operations[0].request;
        let Some(last) = session.operations.last() else {
            return false;
        };
        let terminating = matches!(
            last.intent.request.operation,
            LifecycleOperation::Destroy { .. }
        );
        if original.released
            || original.terminating != terminating
            || original.operations.len() != 1 + usize::from(terminating)
            || session.operations.len() > usize::from(self.bounds.operations_per_sandbox)
            || (!terminating
                && session.operations.len() >= usize::from(self.bounds.operations_per_sandbox))
        {
            return false;
        }
        let mut prior_version = 0u64;
        for (position, operation) in session.operations.iter().enumerate() {
            let intent = &operation.intent;
            let binding = &intent.request.binding;
            if intent.center != profile.center
                || binding.key != *operation.submission.key()
                || binding.key.owner != *owner
                || binding.key.deployment != self.deployment
                || binding.node != origin.identity.node
                || binding.session.as_protocol_id() != &session.original_sandbox
                || keys.contains(&&binding.key)
                || !profile
                    .windows
                    .iter()
                    .any(|w| w.owner == *owner && w.window == binding.key.window)
                || intent.request.validate(&origin.identity.node).is_err()
                || intent.accepted_version_for_request() != Some(intent.accepted_version)
                || Some(intent.accepted_version) != prior_version.checked_add(1)
                || !Self::valid_lifecycle_operation(session, operation, position, origin, original)
            {
                return false;
            }
            keys.push(&binding.key);
            prior_version = intent.accepted_version;
        }
        true
    }

    fn valid_lifecycle_operation(
        session: &LifecycleSession,
        operation: &LifecycleOperationRecord,
        position: usize,
        origin: &sandproof_protocol::internal::lifecycle::LifecycleRequest,
        original: &crate::Sandbox,
    ) -> bool {
        let request = &operation.intent.request;
        match (&operation.submission, &request.operation) {
            (
                LifecycleSubmission::Create { parameters, .. },
                LifecycleOperation::Create {
                    create,
                    target_incarnation,
                },
            ) => {
                position == 0
                    && request.binding.expected == ExpectedSession::Absent
                    && target_incarnation.get() == 1
                    && create == &origin.create
                    && parameters.template_revision == create.template_revision
                    && parameters.template_digest == create.template_digest
                    && parameters.resources == create.resources
                    && parameters.options == create.options
                    && parameters.startup_timeout_ms > 0
                    && session
                        .created_at_ms
                        .checked_add(parameters.startup_timeout_ms)
                        == Some(create.startup_deadline_ms)
                    && parameters.timeout_ms == create.timeout_ms
                    && parameters.max_lifetime_ms == create.max_lifetime_ms
                    && operation.ready == original.operations[0].response
                    && operation
                        .ready
                        .as_ref()
                        .is_none_or(|r| matches!(r.fact, LifecycleFact::Ready { .. }))
            }
            (
                LifecycleSubmission::Unload {
                    session: named,
                    expected_version,
                    ..
                },
                LifecycleOperation::Unload {
                    source_incarnation,
                    capture,
                },
            ) => {
                position == 1
                    && *expected_version == 1
                    && source_incarnation.get() == 1
                    && capture.get() == 1
                    && named == &request.binding.session
                    && request.binding.expected == ExpectedSession::Version(*expected_version)
                    && session.operations[0].ready.is_some()
                    && operation.ready.is_none()
            }
            (
                LifecycleSubmission::Destroy {
                    session: named,
                    expected_version,
                    ..
                },
                LifecycleOperation::Destroy {},
            ) => {
                position > 0
                    && position + 1 == session.operations.len()
                    && *expected_version == position as u64
                    && named == &request.binding.session
                    && request.binding.expected == ExpectedSession::Version(*expected_version)
                    && operation.ready.is_none()
            }
            _ => false,
        }
    }
}
