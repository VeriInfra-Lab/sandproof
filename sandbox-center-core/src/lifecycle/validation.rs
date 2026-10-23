//! Complete replacement for BOTH old history and shadow-table validation.
//! Preserve canonical identity, original input, bounds, ordering, lifetime,
//! exact terminal confirmation and lane checks against one authoritative row.
use super::{
    CenterState, ExpectedSession, LifecycleOperation, LifecycleOperationRecord, LifecycleProfile,
    LifecycleSubmission, OperationKey, Sandbox,
};
use sandproof_protocol::lifecycle::TerminalStatus;
use std::collections::BTreeSet;

impl CenterState {
    pub(crate) fn lifecycle_registration_matches(&self, expected: &Self) -> bool {
        self.lifecycle.format_version == expected.lifecycle.format_version
            && self.lifecycle.center == expected.lifecycle.center
            && self.lifecycle.windows.len() == expected.lifecycle.windows.len()
            && self
                .lifecycle
                .windows
                .iter()
                .zip(&expected.lifecycle.windows)
                .all(|(actual, configured)| actual.registration == configured.registration)
    }
    pub(crate) fn valid_lifecycle(&self) -> bool {
        let p = &self.lifecycle;
        if p.format_version != super::LIFECYCLE_PROFILE_VERSION
            || p.center.deployment != self.deployment
            || p.windows.len() != self.tenants.len().saturating_mul(2)
            || !self.valid_lifecycle_windows(p)
        {
            return false;
        }
        let mut sessions = BTreeSet::new();
        let mut allocations = BTreeSet::new();
        let mut keys = Vec::new();
        let mut index = 0;
        let mut valid = true;
        while valid && index < self.retention_lanes.len() {
            if let Some(result) = self.retention_lanes[index].last_retired.as_ref() {
                let b = &result.request.request.binding;
                valid =
                    sessions.insert(b.session.as_protocol_id().as_str()) && !keys.contains(&&b.key);
                if valid {
                    keys.push(&b.key);
                }
            }
            index += 1;
        }
        index = 0;
        while valid && index < self.sandboxes.len() {
            let session = &self.sandboxes[index];
            if let Some(first) = session.operations.first() {
                let id = first
                    .intent
                    .request
                    .binding
                    .session
                    .as_protocol_id()
                    .as_str();
                valid = sessions.insert(id)
                    && crate::validation::number(id, "sandbox-", self.sandbox_highwater).is_some()
                    && self.valid_lifecycle_session(session, &mut allocations, &mut keys);
            } else {
                valid = false;
            }
            index += 1;
        }
        valid
    }
    fn valid_lifecycle_windows(&self, p: &LifecycleProfile) -> bool {
        let mut index = 0;
        let mut valid = true;
        while valid && index < p.windows.len() {
            let window = &p.windows[index];
            let initial = &window.registration;
            valid = !(!self.tenant_is_registered(&initial.owner)
                || p.windows[..index].iter().any(|w| {
                    w.registration.owner == initial.owner
                        && (w.registration.window.get() == 2) == (initial.window.get() == 2)
                })
                || if initial.window.get() == 2 {
                    window.current != Some(initial.window)
                } else {
                    initial.window.get() % 2 != 1
                        || window.current.is_some_and(|current| {
                            current.get() % 2 != 1 || current.get() < initial.window.get()
                        })
                });
            index += 1;
        }
        index = 0;
        while valid && index < self.tenants.len() {
            let tenant = &self.tenants[index];
            valid = !(p
                .windows
                .iter()
                .filter(|w| {
                    w.registration.owner == tenant.tenant && w.registration.window.get() % 2 == 1
                })
                .count()
                != 1
                || p.windows
                    .iter()
                    .filter(|w| {
                        w.registration.owner == tenant.tenant && w.registration.window.get() == 2
                    })
                    .count()
                    != 1);
            index += 1;
        }
        valid
    }
    fn valid_lifecycle_session<'a>(
        &self,
        session: &'a Sandbox,
        allocations: &mut BTreeSet<u64>,
        keys: &mut Vec<&'a OperationKey>,
    ) -> bool {
        let Some(first) = session.operations.first() else {
            return false;
        };
        let Some(last) = session.operations.last() else {
            return false;
        };
        let origin = &first.intent;
        let b = &origin.request.binding;
        let Some(lane) = self.lane(&b.node, origin.retention.slot) else {
            return false;
        };
        if !origin.retention.valid_for(self.bounds.max_sandboxes)
            || session.operations.len() > usize::from(self.bounds.operations_per_sandbox)
            || (!session.terminating()
                && session.operations.len() >= usize::from(self.bounds.operations_per_sandbox))
            || lane
                .current()
                .map(sandproof_protocol::internal::PositiveCounter::get)
                != Some(origin.retention.window.get())
            || !self.tenant_is_registered(&b.key.owner)
            || !self.nodes.iter().any(|node| node.identity == b.node)
            || (first.create_refusal.is_some()
                && (session.operations.len() != 2
                    || !matches!(
                        last.intent.request.operation,
                        LifecycleOperation::Destroy {}
                    )
                    || session
                        .operations
                        .iter()
                        .any(|operation| operation.running.is_some())))
            || session
                .terminal_confirmation
                .as_ref()
                .is_some_and(|confirmed| !session.released() || last.terminal() != Some(confirmed))
        {
            return false;
        }
        let mut previous_allocation = 0;
        let mut position = 0;
        let mut valid = true;
        while valid && position < session.operations.len() {
            let op = &session.operations[position];
            let intent = &op.intent;
            let actual = &intent.request.binding;
            valid = !(intent.center != self.lifecycle.center
                || actual.key != *op.submission.key()
                || actual.key.deployment != self.deployment
                || actual.key.owner != b.key.owner
                || actual.node != b.node
                || actual.session != b.session
                || intent.retention != origin.retention
                || keys.contains(&&actual.key)
                || !self
                    .lifecycle
                    .windows
                    .iter()
                    .any(|window| window.contains_history(&actual.key))
                || intent.request.validate(&b.node).is_err()
                || intent.accepted_version_for_request() != Some(intent.accepted_version)
                || intent.accepted_version != position as u64 + 1
                || op.allocation_number <= previous_allocation
                || op.allocation_number > self.operation_highwater
                || !allocations.insert(op.allocation_number)
                || (actual.key.window.get() == 2
                    && actual.key.sequence.get() != op.allocation_number)
                || op.terminal.as_ref().is_some_and(|retained| {
                    retained.result.status != TerminalStatus::Cleaned
                        || !retained.result.matches_request(intent)
                        || match retained.retention_started_at_ms {
                            Some(started) => {
                                started < retained.recorded_at_ms
                                    || !session.acceptance_times_not_after(started)
                                    || started.checked_add(self.bounds.result_retention_ms)
                                        != retained.retain_until_ms
                            }
                            None => {
                                retained.retain_until_ms.is_some()
                                    || session.acceptance_times_not_after(retained.recorded_at_ms)
                            }
                        }
                })
                || !self.valid_lifecycle_operation(session, op, position));
            if valid {
                keys.push(&actual.key);
                previous_allocation = op.allocation_number;
            }
            position += 1;
        }
        valid
    }
    fn valid_lifecycle_operation(
        &self,
        session: &Sandbox,
        op: &LifecycleOperationRecord,
        position: usize,
    ) -> bool {
        let intent = &op.intent;
        let request = &intent.request;
        if position != 0 && op.create_refusal.is_some() {
            return false;
        }
        let Some(origin) = session.operations.first() else {
            return false;
        };
        let previous = match position.checked_sub(1) {
            Some(index) => session.operations.get(index),
            None => None,
        };
        let Some(previous) = previous else {
            return self.valid_lifecycle_create_record(op, position);
        };
        if request.binding.expected != ExpectedSession::Version(previous.intent.accepted_version) {
            return false;
        }
        Self::valid_lifecycle_followup(op, origin, previous, position + 1 == session.operations.len())
    }

    fn valid_lifecycle_followup(
        op: &LifecycleOperationRecord,
        origin: &LifecycleOperationRecord,
        previous: &LifecycleOperationRecord,
        is_last: bool,
    ) -> bool {
        let intent = &op.intent;
        let request = &intent.request;
        match (&op.submission, &request.operation) {
            (
                LifecycleSubmission::Renew {
                    session: named,
                    expected_version,
                    timeout_secs,
                    extend_only,
                    ..
                },
                LifecycleOperation::Renew {
                    incarnation,
                    end_at_ms,
                    extend_only: actual_extend,
                },
            ) => {
                let Some(prior) = previous.running.as_ref() else {
                    return false;
                };
                let Some(accepted_at_ms) = op.acceptance_time.sampled_at_ms() else { return false; };
                let Ok(end) = u64::try_from(*end_at_ms) else {
                    return false;
                };
                let target = if *extend_only {
                    prior.end_at_ms.max(end)
                } else {
                    end
                };
                *expected_version == previous.intent.accepted_version
                    && named == &request.binding.session
                    && *extend_only == *actual_extend
                    && incarnation.get() == 1
                    && *timeout_secs > 0
                    && super::normalized_renewal_end(
                        accepted_at_ms,
                        *timeout_secs,
                        prior.max_end_at_ms,
                    ) == end
                    && prior.end_at_ms > accepted_at_ms
                    && prior.max_end_at_ms > accepted_at_ms
                    && op.terminal.is_none()
                    && op.running.as_ref().is_none_or(|facts| {
                        facts.matches_renewed(&origin.intent, intent)
                            && facts.started_at_ms == prior.started_at_ms
                            && facts.max_end_at_ms == prior.max_end_at_ms
                            && facts.startup_deadline_ms == prior.startup_deadline_ms
                            && facts.end_at_ms == target
                    })
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
                *expected_version == previous.intent.accepted_version
                    && named == &request.binding.session
                    && previous.running.is_some()
                    && source_incarnation.get() == 1
                    && capture.get() == 1
                    && op.acceptance_time.sampled_at_ms().is_some()
                    && op.running.is_none()
                    && op.terminal.is_none()
            }
            (
                LifecycleSubmission::Destroy {
                    session: named,
                    expected_version,
                    ..
                },
                LifecycleOperation::Destroy {},
            ) => {
                let super::AcceptanceTime::Unsampled = op.acceptance_time else { return false; };
                is_last
                    && *expected_version == previous.intent.accepted_version
                    && named == &request.binding.session
                    && !matches!(
                        previous.intent.request.operation,
                        LifecycleOperation::Destroy {}
                    )
                    && op.running.is_none()
            }
            _ => false,
        }
    }

    /// The same initial-record branch, kept separate from follow-up checks.
    fn valid_lifecycle_create_record(
        &self,
        op: &LifecycleOperationRecord,
        position: usize,
    ) -> bool {
        let intent = &op.intent;
        let request = &intent.request;
        let Some(accepted_at_ms) = op.acceptance_time.sampled_at_ms() else { return false; };
        match (&op.submission, &request.operation) {
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
                    && create.is_valid()
                    && create.options.len() <= self.bounds.max_options_bytes as usize
                    && parameters.template_revision == create.template_revision
                    && parameters.template_digest == create.template_digest
                    && parameters.resources == create.resources
                    && parameters.options == create.options
                    && parameters.startup_timeout_ms > 0
                    && accepted_at_ms.checked_add(parameters.startup_timeout_ms)
                        == Some(create.startup_deadline_ms)
                    && parameters.timeout_ms == create.timeout_ms
                    && parameters.max_lifetime_ms == create.max_lifetime_ms
                    && op.terminal.is_none()
                    && op
                        .running
                        .as_ref()
                        .is_none_or(|facts| facts.matches_create(intent))
            }
            _ => false,
        }
    }
}
