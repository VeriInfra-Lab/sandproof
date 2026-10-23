//! Original bounded-slot retirement, now bound to the one current operation row.
use crate::{CenterState, DecisionError, LanePhase, NodeMode, RetentionLane, Sandbox};
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    ClosedLifecycleWindow, ConfirmedLifecycleResult, LifecycleWindowClose, TerminalLifecycleObservation,
};

fn terminal(record: &Sandbox) -> Option<&TerminalLifecycleObservation> {
    let result = record.operations.last()?.terminal()?;
    (record.released() && record.terminal_confirmation.as_ref() == Some(result)).then_some(result)
}
impl CenterState {
    #[must_use]
    pub fn retention_lanes(&self) -> &[RetentionLane] { &self.retention_lanes }
    #[must_use]
    pub fn lane(&self, node: &NodeIdentity, slot: u16) -> Option<&RetentionLane> {
        self.retention_lanes.iter().find(|lane| lane.node == *node && lane.retention_slot == slot)
    }
    fn lane_index(&self, node: &NodeIdentity, slot: u16) -> Result<usize, DecisionError> {
        self.retention_lanes.iter().position(|lane| lane.node == *node && lane.retention_slot == slot)
            .ok_or(DecisionError::NotFound)
    }
    fn lane_record(&self, node: &NodeIdentity, slot: u16) -> Option<&Sandbox> {
        self.sandboxes.iter().find(|record| record.operations.first().is_some_and(|first|
            first.intent().request.binding.node == *node && first.intent().retention.slot == slot))
    }
    pub(crate) fn available_lane(&self, node: &NodeIdentity) -> Option<&RetentionLane> {
        self.retention_lanes.iter().find(|lane| lane.node == *node && lane.phase == LanePhase::Open
            && lane.current().is_some() && self.lane_record(node, lane.retention_slot).is_none())
    }
    #[must_use]
    pub fn retirement_candidates(&self, now_ms: u64) -> Vec<TerminalLifecycleObservation> {
        self.sandboxes.iter().filter_map(|record| {
            if !record.result_retention_elapsed(now_ms) { return None; }
            let result = terminal(record)?;
            let lane = self.lane(&result.request.request.binding.node, result.request.retention.slot)?;
            (lane.phase == LanePhase::Open).then(|| result.clone())
        }).collect()
    }
    pub(crate) fn valid_lanes(&self) -> bool {
        let expected = self.nodes.iter().map(|node| usize::from(node.retention_slots)).sum::<usize>();
        let mut identities = std::collections::BTreeSet::new();
        if self.retention_lanes.len() != expected { return false; }
        let mut index = 0;
        let mut valid = true;
        while valid && index < self.retention_lanes.len() {
            let lane = &self.retention_lanes[index];
            if let Some(node) = self.nodes.iter().find(|node| node.identity == lane.node) {
                valid = lane.retention_slot != 0 && lane.retention_slot <= node.retention_slots
                    && identities.insert((lane.node.node_id.as_str(), lane.retention_slot))
                    && (lane.phase == LanePhase::Exhausted) == lane.current().is_none()
                    && valid_last_retired(lane, self);
                if valid { valid = self.valid_lane_records(lane); }
            } else {
                valid = false;
            }
            // The guard established index < length, so this cannot wrap.
            index += 1;
        }
        valid
    }
    /// Separate the inner ordered scan for the fixed Aeneas loop prepass.
    /// Same records, first refusal, count and lane checks; no host effects.
    fn valid_lane_records(&self, lane: &RetentionLane) -> bool {
        let mut count = 0;
        let mut index = 0;
        let mut valid = true;
        while valid && index < self.sandboxes.len() {
            let record = &self.sandboxes[index];
            if let Some(first) = record.operations.first() {
                if first.intent().request.binding.node == lane.node && first.intent().retention.slot == lane.retention_slot {
                    count += 1;
                    valid = count <= 1 && lane.current().map(|w| w.get()) == Some(first.intent().retention.window.get())
                        && lane.phase != LanePhase::Exhausted
                        && (lane.phase != LanePhase::ClosePending || terminal(record).is_some());
                }
            } else {
                valid = false;
            }
            index += 1;
        }
        valid && (count != 0 || lane.phase != LanePhase::ClosePending)
    }
    /// # Errors
    /// Operational drain never retires history or releases resources.
    pub fn begin_drain(&self, node: &NodeIdentity) -> Result<Self, DecisionError> {
        if !self.is_valid() { return Err(DecisionError::Invalid); }
        let index = self.nodes.iter().position(|registered| registered.identity == *node).ok_or(DecisionError::NotFound)?;
        let mut next = self.clone();
        if next.nodes[index].mode == NodeMode::Accepting { next.nodes[index].mode = NodeMode::Draining; }
        Ok(next)
    }
    /// Host acknowledgement is checked against an ALREADY DURABLE center result.
    /// # Errors
    /// Echo, pending, superseded or uncommitted data cannot enable close.
    pub fn confirm_terminal(&self, acknowledgement: &ConfirmedLifecycleResult) -> Result<Self, DecisionError> {
        if !self.is_valid() { return Err(DecisionError::Invalid); }
        let result = &acknowledgement.result;
        let b = &result.request.request.binding;
        let index = self.index(&b.key.owner, b.session.as_protocol_id())?;
        let record = &self.sandboxes[index];
        if !acknowledgement.matches(result) || !record.released()
            || record.operations.last().and_then(|op| op.terminal()) != Some(result)
        { return Err(DecisionError::Stale); }
        let mut next = self.clone();
        next.sandboxes[index].terminal_confirmation = Some(result.clone());
        if !next.is_valid() { return Err(DecisionError::Invalid); }
        Ok(next)
    }
    /// # Errors
    /// Exact confirmed Cleaned and the original lane are mandatory.
    pub fn begin_close(&self, result: &TerminalLifecycleObservation, now_ms: u64)
        -> Result<(Self, LifecycleWindowClose), DecisionError>
    {
        if !self.is_valid() { return Err(DecisionError::Invalid); }
        let b = &result.request.request.binding;
        let index = self.index(&b.key.owner, b.session.as_protocol_id())?;
        if terminal(&self.sandboxes[index]) != Some(result) { return Err(DecisionError::Conflict); }
        if !self.sandboxes[index].result_retention_elapsed(now_ms) { return Err(DecisionError::Unavailable); }
        let lane_index = self.lane_index(&b.node, result.request.retention.slot)?;
        let lane = &self.retention_lanes[lane_index];
        if lane.current().map(|w| w.get()) != Some(result.request.retention.window.get()) || lane.phase == LanePhase::Exhausted {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        next.retention_lanes[lane_index].phase = LanePhase::ClosePending;
        if !next.is_valid() { return Err(DecisionError::Invalid); }
        Ok((next, LifecycleWindowClose { terminal: result.clone() }))
    }
    #[must_use]
    pub fn pending_closes(&self) -> Vec<LifecycleWindowClose> {
        self.retention_lanes.iter().filter(|lane| lane.phase == LanePhase::ClosePending)
            .filter_map(|lane| self.lane_record(&lane.node, lane.retention_slot).and_then(terminal))
            .cloned().map(|terminal| LifecycleWindowClose { terminal }).collect()
    }
    /// # Errors
    /// Exact persisted close, same terminal and same retired-through are required.
    /// Original metadata/native/material pins must also be vetoed by the business CAS.
    pub fn apply_close_ack(&self, ack: &ClosedLifecycleWindow, now_ms: u64) -> Result<Self, DecisionError> {
        if !self.is_valid() { return Err(DecisionError::Invalid); }
        let result = &ack.terminal;
        let b = &result.request.request.binding;
        let retention = result.request.retention;
        let lane_index = self.lane_index(&b.node, retention.slot)?;
        let lane = &self.retention_lanes[lane_index];
        let close = LifecycleWindowClose { terminal: result.clone() };
        if !ack.matches(&close) { return Err(DecisionError::Stale); }
        if retention.window.get() <= lane.retired_through {
            return if lane.last_retired.as_ref() == Some(result) { Ok(self.clone()) } else { Err(DecisionError::Stale) };
        }
        if lane.current().map(|w| w.get()) != Some(retention.window.get()) || lane.phase != LanePhase::ClosePending {
            return Err(DecisionError::Conflict);
        }
        let index = self.index(&b.key.owner, b.session.as_protocol_id())?;
        if terminal(&self.sandboxes[index]) != Some(result) { return Err(DecisionError::Stale); }
        if !self.sandboxes[index].result_retention_elapsed(now_ms) { return Err(DecisionError::Unavailable); }
        let mut next = self.clone();
        // Close the current public window BEFORE deleting any of its details.
        // Known records in older windows still replay before new-key admission.
        for window in &mut next.lifecycle.windows {
            if window.registration.owner == b.key.owner && window.registration.window.get() % 2 == 1
                && self.sandboxes[index].operations.iter().any(|op|
                    Some(op.intent().request.binding.key.window) == window.current)
            {
                window.current = window.current.and_then(|current| current.get().checked_add(2))
                    .and_then(|value| value.try_into().ok());
            }
        }
        next.sandboxes.remove(index);
        let lane = &mut next.retention_lanes[lane_index];
        lane.retired_through = retention.window.get();
        lane.last_retired = Some(result.clone());
        lane.phase = if lane.current().is_some() { LanePhase::Open } else { LanePhase::Exhausted };
        if !next.is_valid() { return Err(DecisionError::Invalid); }
        Ok(next)
    }
}
fn valid_last_retired(lane: &RetentionLane, state: &CenterState) -> bool {
    match (&lane.last_retired, lane.retired_through) {
        (None, 0) => true,
        (Some(result), floor) if floor > 0 => {
            let intent = &result.request;
            let b = &intent.request.binding;
            let close = LifecycleWindowClose { terminal: result.clone() };
            intent.retention.window.get() == floor && intent.retention.slot == lane.retention_slot
                && b.node == lane.node && intent.center == state.lifecycle.center
                && b.key.deployment == state.deployment && intent.request.validate(&lane.node).is_ok()
                && intent.accepted_version_for_request() == Some(intent.accepted_version)
                && crate::validation::number(b.session.as_protocol_id().as_str(), "sandbox-", state.sandbox_highwater).is_some()
                && state.tenant_is_registered(&b.key.owner)
                && state.lifecycle.windows.iter().any(|window| window.contains_history(&b.key))
                && (b.key.window.get() != 2 || b.key.sequence.get() <= state.operation_highwater)
                && close.is_terminal_result()
        }
        _ => false,
    }
}
