//! Branch permits share the native management lock and parent route drain.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, DataAuthorization, DataKind,
    DataLease, DataTarget, RelayReaper, TrackedRelayChild, map_journal, same_secret, watch,
};
use crate::{
    HostBranchBatchKey, HostBranchBatchPhase, HostBranchClose, HostBranchLimits, HostBranchParent,
    HostBranchRoute, HostGuestBranch, JournalError,
};
use sandproof_protocol::{
    branching::{BranchAccessIdentity, BranchAccessSelector, BranchTarget},
    lifecycle::{AuthenticatedCenterContext, LifecycleAccessIdentity},
};

pub(super) struct BranchPins {
    selector: BranchAccessSelector,
    revoked: watch::Sender<bool>,
    drain: DataLease,
}

impl BranchPins {
    pub(super) fn revoke(&self) {
        self.revoked.send_replace(true);
    }

    pub(super) fn drained(&self) -> bool {
        *self.revoked.borrow() && self.revoked.receiver_count() == 0 && self.drain.drained()
    }
}

/// A ready native branch permit retaining the original VM gate and both drain
/// pins. A verified local branch observation is needed after each manager restart.
pub struct HostBranchPermit {
    identity: BranchAccessIdentity,
    context: AuthenticatedCenterContext,
    kind: DataKind,
    route: HostBranchRoute,
    revoked: watch::Receiver<bool>,
    drain: DataLease,
    _gate: std::fs::File,
}

impl DataAuthorization for HostBranchPermit {
    fn target(&self) -> DataTarget {
        DataTarget {
            vm: self.route.parent().vm(),
            generation: self.route.parent().generation(),
        }
    }

    fn revocation(&mut self) -> &mut watch::Receiver<bool> {
        &mut self.revoked
    }

    fn drain(&self) -> &DataLease {
        &self.drain
    }
}

fn route_error(error: JournalError) -> ApplicationError {
    match error {
        JournalError::Conflict | JournalError::UnsupportedStore => ApplicationError::NotFound,
        JournalError::MaintenanceCapacity => ApplicationError::QuotaExceeded,
        other => map_journal(other),
    }
}

/// The driver uses the same original VM authority and helper gate as the
/// original native data plane. Preparing the driver does not issue guest work.
pub struct HostBranchControl<B> {
    driver: B,
    _gate: std::fs::File,
}

impl<B> HostBranchControl<B> {
    #[must_use]
    pub const fn driver(&self) -> &B {
        &self.driver
    }
}

impl<H: ControllerHost> ControllerApplication<H> {
    /// Keep the exact native VM gate while the trusted driver talks to the guest.
    /// When appropriate, reserve the original plan after the protected driver
    /// has verified its checkpoint pin, under the same management lock.
    /// # Errors
    /// An unrecorded or changed plan, expired VM or unavailable gate refuses I/O.
    pub fn prepare_host_branch_control<B>(
        &self,
        parent: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        targets: &[BranchTarget],
        reserve: bool,
        prepare: impl FnOnce(&H, &HostBranchParent) -> Result<Option<B>, ApplicationError>,
    ) -> Result<Option<(HostBranchControl<B>, HostBranchBatchPhase)>, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let now = self.clock.now_ms()?;
        let (native, phase) = inner
            .journal
            .host_branch_control_parent(parent, context, key, targets, now)
            .map_err(route_error)?;
        if phase.is_none() && !reserve {
            return Err(ApplicationError::NotFound);
        }
        let gate = inner
            .host
            .acquire_data_gate(native.sandbox(), native.vm(), native.generation())?
            .ok_or(ApplicationError::Busy)?;
        let Some(driver) = prepare(&inner.host, &native)? else {
            return if phase.is_none() {
                Ok(None)
            } else {
                Err(ApplicationError::OutcomeUnknown)
            };
        };
        let phase = if let Some(original) = phase {
            original
        } else {
            inner
                .journal
                .reserve_host_branch_batch(parent, context, key, targets, now)
                .map_err(route_error)?
        };
        Ok(Some((
            HostBranchControl {
                driver,
                _gate: gate,
            },
            phase,
        )))
    }

    /// Configure the bounded native ledger from trusted startup configuration.
    /// # Errors
    /// Refuses a changed or invalid registered budget.
    pub fn configure_host_branching(
        &self,
        limits: HostBranchLimits,
    ) -> Result<(), ApplicationError> {
        self.inner
            .lock()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .configure_host_branching(limits)
            .map_err(route_error)
    }

    /// Persist the exact plan before any physical guest dispatch.
    /// # Errors
    /// Invalid, foreign, changed or over-budget plans refuse.
    pub fn reserve_host_branch_batch(
        &self,
        parent: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        targets: &[BranchTarget],
    ) -> Result<HostBranchBatchPhase, ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        inner
            .journal
            .reserve_host_branch_batch(parent, context, key, targets, self.clock.now_ms()?)
            .map_err(route_error)
    }

    /// Read the exact prior start plan for the trusted guest control path.
    /// Its credentials must stay on the original host and guest.
    /// # Errors
    /// Inactive, mismatched or unconfigured work refuses forwarding.
    pub fn pending_host_branch_guest_plan(
        &self,
        parent: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
    ) -> Result<Vec<HostGuestBranch>, ApplicationError> {
        self.inner
            .lock()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .pending_host_branch_guest_plan(parent, context, key, self.clock.now_ms()?)
            .map_err(route_error)
    }

    /// Save the complete ready observation supplied by the trusted provider.
    /// Callers must confirm exact guest listeners before marking data readiness.
    /// # Errors
    /// Partial, mismatched or stale observations do not publish.
    pub fn publish_host_branch_batch(
        &self,
        parent: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        ready: &[BranchTarget],
    ) -> Result<(), ApplicationError> {
        self.inner
            .lock()
            .map_err(|_| ApplicationError::Busy)?
            .journal
            .publish_host_branch_batch(parent, context, key, ready, self.clock.now_ms()?)
            .map_err(route_error)
    }

    /// Record a fresh local provider observation of the exact envd listener.
    /// The caller authenticates the original guest and validates the listener
    /// and original guest credential against its trusted control plan.
    /// # Errors
    /// No port or branch override, expired authority or spent pin is allowed.
    pub fn observe_host_branch_data_ready(
        &self,
        identity: &BranchAccessIdentity,
        context: &AuthenticatedCenterContext,
        observed: &BranchTarget,
    ) -> Result<(), ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let route = inner
            .journal
            .host_branch_data_route(identity, context, DataKind::Exec, self.clock.now_ms()?)
            .map_err(route_error)?;
        if identity.selector.branch != observed.sandbox || route.port().get() != observed.port.get()
        {
            return Err(ApplicationError::NotFound);
        }
        let parent = route.parent().sandbox().0;
        let handles = inner.branch_data_pins.entry(parent).or_default();
        if let Some(original) = handles.get(identity.selector.branch.as_str()) {
            if original.selector != identity.selector || *original.revoked.borrow() {
                return Err(ApplicationError::NotFound);
            }
            return Ok(());
        }
        handles.insert(
            identity.selector.branch.as_str().to_owned(),
            BranchPins {
                selector: identity.selector.clone(),
                revoked: watch::channel(false).0,
                drain: DataLease::default(),
            },
        );
        Ok(())
    }

    /// Admit a native branch route and pin the exact shared parent gate while
    /// taking both the branch and parent drain counts under the same lock.
    /// # Errors
    /// Stale, revoked, unobserved and unconfigured routes are denied.
    pub fn lifecycle_branch_data_authorization(
        &self,
        identity: &BranchAccessIdentity,
        context: &AuthenticatedCenterContext,
        kind: DataKind,
    ) -> Result<(HostBranchPermit, String), ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let route = inner
            .journal
            .host_branch_data_route(identity, context, kind, self.clock.now_ms()?)
            .map_err(route_error)?;
        let parent = route.parent().sandbox().0;
        let parent_drain = inner.data_drains.entry(parent).or_default().clone();
        let pins = inner
            .branch_data_pins
            .get(&parent)
            .and_then(|handles| handles.get(identity.selector.branch.as_str()))
            .filter(|pins| pins.selector == identity.selector && !*pins.revoked.borrow())
            .ok_or(ApplicationError::NotFound)?;
        let gate = inner
            .host
            .acquire_data_gate(
                route.parent().sandbox(),
                route.parent().vm(),
                route.parent().generation(),
            )?
            .ok_or(ApplicationError::Busy)?;
        let credential = route.token().to_owned();
        Ok((
            HostBranchPermit {
                identity: identity.clone(),
                context: context.clone(),
                kind,
                route,
                revoked: pins.revoked.subscribe(),
                drain: pins.drain.with_parent(&parent_drain),
                _gate: gate,
            },
            credential,
        ))
    }

    /// Recheck the exact original route and execute a tracked launch under one
    /// native management lock. The trusted callback must record the actual
    /// route cell before running its exact child, as for ordinary data relay.
    /// # Errors
    /// Refuses stale listener, credential, runtime or permission. No fallback.
    pub fn with_host_branch_dispatch<T>(
        &self,
        permit: &HostBranchPermit,
        launch: impl FnOnce(&H, &HostBranchRoute) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let current = inner
            .journal
            .host_branch_data_route(
                &permit.identity,
                &permit.context,
                permit.kind,
                self.clock.now_ms()?,
            )
            .map_err(route_error)?;
        if *permit.revoked.borrow()
            || current.parent() != permit.route.parent()
            || current.port() != permit.route.port()
            || !same_secret(current.token().as_bytes(), permit.route.token().as_bytes())
        {
            return Err(ApplicationError::NotFound);
        }
        launch(&inner.host, &current)
    }

    /// Start a tracked relay for the original already verified branch.
    /// # Errors
    /// Unknown authority, launch or capacity refuses without a fallback.
    pub fn spawn_host_branch_data_relay(
        &self,
        permit: &HostBranchPermit,
        control: bool,
    ) -> Result<TrackedRelayChild, ApplicationError> {
        self.with_host_branch_dispatch(permit, |host, route| {
            let reaper = RelayReaper::new()?;
            let parent = route.parent();
            host.spawn_branch_data_relay(
                parent.sandbox(),
                parent.vm(),
                parent.generation(),
                route.port(),
                control,
            )
            .map(|child| reaper.attach(child))
        })
    }

    /// Persist branch revocation before physical cleanup. A release confirmation
    /// also requires the original durable relay census and trusted guest exit.
    /// # Errors
    /// Scope mismatches, live pins or uncertain outcomes block port reuse.
    pub fn change_host_branch_close(
        &self,
        parent: &LifecycleAccessIdentity,
        context: &AuthenticatedCenterContext,
        key: &HostBranchBatchKey,
        close: HostBranchClose<'_>,
    ) -> Result<(), ApplicationError> {
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let local = inner
            .journal
            .lifecycle_access_local(parent, context)
            .map_err(route_error)?;
        let (confirm, scope) = match close {
            HostBranchClose::Begin(scope) => (false, scope),
            HostBranchClose::ConfirmGone(scope) => (true, scope),
        };
        let matches = |id: &str, pins: &BranchPins| {
            scope.is_none_or(|branch| branch.as_str() == id)
                && pins.selector.operation == key.operation
                && pins.selector.batch == key.batch
        };
        if confirm
            && inner
                .branch_data_pins
                .get(&local.sandbox.0)
                .is_some_and(|handles| {
                    handles
                        .iter()
                        .any(|(id, pins)| matches(id, pins) && !pins.drain.drained())
                })
        {
            return Err(ApplicationError::Busy);
        }
        let now_ms = self.clock.now_ms()?;
        if confirm {
            let (native, ports) = inner
                .journal
                .host_branch_closing_ports(parent, context, key, scope, now_ms)
                .map_err(route_error)?;
            for port in ports {
                if !inner.host.branch_data_gate_drained(
                    native.sandbox(),
                    native.vm(),
                    native.generation(),
                    port,
                )? {
                    return Err(ApplicationError::Busy);
                }
            }
        }
        let result = inner
            .journal
            .change_host_branch_close(parent, context, key, now_ms, close);
        if matches!(result, Err(JournalError::CommitUncertain(_))) {
            inner.revoke_branch_data(local.sandbox);
        } else if result.is_ok()
            && let Some(handles) = inner.branch_data_pins.get_mut(&local.sandbox.0)
        {
            handles.retain(|id, pins| {
                if !matches(id, pins) {
                    return true;
                }
                pins.revoke();
                !confirm
            });
        }
        result.map_err(route_error)
    }
}
