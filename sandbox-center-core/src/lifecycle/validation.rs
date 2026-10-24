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
                valid = sessions.insert(b.session.as_protocol_id().as_str().to_owned())
                    && !keys.contains(&b.key);
                if valid {
                    keys.push(b.key.clone());
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
                valid = sessions.insert(id.to_owned())
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
            valid = self.valid_lifecycle_window(p, index);
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
    fn valid_lifecycle_window(&self, p: &LifecycleProfile, index: usize) -> bool {
        let window = &p.windows[index];
        let initial = &window.registration;
        if !self.tenant_is_registered(&initial.owner)
            || p.windows[..index].iter().any(|w| {
                w.registration.owner == initial.owner
                    && (w.registration.window.get() == 2) == (initial.window.get() == 2)
            })
        {
            return false;
        }
        if initial.window.get() == 2 {
            return window.current == Some(initial.window);
        }
        initial.window.get() % 2 == 1
            && window.current.is_none_or(|current| {
                current.get() % 2 == 1 && current.get() >= initial.window.get()
            })
    }
    fn valid_lifecycle_session(
        &self,
        session: &Sandbox,
        allocations: &mut BTreeSet<u64>,
        keys: &mut Vec<OperationKey>,
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
            || !session.valid_renewal_checkpoint()
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
                || keys.contains(&actual.key)
                || !self
                    .lifecycle
                    .windows
                    .iter()
                    .any(|window| window.contains_history(&actual.key))
                || intent.request.validate(&b.node).is_err()
                || intent.accepted_version_for_request() != Some(intent.accepted_version)
                || (position == 0 && intent.accepted_version != 1)
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
                keys.push(actual.key.clone());
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
        if op.resume_state_lost && !super::resume::lost_origin_valid(&session.operations[..=position]) {
            return false;
        }
        if let Some(observation) = op.unloaded.as_ref() {
            let Some(facts) = observation.unloaded.as_ref() else {
                return false;
            };
            let Some(prior) = position
                .checked_sub(1)
                .and_then(|i| session.operations.get(i))
                .and_then(|previous| previous.running.as_ref())
            else {
                return false;
            };
            if !observation.matches_request(&op.intent)
                || facts.started_at_ms != prior.started_at_ms
                || facts.end_at_ms != prior.end_at_ms
                || facts.max_end_at_ms != prior.max_end_at_ms
                || facts.startup_deadline_ms != prior.startup_deadline_ms
            {
                return false;
            }
        }
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
        if !session.valid_version_link(previous, op) {
            return false;
        }
        Self::valid_lifecycle_followup(
            op,
            origin,
            previous,
            position + 1 == session.operations.len(),
            &session.operations[..position],
        )
    }

    fn valid_lifecycle_followup(
        op: &LifecycleOperationRecord,
        origin: &LifecycleOperationRecord,
        previous: &LifecycleOperationRecord,
        is_last: bool,
        history: &[LifecycleOperationRecord],
    ) -> bool {
        let intent = &op.intent;
        let request = &intent.request;
        let LifecycleSubmission::Renew {
            session: named,
            expected_version,
            timeout_secs,
            extend_only,
            ..
        } = &op.submission
        else {
            return Self::valid_lifecycle_nonrenewal(op, previous, is_last, history);
        };
        let LifecycleOperation::Renew {
            incarnation,
            end_at_ms,
            extend_only: actual_extend,
        } = &request.operation
        else {
            return false;
        };
        let Some(prior) = previous.running.as_ref() else {
            return false;
        };
        let Some(previous_end) = op.previous_effective_end_at_ms else {
            return false;
        };
        let Some(accepted_at_ms) = op.acceptance_time.sampled_at_ms() else {
            return false;
        };
        let Ok(end) = u64::try_from(*end_at_ms) else {
            return false;
        };
        let target = if *extend_only {
            previous_end.max(end)
        } else {
            end
        };
        request.binding.expected == ExpectedSession::Version(*expected_version)
            && named == &request.binding.session
            && *extend_only == *actual_extend
            && Some(*incarnation)
                == super::resume::running_incarnation(&previous.intent.request.operation)
            && *timeout_secs > 0
            && super::normalized_renewal_end(accepted_at_ms, *timeout_secs, prior.max_end_at_ms)
                == end
            && previous_end > accepted_at_ms
            && previous_end > prior.started_at_ms
            && previous_end <= prior.max_end_at_ms
            && (*expected_version != previous.intent.accepted_version
                || previous_end == prior.end_at_ms)
            && prior.max_end_at_ms > accepted_at_ms
            && op.terminal.is_none()
            && op.running.as_ref().is_none_or(|facts| {
                super::resume::renewed_facts_match(history, &origin.intent, intent, facts)
                    && facts.started_at_ms == prior.started_at_ms
                    && facts.max_end_at_ms == prior.max_end_at_ms
                    && facts.startup_deadline_ms == prior.startup_deadline_ms
                    && facts.end_at_ms == target
            })
    }

    fn valid_lifecycle_nonrenewal(
        op: &LifecycleOperationRecord,
        previous: &LifecycleOperationRecord,
        is_last: bool,
        history: &[LifecycleOperationRecord],
    ) -> bool {
        let request = &op.intent.request;
        match &op.submission {
            LifecycleSubmission::Unload {
                session: named,
                expected_version,
                ..
            } => {
                let LifecycleOperation::Unload {
                    source_incarnation,
                    capture,
                } = &request.operation
                else {
                    return false;
                };
                request.binding.expected == ExpectedSession::Version(*expected_version)
                    && named == &request.binding.session
                    && previous.running.is_some()
                    && op.previous_effective_end_at_ms.is_none()
                    && Some(*source_incarnation)
                        == super::resume::running_incarnation(&previous.intent.request.operation)
                    && super::resume::next_capture(history) == Ok(*capture)
                    && op.acceptance_time.sampled_at_ms().is_some()
                    && op.running.is_none()
                    && op.terminal.is_none()
            }
            LifecycleSubmission::Resume {
                session: named,
                expected_version,
                ..
            } => {
                let Some(source) = previous.unloaded.as_ref().and_then(|o| o.unloaded.as_ref())
                else {
                    return false;
                };
                let LifecycleOperation::Resume {
                    source_incarnation,
                    capture,
                    target_incarnation,
                    manifest_digest,
                } = request.operation
                else {
                    return false;
                };
                request.binding.expected == ExpectedSession::Version(*expected_version)
                    && named == &request.binding.session
                    && source_incarnation == source.source_incarnation
                    && capture == source.capture
                    && source.source_incarnation.checked_next() == Ok(target_incarnation)
                    && manifest_digest == source.manifest_digest
                    && op.acceptance_time.sampled_at_ms().is_some_and(|time| {
                        time < source.end_at_ms
                            && time < source.max_end_at_ms
                            && previous
                                .acceptance_time
                                .sampled_at_ms()
                                .is_some_and(|old| time >= old)
                    })
                    && op.previous_effective_end_at_ms.is_none()
                    && op.unloaded.is_none()
                    && op.terminal.is_none()
                    && op
                        .running
                        .as_ref()
                        .is_none_or(|facts| facts.matches_resumed(&op.intent, source))
            }
            LifecycleSubmission::Destroy {
                session: named,
                expected_version,
                ..
            } => {
                if !matches!(request.operation, LifecycleOperation::Destroy {}) {
                    return false;
                }
                let super::AcceptanceTime::Unsampled = op.acceptance_time else {
                    return false;
                };
                is_last
                    && request.binding.expected == ExpectedSession::Version(*expected_version)
                    && named == &request.binding.session
                    && op.previous_effective_end_at_ms.is_none()
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
        let Some(accepted_at_ms) = op.acceptance_time.sampled_at_ms() else {
            return false;
        };
        let LifecycleSubmission::Create { parameters, .. } = &op.submission else {
            return false;
        };
        let LifecycleOperation::Create {
            create,
            target_incarnation,
        } = &request.operation
        else {
            return false;
        };
        position == 0
            && op.previous_effective_end_at_ms.is_none()
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
}
