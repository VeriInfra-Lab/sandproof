//! Pure window decisions only; no network, storage, host sweep or TTL.
//! Adapters must commit close intent before dispatch and veto outstanding
//! business/publication references before committing terminal-history removal.
use crate::{CenterState, DecisionError, NodeWindow, WindowPhase};
use sandproof_protocol::internal::{
    NodeIdentity, PositiveCounter,
    lifecycle::{LifecycleFact, LifecycleResponse},
    window::{WindowCloseAck, WindowCloseRequest},
};
use std::collections::BTreeSet;

fn terminal(response: &LifecycleResponse) -> bool {
    matches!(
        response.fact,
        LifecycleFact::Cleaned | LifecycleFact::NotAccepted
    )
}

impl CenterState {
    /// Read-only durable frontier, never supplied by node health observations.
    #[must_use]
    pub fn window(&self, node: &NodeIdentity) -> Option<&NodeWindow> {
        self.node_windows.iter().find(|window| &window.node == node)
    }

    fn window_index(&self, node: &NodeIdentity) -> Result<usize, DecisionError> {
        self.node_windows
            .iter()
            .position(|window| &window.node == node)
            .ok_or(DecisionError::NotFound)
    }

    fn terminal_window(&self, node: &NodeIdentity, window: PositiveCounter) -> bool {
        self.sandboxes
            .iter()
            .filter(|record| {
                record.operations.first().is_some_and(|first| {
                    &first.request.identity.node == node
                        && first.request.identity.request_window == window
                })
            })
            .all(|record| {
                record.released
                    && record.operations.last().is_some_and(|last| {
                        last.response.as_ref().is_some_and(|response| {
                            terminal(response)
                                && record.terminal_confirmation.as_ref() == Some(response)
                        })
                    })
            })
    }

    /// Finite background work derived from the current authority, never from
    /// elapsed time, a missing heartbeat or an independent retirement queue.
    /// Empty successors are deliberately excluded: rotating them would consume
    /// the window identity space without any completed lifecycle history.
    #[must_use]
    pub fn idle_window_candidates(&self) -> Vec<NodeIdentity> {
        self.node_windows
            .iter()
            .filter(|window| matches!(window.phase, WindowPhase::Open | WindowPhase::Draining))
            .filter_map(|window| {
                let current = window.current?;
                let contains_history = self.sandboxes.iter().any(|record| {
                    record.operations.first().is_some_and(|first| {
                        first.request.identity.node == window.node
                            && first.request.identity.request_window == current
                    })
                });
                (contains_history && self.terminal_window(&window.node, current))
                    .then(|| window.node.clone())
            })
            .collect()
    }

    pub(crate) fn valid_windows(&self) -> bool {
        let mut nodes = BTreeSet::new();
        self.node_windows.len() == self.nodes.len()
            && self.node_windows.iter().all(|window| {
                if !nodes.insert(window.node.node_id.as_str())
                    || !self.nodes.iter().any(|node| node.identity == window.node)
                {
                    return false;
                }
                match (window.phase, window.current) {
                    (WindowPhase::Exhausted, None) => window.retired_through == u64::MAX,
                    (WindowPhase::Exhausted, Some(_)) | (_, None) => false,
                    (phase, Some(current)) => {
                        window.retired_through.checked_add(1) == Some(current.get())
                            && (phase != WindowPhase::ClosePending
                                || self.terminal_window(&window.node, current))
                    }
                }
            })
    }

    /// Stop NEW creates in this node's window. Existing lifecycle work is not
    /// cancelled. The adapter must commit this state before treating it as closed
    /// to admission; draining alone never releases a resource or native pin.
    /// # Errors
    /// Unknown nodes, invalid authority or exhausted windows are refused.
    pub fn begin_drain(&self, node: &NodeIdentity) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.window_index(node)?;
        if self.node_windows[index].phase == WindowPhase::Exhausted {
            return Err(DecisionError::Exhausted);
        }
        let mut next = self.clone();
        if next.node_windows[index].phase == WindowPhase::Open {
            next.node_windows[index].phase = WindowPhase::Draining;
        }
        Ok(next)
    }

    /// Record the host's EXACT durable acknowledgement of the latest already
    /// committed terminal result. Authenticated transport and actual host commit
    /// are adapter obligations, not facts established by this pure function.
    /// # Errors
    /// Unknown, superseded, nonterminal or altered results cannot enable close.
    pub fn confirm_terminal(&self, result: &LifecycleResponse) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.index(&result.identity.tenant_id, &result.identity.sandbox_id)?;
        let record = &self.sandboxes[index];
        if !record.released
            || !terminal(result)
            || record.operations.last().and_then(|op| op.response.as_ref()) != Some(result)
        {
            return Err(DecisionError::Stale);
        }
        let mut next = self.clone();
        next.sandboxes[index].terminal_confirmation = Some(result.clone());
        Ok(next)
    }

    /// Produce one fixed close identity AFTER draining and exact terminal
    /// confirmation. The returned state must durably commit before dispatch.
    /// Lost replies retain this same close, never create another operation.
    /// # Errors
    /// Open, active, unknown, stopped-only or unconfirmed windows cannot close.
    pub fn begin_close(
        &self,
        node: &NodeIdentity,
    ) -> Result<(Self, WindowCloseRequest), DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.window_index(node)?;
        let frontier = &self.node_windows[index];
        let current = frontier.current.ok_or(DecisionError::Exhausted)?;
        if frontier.phase == WindowPhase::Open || !self.terminal_window(node, current) {
            return Err(DecisionError::Conflict);
        }
        let mut next = self.clone();
        next.node_windows[index].phase = WindowPhase::ClosePending;
        Ok((next, WindowCloseRequest::new(node.clone(), current)))
    }

    /// A bounded outbox projection from VALIDATED, COMMITTED authority; there is
    /// no independently growing close-retry queue or automatic window issuance.
    #[must_use]
    pub fn pending_closes(&self) -> Vec<WindowCloseRequest> {
        self.node_windows
            .iter()
            .filter(|window| window.phase == WindowPhase::ClosePending)
            .filter_map(|window| {
                window
                    .current
                    .map(|current| WindowCloseRequest::new(window.node.clone(), current))
            })
            .collect()
    }

    /// Remove only confirmed terminal LIFECYCLE history after the exact close
    /// acknowledgement. This is NOT a physical cleanup or business-pin check.
    /// The composition adapter MUST veto live metadata/build/publication pins
    /// and atomically preserve the full business envelope's validation/CAS;
    /// blindly replacing only its center projection is not a valid retirement.
    /// Global identity highwaters survive. MAX retires into permanent Exhausted.
    /// # Errors
    /// Unexpected/foreign acknowledgements cannot advance or erase a window.
    pub fn apply_close_ack(&self, ack: &WindowCloseAck) -> Result<Self, DecisionError> {
        if !self.is_valid() {
            return Err(DecisionError::Invalid);
        }
        let index = self.window_index(&ack.identity.node)?;
        let frontier = &self.node_windows[index];
        ack.identity
            .validate_node(&frontier.node)
            .map_err(|_| DecisionError::Stale)?;
        let target = ack.identity.request_window;
        if target.get() <= frontier.retired_through {
            // This namespace issued every sequence through its durable frontier.
            // A duplicate acknowledgement cannot touch its active successor.
            return Ok(self.clone());
        }
        let current = frontier.current.ok_or(DecisionError::Exhausted)?;
        let request = WindowCloseRequest::new(frontier.node.clone(), current);
        ack.validate_for(&request)
            .map_err(|_| DecisionError::Stale)?;
        if frontier.phase != WindowPhase::ClosePending
            || !self.terminal_window(&frontier.node, current)
        {
            return Err(DecisionError::Conflict);
        }
        let mut next = self.clone();
        next.sandboxes.retain(|record| {
            record.operations.first().is_none_or(|first| {
                first.request.identity.node != frontier.node
                    || first.request.identity.request_window != current
            })
        });
        let window = &mut next.node_windows[index];
        window.retired_through = current.get();
        window.current = current.checked_next().ok();
        window.phase = if window.current.is_some() {
            WindowPhase::Open
        } else {
            WindowPhase::Exhausted
        };
        if next.is_valid() {
            Ok(next)
        } else {
            Err(DecisionError::Invalid)
        }
    }
}
