//! Controlled HostPort facts around real center reducers, never KVM evidence.
use super::*;
#[derive(Clone)]
pub(super) struct Host {
    pub(super) calls: Arc<AtomicU64>,
    pub(super) observations: Arc<AtomicU64>,
    pub(super) clock: TestClock,
    pub(super) unknown: Arc<AtomicBool>,
    pub(super) transient_unknown: Arc<AtomicU64>,
    pub(super) consume_reply_budget: Arc<AtomicBool>,
    pub(super) unknown_confirmation: Arc<AtomicBool>,
    pub(super) unknown_close: Arc<AtomicBool>,
    pub(super) close_requests: Arc<Mutex<Vec<LifecycleWindowClose>>>,
    pub(super) confirmation_budgets: Arc<Mutex<Vec<Duration>>>,
    pub(super) confirmation_gate: Arc<Mutex<Option<NetworkGate>>>,
    pub(super) recorded: Arc<AtomicBool>,
    pub(super) capacity_refusal: Arc<AtomicBool>,
    pub(super) last: Arc<Mutex<Option<AcceptedLifecycleIntent>>>,
    pub(super) records: Arc<Mutex<Vec<AcceptedLifecycleIntent>>>,
    pub(super) ready: Arc<Mutex<Vec<CreateObservation>>>,
    pub(super) terminals: Arc<Mutex<Vec<TerminalLifecycleObservation>>>,
    pub(super) fence_after_send: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    pub(super) gate: Arc<Mutex<Option<NetworkGate>>>,
    pub(super) late_response: Arc<Mutex<Option<CreateObservation>>>,
    pub(super) late_terminal: Arc<Mutex<Option<TerminalLifecycleObservation>>>,
}
impl Default for Host {
    fn default() -> Self {
        Self {
            calls: Arc::new(AtomicU64::new(0)), observations: Arc::new(AtomicU64::new(0)),
            clock: TestClock(Arc::new(AtomicU64::new(1000))),
            unknown: Arc::new(AtomicBool::new(false)), transient_unknown: Arc::new(AtomicU64::new(0)),
            consume_reply_budget: Arc::new(AtomicBool::new(false)), unknown_confirmation: Arc::new(AtomicBool::new(false)),
            unknown_close: Arc::new(AtomicBool::new(false)), close_requests: Arc::new(Mutex::new(Vec::new())),
            confirmation_budgets: Arc::new(Mutex::new(Vec::new())), confirmation_gate: Arc::new(Mutex::new(None)),
            recorded: Arc::new(AtomicBool::new(false)), capacity_refusal: Arc::new(AtomicBool::new(false)),
            last: Arc::new(Mutex::new(None)), records: Arc::new(Mutex::new(Vec::new())),
            ready: Arc::new(Mutex::new(Vec::new())), terminals: Arc::new(Mutex::new(Vec::new())),
            fence_after_send: Arc::new(Mutex::new(None)), gate: Arc::new(Mutex::new(None)),
            late_response: Arc::new(Mutex::new(None)), late_terminal: Arc::new(Mutex::new(None)),
        }
    }
}
impl Host {
    fn finish_observation(&self, budget: Duration) {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate { gate.entered.send(()).unwrap(); gate.release.recv().unwrap(); }
        if self.consume_reply_budget.load(Ordering::SeqCst) { consume_reply_budget(budget); }
        if let Some(fence) = self.fence_after_send.lock().unwrap().take() { fence.store(true, Ordering::SeqCst); }
    }
    fn begin_confirmation(&self, budget: Duration) -> Result<(), PortError> {
        self.confirmation_budgets.lock().unwrap().push(budget);
        if self.unknown_confirmation.load(Ordering::SeqCst) { return Err(PortError::OutcomeUnknown); }
        Ok(())
    }
    fn finish_confirmation(&self) {
        let gate = self.confirmation_gate.lock().unwrap().take();
        if let Some(gate) = gate { gate.entered.send(()).unwrap(); gate.release.recv().unwrap(); }
    }
}
impl HostPort for Host {
    fn record_lifecycle(&self, intent: &AcceptedLifecycleIntent, _: Duration) -> Result<RecordedLifecycleIntent, PortError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some(intent.clone());
        if self.unknown.load(Ordering::SeqCst) || self.transient_unknown.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok() {
            return Err(PortError::Busy);
        }
        let mut records = self.records.lock().unwrap();
        if let Some(old) = records.iter().find(|r| r.request.binding.key == intent.request.binding.key) {
            if old != intent { return Err(PortError::Conflict); }
        } else { records.push(intent.clone()); }
        let latest = records.iter().filter(|r| r.request.binding.session == intent.request.binding.session
            && r.request.binding.key.owner == intent.request.binding.key.owner).max_by_key(|r| r.accepted_version).unwrap();
        Ok(RecordedLifecycleIntent { intent: intent.clone(), current_session_version: latest.accepted_version,
            current: latest == intent, terminal: false })
    }
    fn observe_lifecycle_create(&self, original: &AcceptedLifecycleIntent, budget: Duration) -> Result<CreateObservation, PortError> {
        self.observations.fetch_add(1, Ordering::SeqCst);
        let current = self.records.lock().unwrap().iter().filter(|r| r.request.binding.session == original.request.binding.session
            && r.request.binding.key.owner == original.request.binding.key.owner).max_by_key(|r| r.accepted_version).cloned().ok_or(PortError::NotFound)?;
        let now = self.clock.0.load(Ordering::SeqCst);
        let mut view = ready_observation(original, &current, now);
        if self.capacity_refusal.load(Ordering::SeqCst) && current == *original {
            view.historical_create_succeeded = false;
            view.readiness = CurrentCreateReadiness::Refused { reason: CreateRefusalReason::Capacity };
        } else if self.recorded.load(Ordering::SeqCst) {
            view.historical_create_succeeded = false;
            view.readiness = CurrentCreateReadiness::Pending;
        } else if let LifecycleOperation::Renew { end_at_ms, extend_only, .. } = current.request.operation {
            let prior = self.ready.lock().unwrap().iter().rev().find(|v| v.request == *original).cloned();
            let CurrentCreateReadiness::Observed { facts } = &mut view.readiness else { return Err(PortError::Busy); };
            let target = u64::try_from(end_at_ms).map_err(|_| PortError::Busy)?;
            let previous_end = prior.as_ref().and_then(|v| match &v.readiness { CurrentCreateReadiness::Observed { facts } => Some(facts.end_at_ms), _ => None }).unwrap();
            facts.end_at_ms = if extend_only { target.max(previous_end) } else { target };
        }
        self.ready.lock().unwrap().push(view.clone());
        self.finish_observation(budget);
        Ok(self.late_response.lock().unwrap().take().unwrap_or(view))
    }
    fn observe_lifecycle_terminal(&self, intent: &AcceptedLifecycleIntent, budget: Duration) -> Result<TerminalLifecycleObservation, PortError> {
        let result = TerminalLifecycleObservation { request: intent.clone(), status: if self.recorded.load(Ordering::SeqCst) { TerminalStatus::Pending } else { TerminalStatus::Cleaned } };
        self.terminals.lock().unwrap().push(result.clone());
        self.finish_observation(budget);
        Ok(self.late_terminal.lock().unwrap().take().unwrap_or(result))
    }
    fn confirm_lifecycle_terminal(&self, result: &TerminalLifecycleObservation, budget: Duration) -> Result<ConfirmedLifecycleResult, PortError> {
        self.begin_confirmation(budget)?;
        if !self.terminals.lock().unwrap().contains(result) || result.status != TerminalStatus::Cleaned { return Err(PortError::Conflict); }
        self.finish_confirmation();
        Ok(ConfirmedLifecycleResult { result: result.clone() })
    }
    fn confirm_lifecycle_renewal(&self, result: &RenewalConfirmation, budget: Duration) -> Result<ConfirmedRenewal, PortError> {
        self.begin_confirmation(budget)?;
        let saved = self.ready.lock().unwrap().iter().any(|v| v.current_request == result.request
            && matches!(&v.readiness, CurrentCreateReadiness::Observed { facts } if facts.end_at_ms == result.effective_end_at_ms));
        if !saved { return Err(PortError::Conflict); }
        self.finish_confirmation();
        Ok(ConfirmedRenewal { result: result.clone() })
    }
    fn close_lifecycle_window(&self, request: &LifecycleWindowClose, _: Duration) -> Result<ClosedLifecycleWindow, PortError> {
        self.close_requests.lock().unwrap().push(request.clone());
        if self.unknown_close.load(Ordering::SeqCst) { return Err(PortError::OutcomeUnknown); }
        Ok(ClosedLifecycleWindow { terminal: request.terminal.clone(), retired_through: request.terminal.request.retention.window })
    }
}
