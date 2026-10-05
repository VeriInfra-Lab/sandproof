//! Bounded, server-issued request windows inside the controller reducer.
//!
//! A window permits **new** admissions until its cutoff. Accepted request
//! tuples and their promised result retention live in the caller's durable job
//! ledger: lookup `(tenant, window, client key)` there before asking `Admit`.
//! Closing a window never cancels, forgets, or unpins an accepted job. Legacy
//! no-window keys are a separate identity domain, never an implicit window.
//!
//! Only committed decisions advance the time floor. Before returning an expiry
//! refusal, the adapter must durably commit `AdvanceTime` (which closes expired
//! windows), even when the subsequent admission rejects. A rejected reduction
//! does not persist an observation of time. The adapter must commit successful
//! admission validation and the accepted tuple in the same serialized
//! transaction; validation alone is not a reservation or permission to dispatch.
//!
//! IDs start at zero and never wrap. An absent ID at/below `issued_through` is
//! closed; an ID above it was never issued. No closed-ID list is needed, and an
//! old pending job cannot pin any open-window record. The external ID must also
//! bind the deployment namespace, which must not be reused with a reset store.

use crate::TenantId;
use std::fmt;

/// A server-issued sequence within one persistent deployment namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct WindowId(u64);

impl WindowId {
    /// Decode an identifier, without asserting that the server issued it.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Explicit deployment bounds; there are no production defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "WindowLimitsWire"))]
pub struct WindowLimits {
    max_open_windows: u64,
    max_duration_ms: u64,
}

impl WindowLimits {
    /// # Errors
    /// Both the record limit and maximum duration must be positive.
    pub const fn new(max_open_windows: u64, max_duration_ms: u64) -> Result<Self, WindowReject> {
        if max_open_windows == 0 || max_duration_ms == 0 {
            return Err(WindowReject::InvalidLimits);
        }
        Ok(Self {
            max_open_windows,
            max_duration_ms,
        })
    }

    #[must_use]
    pub const fn max_open_windows(self) -> u64 {
        self.max_open_windows
    }

    #[must_use]
    pub const fn max_duration_ms(self) -> u64 {
        self.max_duration_ms
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowLimitsWire {
    max_open_windows: u64,
    max_duration_ms: u64,
}

#[cfg(feature = "serde")]
impl TryFrom<WindowLimitsWire> for WindowLimits {
    type Error = WindowReject;

    fn try_from(value: WindowLimitsWire) -> Result<Self, Self::Error> {
        Self::new(value.max_open_windows, value.max_duration_ms)
    }
}

/// Read-only binding returned by issuance and exposed by state queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct WindowRecord {
    id: WindowId,
    tenant: TenantId,
    issued_at_ms: u64,
    expires_at_ms: u64,
}

impl WindowRecord {
    #[must_use]
    pub const fn id(self) -> WindowId {
        self.id
    }

    #[must_use]
    pub const fn tenant(self) -> TenantId {
        self.tenant
    }

    #[must_use]
    pub const fn issued_at_ms(self) -> u64 {
        self.issued_at_ms
    }

    #[must_use]
    pub const fn expires_at_ms(self) -> u64 {
        self.expires_at_ms
    }
}

/// Complete durable window state. Open records are strictly ID-ordered.
/// `None` means no ID has ever been issued; `Some(0)` includes the first ID.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "WindowStateWire"))]
pub struct WindowState {
    limits: WindowLimits,
    issued_through: Option<u64>,
    time_floor_ms: u64,
    open: Vec<WindowRecord>,
}

impl WindowState {
    #[must_use]
    pub const fn new(limits: WindowLimits) -> Self {
        Self {
            limits,
            issued_through: None,
            time_floor_ms: 0,
            open: Vec::new(),
        }
    }

    #[must_use]
    pub const fn limits(&self) -> WindowLimits {
        self.limits
    }

    #[must_use]
    pub const fn issued_through(&self) -> Option<u64> {
        self.issued_through
    }

    #[must_use]
    pub const fn time_floor_ms(&self) -> u64 {
        self.time_floor_ms
    }

    /// The persisted monotone time floor, in milliseconds.
    #[must_use]
    pub const fn time_floor(&self) -> u64 {
        self.time_floor_ms
    }

    #[must_use]
    pub fn open_windows(&self) -> &[WindowRecord] {
        &self.open
    }

    #[must_use]
    pub fn window(&self, id: WindowId) -> Option<&WindowRecord> {
        self.open.iter().find(|record| record.id == id)
    }

    /// Validate a persisted snapshot before reducing or trusting its views.
    /// This checks structure, not authenticity of the trusted storage medium.
    /// Deserialization also calls this method before returning a state.
    /// # Errors
    /// Rejects invalid bounds, excess/duplicate/unissued records, and dates
    /// inconsistent with issuance, the configured duration or the time floor.
    pub fn validate(&self) -> Result<(), WindowReject> {
        WindowLimits::new(self.limits.max_open_windows, self.limits.max_duration_ms)?;
        if u64::try_from(self.open.len()).map_err(|_| WindowReject::InvalidState)?
            > self.limits.max_open_windows
        {
            return Err(WindowReject::InvalidState);
        }
        let mut prior = None;
        for record in &self.open {
            let duration = record
                .expires_at_ms
                .checked_sub(record.issued_at_ms)
                .ok_or(WindowReject::InvalidState)?;
            if self.issued_through.is_none_or(|last| record.id.0 > last)
                || prior.is_some_and(|last| record.id <= last)
                || record.issued_at_ms > self.time_floor_ms
                || record.expires_at_ms <= self.time_floor_ms
                || duration == 0
                || duration > self.limits.max_duration_ms
            {
                return Err(WindowReject::InvalidState);
            }
            prior = Some(record.id);
        }
        Ok(())
    }

    fn advance_time(&mut self, now_ms: u64) -> u64 {
        let before = self.open.len();
        self.open.retain(|record| record.expires_at_ms > now_ms);
        self.time_floor_ms = now_ms;
        // validate() has established that the collection length fits u64.
        u64::try_from(before - self.open.len()).expect("validated window count")
    }

    fn require_open(
        &self,
        tenant: TenantId,
        window: WindowId,
        now_ms: u64,
    ) -> Result<WindowRecord, WindowReject> {
        let Some(record) = self.window(window).copied() else {
            return Err(
                if self.issued_through.is_some_and(|last| window.0 <= last) {
                    WindowReject::ClosedWindow
                } else {
                    WindowReject::UnknownWindow
                },
            );
        };
        if record.tenant != tenant {
            return Err(WindowReject::WrongTenant);
        }
        if now_ms >= record.expires_at_ms {
            return Err(WindowReject::ClosedWindow);
        }
        Ok(record)
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowStateWire {
    limits: WindowLimits,
    issued_through: Option<u64>,
    time_floor_ms: u64,
    open: Vec<WindowRecord>,
}

#[cfg(feature = "serde")]
impl TryFrom<WindowStateWire> for WindowState {
    type Error = WindowReject;

    fn try_from(value: WindowStateWire) -> Result<Self, Self::Error> {
        let state = Self {
            limits: value.limits,
            issued_through: value.issued_through,
            time_floor_ms: value.time_floor_ms,
            open: value.open,
        };
        state.validate()?;
        Ok(state)
    }
}

/// Trusted inputs to the existing controller's maintenance boundary.
/// Tenant identity comes from authentication, never a public request body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub enum WindowInput {
    Issue {
        tenant: TenantId,
        now_ms: u64,
        duration_ms: u64,
    },
    Close {
        tenant: TenantId,
        window: WindowId,
        now_ms: u64,
    },
    AdvanceTime {
        now_ms: u64,
    },
    /// Check only a new request's window. Existing accepted tuple lookup,
    /// key/result count and byte reservations are mandatory separate steps
    /// within the same durable transaction, before any external side effect.
    Admit {
        tenant: TenantId,
        window: WindowId,
        now_ms: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WindowEvent {
    Issued(WindowRecord),
    Closed {
        window: WindowId,
    },
    TimeAdvanced {
        now_ms: u64,
        expired: u64,
    },
    /// Not an accepted job or a reservation; see `WindowInput::Admit`.
    AdmissionValidated(WindowRecord),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WindowReject {
    InvalidLimits,
    InvalidState,
    InvalidDuration,
    ClockRegression,
    TimeOverflow,
    IdExhausted,
    CapacityExceeded,
    UnknownWindow,
    ClosedWindow,
    WrongTenant,
}

impl fmt::Display for WindowReject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for WindowReject {}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct WindowDecision {
    pub event: WindowEvent,
    pub next: WindowState,
}

/// Reduce a window input as part of the existing controller boundary.
/// No filesystem, clock source, randomness, job execution or persistence lives
/// here. Only the returned successor can become the next durable state.
/// # Errors
/// Rejects malformed state/input, clock regression, exhausted capacity or
/// identities, and new admission/closure of unknown, closed or foreign windows.
/// Rejection leaves the input state unchanged, including its time floor.
pub fn reduce_windows(
    state: &WindowState,
    input: WindowInput,
) -> Result<WindowDecision, WindowReject> {
    state.validate()?;
    let now_ms = match &input {
        WindowInput::Issue { now_ms, .. }
        | WindowInput::Close { now_ms, .. }
        | WindowInput::AdvanceTime { now_ms }
        | WindowInput::Admit { now_ms, .. } => *now_ms,
    };
    if now_ms < state.time_floor_ms {
        return Err(WindowReject::ClockRegression);
    }
    let mut next = state.clone();
    let event = match input {
        WindowInput::Issue {
            tenant,
            duration_ms,
            ..
        } => {
            if duration_ms == 0 || duration_ms > state.limits.max_duration_ms {
                return Err(WindowReject::InvalidDuration);
            }
            let expires_at_ms = now_ms
                .checked_add(duration_ms)
                .ok_or(WindowReject::TimeOverflow)?;
            let id = match state.issued_through {
                None => 0,
                Some(last) => last.checked_add(1).ok_or(WindowReject::IdExhausted)?,
            };
            next.advance_time(now_ms);
            if u64::try_from(next.open.len()).map_err(|_| WindowReject::InvalidState)?
                >= state.limits.max_open_windows
            {
                return Err(WindowReject::CapacityExceeded);
            }
            let record = WindowRecord {
                id: WindowId(id),
                tenant,
                issued_at_ms: now_ms,
                expires_at_ms,
            };
            next.issued_through = Some(id);
            next.open.push(record);
            WindowEvent::Issued(record)
        }
        WindowInput::Close { tenant, window, .. } => {
            state.require_open(tenant, window, now_ms)?;
            next.advance_time(now_ms);
            next.open.retain(|record| record.id != window);
            WindowEvent::Closed { window }
        }
        WindowInput::AdvanceTime { .. } => {
            let expired = next.advance_time(now_ms);
            WindowEvent::TimeAdvanced { now_ms, expired }
        }
        WindowInput::Admit { tenant, window, .. } => {
            let record = state.require_open(tenant, window, now_ms)?;
            next.advance_time(now_ms);
            WindowEvent::AdmissionValidated(record)
        }
    };
    Ok(WindowDecision { event, next })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(count: u64, duration: u64) -> WindowState {
        WindowState::new(WindowLimits::new(count, duration).unwrap())
    }

    fn issue(state: &mut WindowState, tenant: u64, now: u64, duration: u64) -> WindowId {
        let decision = reduce_windows(
            state,
            WindowInput::Issue {
                tenant: TenantId(tenant),
                now_ms: now,
                duration_ms: duration,
            },
        )
        .unwrap();
        let WindowEvent::Issued(record) = decision.event else {
            panic!("issuance returned a different event");
        };
        assert_eq!(record.tenant(), TenantId(tenant));
        assert_eq!(record.issued_at_ms(), now);
        assert_eq!(record.expires_at_ms(), now + duration);
        decision.next.validate().unwrap();
        *state = decision.next;
        record.id()
    }

    fn rejected(state: &WindowState, input: WindowInput, reason: WindowReject) {
        let before = state.clone();
        assert_eq!(reduce_windows(state, input), Err(reason));
        assert_eq!(state, &before);
    }

    fn admit(tenant: u64, window: WindowId, now_ms: u64) -> WindowInput {
        WindowInput::Admit {
            tenant: TenantId(tenant),
            window,
            now_ms,
        }
    }

    #[test]
    fn explicit_limits_reject_zero_and_do_not_supply_defaults() {
        assert_eq!(WindowLimits::new(0, 1), Err(WindowReject::InvalidLimits));
        assert_eq!(WindowLimits::new(1, 0), Err(WindowReject::InvalidLimits));
        let limits = WindowLimits::new(7, 11).unwrap();
        assert_eq!(limits.max_open_windows(), 7);
        assert_eq!(limits.max_duration_ms(), 11);
        assert_eq!(WindowState::new(limits).limits(), limits);
    }

    #[test]
    fn zero_is_a_valid_first_id_and_never_issued_is_distinct() {
        let mut state = fresh(1, 10);
        assert_eq!(state.issued_through(), None);
        rejected(
            &state,
            admit(1, WindowId::new(0), 0),
            WindowReject::UnknownWindow,
        );
        let id = issue(&mut state, 1, 0, 10);
        assert_eq!(id.value(), 0);
        assert_eq!(state.issued_through(), Some(0));
        let decision = reduce_windows(&state, admit(1, id, 9)).unwrap();
        assert!(matches!(decision.event, WindowEvent::AdmissionValidated(_)));
        assert_eq!(decision.next.time_floor_ms(), 9);
    }

    #[test]
    fn issue_validates_duration_capacity_and_checked_time_without_mutation() {
        let mut state = fresh(1, 10);
        for duration_ms in [0, 11] {
            rejected(
                &state,
                WindowInput::Issue {
                    tenant: TenantId(1),
                    now_ms: 0,
                    duration_ms,
                },
                WindowReject::InvalidDuration,
            );
        }
        rejected(
            &state,
            WindowInput::Issue {
                tenant: TenantId(1),
                now_ms: u64::MAX,
                duration_ms: 1,
            },
            WindowReject::TimeOverflow,
        );
        issue(&mut state, 1, 0, 10);
        rejected(
            &state,
            WindowInput::Issue {
                tenant: TenantId(2),
                now_ms: 1,
                duration_ms: 1,
            },
            WindowReject::CapacityExceeded,
        );
        assert_eq!(state.time_floor_ms(), 0);
        let next = issue(&mut state, 2, 10, 1);
        assert_eq!(next.value(), 1);
        assert_eq!(state.open_windows().len(), 1);
    }

    #[test]
    fn arbitrary_close_order_never_reinserts_or_reuses_a_sequence() {
        let mut state = fresh(4, 100);
        let ids = [
            issue(&mut state, 1, 0, 100),
            issue(&mut state, 1, 0, 100),
            issue(&mut state, 1, 0, 100),
            issue(&mut state, 1, 0, 100),
        ];
        for id in [ids[2], ids[0], ids[3], ids[1]] {
            let input = WindowInput::Close {
                tenant: TenantId(1),
                window: id,
                now_ms: 1,
            };
            state = reduce_windows(&state, input).unwrap().next;
            rejected(&state, input, WindowReject::ClosedWindow);
            rejected(&state, admit(1, id, 1), WindowReject::ClosedWindow);
            assert_eq!(state.issued_through(), Some(3));
        }
        assert!(state.open_windows().is_empty());
        assert_eq!(issue(&mut state, 1, 1, 100).value(), 4);
        rejected(
            &state,
            admit(1, WindowId::new(5), 1),
            WindowReject::UnknownWindow,
        );
    }

    #[test]
    fn tenant_binding_is_required_for_admission_and_close() {
        let mut state = fresh(1, 10);
        let id = issue(&mut state, 1, 0, 10);
        rejected(&state, admit(2, id, 1), WindowReject::WrongTenant);
        rejected(
            &state,
            WindowInput::Close {
                tenant: TenantId(2),
                window: id,
                now_ms: 1,
            },
            WindowReject::WrongTenant,
        );
        assert!(reduce_windows(&state, admit(1, id, 1)).is_ok());
    }

    #[test]
    fn newer_windows_can_expire_before_older_windows_without_a_closed_list() {
        let mut state = fresh(2, 100);
        let oldest = issue(&mut state, 1, 0, 100);
        let newer = issue(&mut state, 2, 0, 1);
        state = reduce_windows(&state, WindowInput::AdvanceTime { now_ms: 1 })
            .unwrap()
            .next;
        assert_eq!(state.open_windows().len(), 1);
        assert!(state.window(oldest).is_some());
        assert!(state.window(newer).is_none());
        rejected(&state, admit(2, newer, 1), WindowReject::ClosedWindow);
        assert_eq!(issue(&mut state, 3, 1, 1).value(), 2);
        rejected(
            &state,
            WindowInput::Close {
                tenant: TenantId(3),
                window: WindowId::new(3),
                now_ms: 1,
            },
            WindowReject::UnknownWindow,
        );
    }

    #[test]
    fn expiry_is_exclusive_and_persisted_time_cannot_move_backwards() {
        let mut state = fresh(1, 10);
        let id = issue(&mut state, 1, 10, 10);
        assert!(reduce_windows(&state, admit(1, id, 19)).is_ok());
        rejected(&state, admit(1, id, 20), WindowReject::ClosedWindow);
        // Rejection does not commit the observed time. The adapter must commit
        // this maintenance decision before responding with an expiry refusal.
        let decision = reduce_windows(&state, WindowInput::AdvanceTime { now_ms: 20 }).unwrap();
        assert_eq!(
            decision.event,
            WindowEvent::TimeAdvanced {
                now_ms: 20,
                expired: 1
            }
        );
        state = decision.next;
        assert!(state.open_windows().is_empty());
        rejected(&state, admit(1, id, 19), WindowReject::ClockRegression);
        rejected(&state, admit(1, id, 20), WindowReject::ClosedWindow);
        for input in [
            WindowInput::AdvanceTime { now_ms: 19 },
            WindowInput::Issue {
                tenant: TenantId(1),
                now_ms: 19,
                duration_ms: 1,
            },
            WindowInput::Close {
                tenant: TenantId(1),
                window: id,
                now_ms: 19,
            },
        ] {
            rejected(&state, input, WindowReject::ClockRegression);
        }
    }

    #[test]
    fn successful_admission_and_close_also_advance_expiry_of_other_windows() {
        let mut state = fresh(3, 10);
        let first = issue(&mut state, 1, 0, 1);
        let second = issue(&mut state, 1, 0, 5);
        let third = issue(&mut state, 2, 0, 10);
        state = reduce_windows(&state, admit(1, second, 1)).unwrap().next;
        assert!(state.window(first).is_none());
        assert_eq!(state.time_floor_ms(), 1);
        state = reduce_windows(
            &state,
            WindowInput::Close {
                tenant: TenantId(2),
                window: third,
                now_ms: 5,
            },
        )
        .unwrap()
        .next;
        assert!(state.open_windows().is_empty());
        assert_eq!(state.time_floor_ms(), 5);
        assert_eq!(state.issued_through(), Some(2));
    }

    #[test]
    fn a_pending_old_tuple_does_not_pin_windows_or_subsequent_closures() {
        let mut state = fresh(1, 1);
        let old = issue(&mut state, 1, 0, 1);
        let accepted_tuple = (TenantId(1), old, "client-key", "pending-build");
        for now in 1..=1000 {
            assert_eq!(issue(&mut state, 2, now, 1).value(), now);
            assert_eq!(state.open_windows().len(), 1);
        }
        rejected(&state, admit(1, old, 1000), WindowReject::ClosedWindow);
        // The actual job remains in the separate accepted-tuple ledger; window
        // expiry neither consumes it nor requires retaining that window row.
        assert_eq!(
            accepted_tuple,
            (TenantId(1), WindowId::new(0), "client-key", "pending-build")
        );
    }

    #[test]
    fn last_identifier_is_issued_once_then_exhaustion_never_wraps() {
        let mut state = fresh(1, 10);
        state.issued_through = Some(u64::MAX - 1);
        let last = issue(&mut state, 1, 0, 1);
        assert_eq!(last.value(), u64::MAX);
        state = reduce_windows(&state, WindowInput::AdvanceTime { now_ms: 1 })
            .unwrap()
            .next;
        rejected(
            &state,
            WindowInput::Issue {
                tenant: TenantId(1),
                now_ms: 1,
                duration_ms: 1,
            },
            WindowReject::IdExhausted,
        );
        rejected(&state, admit(1, last, 1), WindowReject::ClosedWindow);
        assert!(state.open_windows().is_empty());
    }

    #[test]
    fn last_time_is_a_valid_cutoff_but_cannot_overflow_or_reopen() {
        let mut state = fresh(1, 1);
        let id = issue(&mut state, 1, u64::MAX - 1, 1);
        state = reduce_windows(&state, WindowInput::AdvanceTime { now_ms: u64::MAX })
            .unwrap()
            .next;
        rejected(&state, admit(1, id, u64::MAX), WindowReject::ClosedWindow);
        rejected(
            &state,
            WindowInput::Issue {
                tenant: TenantId(1),
                now_ms: u64::MAX,
                duration_ms: 1,
            },
            WindowReject::TimeOverflow,
        );
    }

    #[test]
    fn corrupt_checkpoint_shapes_are_rejected_before_reduction() {
        let mut valid = fresh(2, 10);
        issue(&mut valid, 1, 5, 10);
        let mut variants = Vec::new();
        let mut bad = valid.clone();
        bad.issued_through = None;
        variants.push(bad);
        let mut bad = valid.clone();
        bad.open[0].id = WindowId(1);
        variants.push(bad);
        let mut bad = valid.clone();
        bad.open.push(bad.open[0]);
        variants.push(bad);
        let mut bad = valid.clone();
        bad.time_floor_ms = 4;
        variants.push(bad);
        let mut bad = valid.clone();
        bad.time_floor_ms = 15;
        variants.push(bad);
        let mut bad = valid.clone();
        bad.open[0].expires_at_ms = 16;
        variants.push(bad);
        let mut bad = valid.clone();
        bad.open[0].expires_at_ms = 4;
        variants.push(bad);
        let mut bad = valid;
        bad.open[0].expires_at_ms = 5;
        variants.push(bad);
        for state in variants {
            rejected(
                &state,
                WindowInput::AdvanceTime { now_ms: 20 },
                WindowReject::InvalidState,
            );
        }
        let state = WindowState {
            limits: WindowLimits {
                max_open_windows: 0,
                max_duration_ms: 1,
            },
            issued_through: None,
            time_floor_ms: 0,
            open: Vec::new(),
        };
        rejected(
            &state,
            WindowInput::AdvanceTime { now_ms: 0 },
            WindowReject::InvalidLimits,
        );
    }

    #[test]
    fn expired_record_limit_is_reclaimed_without_any_unbounded_closed_list() {
        let mut state = fresh(2, 10);
        issue(&mut state, 1, 0, 1);
        issue(&mut state, 2, 0, 2);
        let decision = reduce_windows(&state, WindowInput::AdvanceTime { now_ms: 2 }).unwrap();
        assert_eq!(
            decision.event,
            WindowEvent::TimeAdvanced {
                now_ms: 2,
                expired: 2
            }
        );
        state = decision.next;
        let again = reduce_windows(&state, WindowInput::AdvanceTime { now_ms: 2 }).unwrap();
        assert_eq!(again.next, state);
        assert_eq!(
            again.event,
            WindowEvent::TimeAdvanced {
                now_ms: 2,
                expired: 0
            }
        );
        assert_eq!(issue(&mut state, 3, 2, 10).value(), 2);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_wire_restoration_runs_validation_and_preserves_frontiers() {
        let mut state = fresh(1, 10);
        issue(&mut state, 1, 3, 10);
        let restored = WindowState::try_from(WindowStateWire {
            limits: state.limits,
            issued_through: state.issued_through,
            time_floor_ms: state.time_floor_ms,
            open: state.open.clone(),
        })
        .unwrap();
        assert_eq!(restored, state);
        assert_eq!(
            WindowLimits::try_from(WindowLimitsWire {
                max_open_windows: 0,
                max_duration_ms: 1
            }),
            Err(WindowReject::InvalidLimits)
        );
        assert_eq!(
            WindowState::try_from(WindowStateWire {
                limits: state.limits,
                issued_through: None,
                time_floor_ms: state.time_floor_ms,
                open: state.open
            }),
            Err(WindowReject::InvalidState)
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn actual_serde_limit_deserialization_cannot_bypass_positive_bounds() {
        use serde::{Deserialize, de::value::MapDeserializer};

        for (count, duration) in [(0, 1), (1, 0)] {
            let fields = [("max_open_windows", count), ("max_duration_ms", duration)];
            let value = MapDeserializer::<_, serde::de::value::Error>::new(fields.into_iter());
            assert!(WindowLimits::deserialize(value).is_err());
        }
        let fields = [("max_open_windows", 2), ("max_duration_ms", 5)];
        let value = MapDeserializer::<_, serde::de::value::Error>::new(fields.into_iter());
        assert_eq!(
            WindowLimits::deserialize(value).unwrap(),
            WindowLimits::new(2, 5).unwrap()
        );
    }
}
