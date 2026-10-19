//! Pure bounded-slot retirement. No network, TTL, host sweep or second queue.
//! The adapter commits intent and vetoes its original business pins before close.
use crate::{CenterState, DecisionError, LanePhase, NodeMode, RetentionLane, Sandbox};
use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::{LifecycleFact, LifecycleResponse},
    window::{WindowCloseAck, WindowCloseRequest},
};
use std::collections::BTreeSet;

fn terminal(record: &Sandbox) -> Option<&LifecycleResponse> {
    let response = record.operations.last()?.response.as_ref()?;
    (record.released
        && matches!(
            response.fact,
            LifecycleFact::Cleaned | LifecycleFact::NotAccepted
        )
        && record.terminal_confirmation.as_ref() == Some(response))
    .then_some(response)
}

impl CenterState {
    #[must_use]
    pub fn retention_lanes(&self) -> &[RetentionLane] {
        &self.retention_lanes
    }

    #[must_use]
    pub fn lane(&self, node: &NodeIdentity, slot: u16) -> Option<&RetentionLane> {
        let index = self
            .retention_lanes
            .iter()
            .position(|lane| &lane.node == node && lane.retention_slot == slot)?;
        self.retention_lanes.get(index)
    }

    fn lane_index(&self, node: &NodeIdentity, slot: u16) -> Result<usize, DecisionError> {
        self.retention_lanes
            .iter()
            .position(|lane| &lane.node == node && lane.retention_slot == slot)
            .ok_or(DecisionError::NotFound)
    }

    fn lane_record(&self, node: &NodeIdentity, slot: u16) -> Option<&Sandbox> {
        let index = self.sandboxes.iter().position(|record| {
            record.operations.first().is_some_and(|first| {
                &first.request.identity.node == node
                    && first.request.identity.retention_slot == slot
            })
        })?;
        self.sandboxes.get(index)
    }

    pub(crate) fn available_lane(&self, node: &NodeIdentity) -> Option<&RetentionLane> {
        let index = self.retention_lanes.iter().position(|lane| {
            &lane.node == node
                && lane.phase == LanePhase::Open
                && lane.current().is_some()
                && self.lane_record(node, lane.retention_slot).is_none()
        })?;
        self.retention_lanes.get(index)
    }

    /// Empty slots do not rotate, and a long-lived peer never blocks this candidate.
    #[must_use]
    pub fn retirement_candidates(&self) -> Vec<LifecycleResponse> {
        self.sandboxes
            .iter()
            .filter_map(|record| {
                let result = terminal(record)?;
                let lane = self.lane(&result.identity.node, result.identity.retention_slot)?;
                (lane.phase == LanePhase::Open).then(|| result.clone())
            })
            .collect()
    }

    pub(crate) fn valid_lanes(&self) -> bool {
        let expected = self
            .nodes
            .iter()
            .map(|node| usize::from(node.retention_slots))
            .sum::<usize>();
        let mut identities = BTreeSet::new();
        self.retention_lanes.len() == expected
            && self.retention_lanes.iter().all(|lane| {
                let Some(index) = self
                    .nodes
                    .iter()
                    .position(|node| node.identity == lane.node)
                else {
                    return false;
                };
                let node = &self.nodes[index];
                if lane.retention_slot == 0
                    || lane.retention_slot > node.retention_slots
                    || !identities.insert((lane.node.node_id.as_str(), lane.retention_slot))
                    || (lane.phase == LanePhase::Exhausted) != lane.current().is_none()
                    || !valid_last_retired(lane, self)
                {
                    return false;
                }
                let records = self
                    .sandboxes
                    .iter()
                    .filter(|record| {
                        record.operations.first().is_some_and(|first| {
                            first.request.identity.node == lane.node
                                && first.request.identity.retention_slot == lane.retention_slot
                        })
                    })
                    .collect::<Vec<_>>();
                if records.len() > 1 {
                    return false;
                }
                match records.first() {
                    Some(record) => record.operations.first().is_some_and(|first| {
                        lane.current() == Some(first.request.identity.request_window)
                            && lane.phase != LanePhase::Exhausted
                            && (lane.phase != LanePhase::ClosePending || terminal(record).is_some())
                    }),
                    None => lane.phase != LanePhase::ClosePending,
                }
            })
    }

    /// Operational node drain only. It never rotates retention slots or releases quota.
    /// # Errors
    /// Unknown nodes and invalid authority cannot alter protected registration.
    pub fn begin_drain(&self, node: &NodeIdentity) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self
            .nodes
            .iter()
            .position(|registered| &registered.identity == node)
            .ok_or(DecisionError::NotFound)?;
        let mut next = self.clone();
        if next.nodes[index].mode == NodeMode::Accepting {
            next.nodes[index].mode = NodeMode::Draining;
        }
        Ok(next)
    }

    /// Persist only the host's EXACT acknowledged latest terminal result.
    /// # Errors
    /// Foreign, superseded, uncommitted and nonterminal facts cannot enable close.
    pub fn confirm_terminal(&self, result: &LifecycleResponse) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.index(&result.identity.tenant_id, &result.identity.sandbox_id)?;
        let record = &self.sandboxes[index];
        if !record.released
            || WindowCloseRequest::new(result.clone())
                .validate(&result.identity.node)
                .is_err()
            || record.operations.last().and_then(|op| op.response.as_ref()) != Some(result)
        {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        next.sandboxes[index].terminal_confirmation = Some(result.clone());
        Ok(next)
    }

    /// Commit one slot's close intent before dispatch, without draining its node.
    /// The composition adapter must veto its original metadata/build/publication pins.
    /// # Errors
    /// Active, unconfirmed, stale or changed terminal results cannot close a slot.
    pub fn begin_close(
        &self,
        result: &LifecycleResponse,
    ) -> Result<(Self, WindowCloseRequest), DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.index(&result.identity.tenant_id, &result.identity.sandbox_id)?;
        if terminal(&self.sandboxes[index]) != Some(result) {
            return Err(DecisionError::Conflict);
        }
        let lane_index = self.lane_index(&result.identity.node, result.identity.retention_slot)?;
        let lane = &self.retention_lanes[lane_index];
        if lane.current() != Some(result.identity.request_window)
            || lane.phase == LanePhase::Exhausted
        {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        next.retention_lanes[lane_index].phase = LanePhase::ClosePending;
        Ok((next, WindowCloseRequest::new(result.clone())))
    }

    /// Finite projection of COMMITTED close intents, not an independently growing queue.
    #[must_use]
    pub fn pending_closes(&self) -> Vec<WindowCloseRequest> {
        self.retention_lanes
            .iter()
            .filter(|lane| lane.phase == LanePhase::ClosePending)
            .filter_map(|lane| {
                self.lane_record(&lane.node, lane.retention_slot)
                    .and_then(terminal)
            })
            .cloned()
            .map(WindowCloseRequest::new)
            .collect()
    }

    /// Retire only the exact confirmed terminal incarnation after its durable ACK.
    /// Original business pins must be vetoed in the SAME complete envelope CAS.
    /// Native physical cleanup is never performed or invented by this pure function.
    /// # Errors
    /// Unknown, altered, future or unconfirmed ACKs cannot touch the slot's successor.
    pub fn apply_close_ack(&self, ack: &WindowCloseAck) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let id = &ack.terminal.identity;
        let lane_index = self.lane_index(&id.node, id.retention_slot)?;
        let lane = &self.retention_lanes[lane_index];
        WindowCloseRequest::new(ack.terminal.clone())
            .validate(&lane.node)
            .map_err(|_| DecisionError::Stale)?;
        if id.request_window.get() <= lane.retired_through {
            return if lane.last_retired.as_ref() == Some(&ack.terminal) {
                Ok(self.clone())
            } else {
                Err(DecisionError::Stale)
            };
        }
        if lane.current() != Some(id.request_window) || lane.phase != LanePhase::ClosePending {
            return Err(DecisionError::Conflict);
        }
        let index = self.index(&id.tenant_id, &id.sandbox_id)?;
        if terminal(&self.sandboxes[index]) != Some(&ack.terminal) {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        next.sandboxes.remove(index);
        let lane = &mut next.retention_lanes[lane_index];
        lane.retired_through = id.request_window.get();
        lane.last_retired = Some(ack.terminal.clone());
        lane.phase = if lane.current().is_some() {
            LanePhase::Open
        } else {
            LanePhase::Exhausted
        };
        if next.is_valid() {
            Ok(next)
        } else {
            Err(DecisionError::Invalid)
        }
    }
}

fn valid_last_retired(lane: &RetentionLane, state: &CenterState) -> bool {
    match (&lane.last_retired, lane.retired_through) {
        (None, 0) => true,
        (Some(result), floor) if floor > 0 => {
            result.identity.request_window.get() == floor
                && result.identity.retention_slot == lane.retention_slot
                && crate::validation::number(
                    result.identity.sandbox_id.as_str(),
                    "sandbox-",
                    state.sandbox_highwater,
                )
                .is_some()
                && crate::validation::number(
                    result.identity.operation_id.as_str(),
                    "operation-",
                    state.operation_highwater,
                )
                .is_some()
                && state
                    .tenants
                    .iter()
                    .any(|tenant| tenant.tenant == result.identity.tenant_id)
                && WindowCloseRequest::new(result.clone())
                    .validate(&lane.node)
                    .is_ok()
        }
        _ => false,
    }
}
