//! Exact terminal confirmation and finite retained-position retirement.
//! These carriers are not proof tokens. The original journal must check its
//! own durable confirmation and all remaining resource/material responsibility.
use super::{RetentionWindowId, TerminalLifecycleObservation, TerminalStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedLifecycleResult {
    pub result: TerminalLifecycleObservation,
}
impl ConfirmedLifecycleResult {
    #[must_use]
    pub fn matches(&self, result: &TerminalLifecycleObservation) -> bool {
        self.result == *result
            && result.status == TerminalStatus::Cleaned
            && result.matches_request(&result.request)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleWindowClose {
    pub terminal: TerminalLifecycleObservation,
}
impl LifecycleWindowClose {
    /// Shape only. A caller cannot establish journal confirmation by sending
    /// this value, and closing never supplies missing physical cleanup facts.
    #[must_use]
    pub fn is_terminal_result(&self) -> bool {
        self.terminal.status == TerminalStatus::Cleaned
            && self.terminal.matches_request(&self.terminal.request)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosedLifecycleWindow {
    pub terminal: TerminalLifecycleObservation,
    pub retired_through: RetentionWindowId,
}
impl ClosedLifecycleWindow {
    /// Only the exact recent terminal stamp can acknowledge a repeated close.
    /// A newer frontier, absent row or echoed request alone cannot do so.
    #[must_use]
    pub fn matches(&self, request: &LifecycleWindowClose) -> bool {
        request.is_terminal_result()
            && self.terminal == request.terminal
            && self.retired_through == request.terminal.request.retention.window
    }
}
