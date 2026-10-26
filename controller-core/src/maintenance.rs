//! Checked maintenance of the existing controller state. These transitions
//! perform no I/O and never constitute physical cleanup evidence themselves.

pub mod retirement;
pub mod slots;
mod unload_history;
pub mod released_execution;
pub mod windows;
pub use unload_history::released_unload_termination_history;

use crate::{
    Authority, ControllerDecision, ControllerEvent, FsId, OperationKey, Phase, SandboxId, Store,
    VmStatus, World,
};
use retirement::{RetainedOperation, RetireRequest, RetirementReject};
use slots::{HostSlotId, HostSlotRecord, SlotInput, SlotReject};
use windows::{WindowInput, WindowLimits, WindowReject, WindowState};

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct MaintenanceLimits {
    pub max_retained_results: u64,
    pub max_host_slots: u64,
    pub windows: WindowLimits,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaintenanceState {
    pub(crate) sandbox_frontier: Option<u64>,
    pub(crate) workspace_frontier: Option<u64>,
    pub(crate) retained: Store<OperationKey, RetainedOperation>,
    pub(crate) time_floor: u64,
    pub(crate) max_retained_results: u64,
    pub(crate) max_host_slots: u64,
    pub(crate) slots: Store<HostSlotId, HostSlotRecord>,
    pub(crate) windows: WindowState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MaintenanceInput {
    Enable {
        limits: MaintenanceLimits,
        now_ms: u64,
    },
    Retire(RetireRequest),
    ExpireResults {
        now_ms: u64,
    },
    Slot(SlotInput),
    Window(WindowInput),
    /// Original confirmed release, not a new physical fence acknowledgement.
    CloseReleasedExecution(released_execution::ReleasedExecution),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenanceEvent {
    Enabled,
    Retired(SandboxId),
    ResultsExpired,
    SlotChanged(HostSlotId),
    SlotChecked(HostSlotId),
    Window(windows::WindowEvent),
    ReleasedExecutionClosed(SandboxId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaintenanceReject {
    ControllerDown,
    Disabled,
    AlreadyEnabled,
    InvalidLimits,
    TimeRegression,
    ResourceMismatch,
    ResourcesNotReclaimed,
    UnresolvedWork,
    Retirement(RetirementReject),
    Slot(SlotReject),
    Window(WindowReject),
}

impl World {
    #[must_use]
    pub fn maintenance_state(&self) -> Option<&MaintenanceState> {
        self.maintenance.as_ref()
    }
}

impl MaintenanceState {
    /// Permanent identity barriers, including identities with expired results.
    #[must_use]
    pub const fn identity_frontiers(&self) -> (Option<u64>, Option<u64>) {
        (self.sandbox_frontier, self.workspace_frontier)
    }
    #[must_use]
    pub fn limits(&self) -> MaintenanceLimits {
        MaintenanceLimits {
            max_retained_results: self.max_retained_results,
            max_host_slots: self.max_host_slots,
            windows: self.windows.limits(),
        }
    }
    #[must_use]
    pub fn retained_result(&self, key: OperationKey) -> Option<&RetainedOperation> {
        self.retained.get(&key)
    }
    #[must_use]
    pub fn retained_results(&self) -> &[(OperationKey, RetainedOperation)] {
        self.retained.entries()
    }
    #[must_use]
    pub fn retained_count(&self) -> usize {
        self.retained.entries().len()
    }
    #[must_use]
    pub fn host_slot(&self, position: HostSlotId) -> Option<HostSlotRecord> {
        self.slots.get_copy(&position)
    }
    #[must_use]
    pub fn host_slots(&self) -> &[(HostSlotId, HostSlotRecord)] {
        self.slots.entries()
    }
    #[must_use]
    pub fn windows(&self) -> &WindowState {
        &self.windows
    }
    #[must_use]
    pub const fn time_floor(&self) -> u64 {
        self.time_floor
    }
}

pub(crate) fn fresh_identity(world: &World, sandbox: SandboxId, workspace: FsId) -> bool {
    world.maintenance.as_ref().is_none_or(|state| {
        state.sandbox_frontier.is_none_or(|last| sandbox.0 > last)
            && state
                .workspace_frontier
                .is_none_or(|last| workspace.0 > last)
    })
}

pub(crate) fn record_identity(world: &mut World, sandbox: SandboxId, workspace: FsId) {
    if let Some(state) = &mut world.maintenance {
        state.sandbox_frontier = Some(sandbox.0);
        state.workspace_frontier = Some(workspace.0);
    }
}

fn slot_position(input: SlotInput) -> HostSlotId {
    match input {
        SlotInput::Bind { position, .. }
        | SlotInput::BeginRelease { position, .. }
        | SlotInput::ConfirmRelease { position, .. }
        | SlotInput::CheckOccupied { position, .. } => position,
    }
}

fn apply_slot(world: &World, input: SlotInput) -> Result<World, MaintenanceReject> {
    let state = world
        .maintenance
        .as_ref()
        .ok_or(MaintenanceReject::Disabled)?;
    let position = slot_position(input);
    if position.0 >= state.max_host_slots {
        return Err(MaintenanceReject::Slot(SlotReject::OutOfRange));
    }
    match input {
        SlotInput::Bind { binding, .. } => {
            let record = world
                .sandbox_record(binding.sandbox)
                .ok_or(MaintenanceReject::ResourceMismatch)?;
            let operation = record
                .current_operation
                .ok_or(MaintenanceReject::ResourceMismatch)?;
            let plan = world
                .operation_plan(OperationKey {
                    sandbox: binding.sandbox,
                    operation: operation.id,
                })
                .ok_or(MaintenanceReject::ResourceMismatch)?;
            if record.owner != binding.owner
                || record.generation != binding.generation
                || record.terminal
                || record.phase != Phase::Starting
                || plan.vm != Some(binding.vm)
                || plan.slot != Some(binding.network)
                || plan.generation != binding.generation
                || !plan.sealed
            {
                return Err(MaintenanceReject::ResourceMismatch);
            }
            if state
                .slots
                .entries()
                .iter()
                .any(|(id, row)| *id != position && row.binding.vm == binding.vm)
            {
                return Err(MaintenanceReject::Slot(SlotReject::IdentityAlreadyBound));
            }
        }
        SlotInput::BeginRelease { binding, .. } | SlotInput::ConfirmRelease { binding, .. } => {
            let record = world
                .sandbox_record(binding.sandbox)
                .ok_or(MaintenanceReject::ResourceMismatch)?;
            let terminal = record.terminal
                && record.phase == Phase::Destroyed
                && record.desired == Authority::Closed
                && record.effective == Authority::Closed;
            let unloaded = paused_release_state(world, binding.sandbox, record);
            if record.owner != binding.owner || !(terminal || unloaded) {
                return Err(MaintenanceReject::ResourcesNotReclaimed);
            }
            if world.vm_record(binding.vm).is_some_and(|vm| {
                vm.owner != binding.owner
                    || vm.sandbox != binding.sandbox
                    || vm.generation != binding.generation
                    || vm.status != VmStatus::Reclaimed
            }) || world.slot_record(binding.network).is_some_and(|slot| {
                slot.owner != binding.owner
                    || slot.sandbox != binding.sandbox
                    || slot.vm != binding.vm
                    || slot.active
                    || !slot.reclaimed
            }) {
                return Err(MaintenanceReject::ResourcesNotReclaimed);
            }
            if world.effect_ids().iter().any(|id| {
                id.command().sandbox() == binding.sandbox
                    && matches!(
                        world.effect_status(id),
                        Some(crate::EffectStatus::Dispatched | crate::EffectStatus::OutcomeUnknown)
                    )
            }) {
                return Err(MaintenanceReject::UnresolvedWork);
            }
        }
        SlotInput::CheckOccupied { .. } => {}
    }
    let row =
        slots::reduce_slot(state.host_slot(position), input).map_err(MaintenanceReject::Slot)?;
    let mut next = world.clone();
    next.maintenance
        .as_mut()
        .ok_or(MaintenanceReject::Disabled)?
        .slots
        .put(position, row);
    Ok(next)
}

fn sandbox_frontier(world: &World) -> Option<u64> {
    let mut frontier = None;
    for (id, _) in world.sandboxes.entries() {
        if frontier.is_none_or(|previous| id.0 > previous) {
            frontier = Some(id.0);
        }
    }
    frontier
}

fn workspace_frontier(world: &World) -> Option<u64> {
    let mut frontier = None;
    for (id, _) in world.filesystems.entries() {
        if frontier.is_none_or(|previous| id.0 > previous) {
            frontier = Some(id.0);
        }
    }
    frontier
}

/// Reduce maintenance through the same pure controller boundary as lifecycle
/// work. Service/host integration must durably validate external certificates.
///
/// # Errors
/// Rejection returns no successor and preserves all supplied state.
pub fn handle_maintenance(
    world: &World,
    input: MaintenanceInput,
) -> Result<ControllerDecision, MaintenanceReject> {
    if !world.controller_up {
        return Err(MaintenanceReject::ControllerDown);
    }
    let (next, event) = match input {
        MaintenanceInput::Enable { limits, now_ms } => {
            if world.maintenance.is_some() {
                return Err(MaintenanceReject::AlreadyEnabled);
            }
            if limits.max_retained_results == 0
                || limits.max_host_slots == 0
                || usize::try_from(limits.max_retained_results).is_err()
                || usize::try_from(limits.max_host_slots).is_err()
            {
                return Err(MaintenanceReject::InvalidLimits);
            }
            let mut next = world.clone();
            next.maintenance = Some(MaintenanceState {
                sandbox_frontier: sandbox_frontier(world),
                workspace_frontier: workspace_frontier(world),
                retained: Store::default(),
                time_floor: now_ms,
                max_retained_results: limits.max_retained_results,
                max_host_slots: limits.max_host_slots,
                slots: Store::default(),
                windows: windows::reduce_windows(
                    &WindowState::new(limits.windows),
                    WindowInput::AdvanceTime { now_ms },
                )
                .map_err(MaintenanceReject::Window)?
                .next,
            });
            (next, MaintenanceEvent::Enabled)
        }
        MaintenanceInput::Retire(request) => {
            let sandbox = request.sandbox;
            (
                retirement::retire(world, request).map_err(MaintenanceReject::Retirement)?,
                MaintenanceEvent::Retired(sandbox),
            )
        }
        MaintenanceInput::ExpireResults { now_ms } => {
            let mut next = world.clone();
            let state = next
                .maintenance
                .as_mut()
                .ok_or(MaintenanceReject::Disabled)?;
            if now_ms < state.time_floor {
                return Err(MaintenanceReject::TimeRegression);
            }
            state.windows =
                windows::reduce_windows(&state.windows, WindowInput::AdvanceTime { now_ms })
                    .map_err(MaintenanceReject::Window)?
                    .next;
            state.time_floor = now_ms;
            state.retained.retain(|_, value| value.expires_at > now_ms);
            (next, MaintenanceEvent::ResultsExpired)
        }
        MaintenanceInput::CloseReleasedExecution(basis) => (
            released_execution::close(world, basis)?,
            MaintenanceEvent::ReleasedExecutionClosed(basis.key.sandbox),
        ),
        MaintenanceInput::Slot(input) => {
            let event = if matches!(input, SlotInput::CheckOccupied { .. }) {
                MaintenanceEvent::SlotChecked(slot_position(input))
            } else {
                MaintenanceEvent::SlotChanged(slot_position(input))
            };
            (apply_slot(world, input)?, event)
        }
        MaintenanceInput::Window(input) => {
            let state = world
                .maintenance
                .as_ref()
                .ok_or(MaintenanceReject::Disabled)?;
            let decision = windows::reduce_windows(&state.windows, input)
                .map_err(MaintenanceReject::Window)?;
            let mut next = world.clone();
            let state = next
                .maintenance
                .as_mut()
                .ok_or(MaintenanceReject::Disabled)?;
            state.time_floor = decision.next.time_floor();
            state.windows = decision.next;
            (next, MaintenanceEvent::Window(decision.event))
        }
    };
    Ok(ControllerDecision {
        event: ControllerEvent::Maintenance(event),
        next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Actor, ControllerInput, CreateRequest, FilePolicyRef, NetworkPolicy, OperationId,
        PolicyDigest, Request, TenantId, handle_controller, initial_world,
    };

    fn create(world: &World, id: u64, workspace: u64) -> Result<World, crate::ControllerReject> {
        handle_controller(
            world,
            ControllerInput::Request(Request::Create(CreateRequest {
                actor: Actor::Tenant(TenantId(1)),
                owner: TenantId(1),
                sandbox: SandboxId(id),
                workspace: FsId(workspace),
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(1),
                },
                policy: NetworkPolicy::DenyAll,
                operation: OperationId(1),
            })),
        )
        .map(|d| d.next)
    }

    fn limits() -> MaintenanceLimits {
        MaintenanceLimits {
            max_retained_results: 10,
            max_host_slots: 2,
            windows: WindowLimits::new(2, 100).unwrap(),
        }
    }

    #[test]
    fn legacy_order_is_preserved_until_explicit_boundary() {
        let old = create(&initial_world(), 10, 20).unwrap();
        let old = create(&old, 0, 0).unwrap();
        let enabled = handle_maintenance(
            &old,
            MaintenanceInput::Enable {
                limits: limits(),
                now_ms: 0,
            },
        )
        .unwrap()
        .next;
        assert_eq!(enabled.sandbox_records(), old.sandbox_records());
        assert_eq!(enabled.effect_ids(), old.effect_ids());
        assert!(create(&enabled, 11, 21).is_ok());
        assert!(create(&enabled, 9, 21).is_err());
        assert!(create(&enabled, 11, 19).is_err());
        assert!(
            handle_maintenance(
                &enabled,
                MaintenanceInput::Enable {
                    limits: limits(),
                    now_ms: 0
                }
            )
            .is_err()
        );
    }

    #[test]
    fn frontier_preserves_max_without_wrapping_or_rejecting_enable() {
        let old = create(&initial_world(), u64::MAX, u64::MAX).unwrap();
        let enabled = handle_maintenance(
            &old,
            MaintenanceInput::Enable {
                limits: limits(),
                now_ms: 1,
            },
        )
        .unwrap()
        .next;
        assert!(!fresh_identity(&enabled, SandboxId(0), FsId(0)));
        assert!(!fresh_identity(
            &enabled,
            SandboxId(u64::MAX),
            FsId(u64::MAX)
        ));
        assert_eq!(
            handle_maintenance(&enabled, MaintenanceInput::ExpireResults { now_ms: 0 }),
            Err(MaintenanceReject::TimeRegression)
        );
    }

    #[test]
    fn frontier_queries_match_original_max_and_enable_rejection_order() {
        for identifiers in [
            vec![],
            vec![(0, 0)],
            vec![(10, 30), (0, 2), (20, 0)],
            vec![(u64::MAX, 2), (2, u64::MAX), (1, 1)],
        ] {
            let mut world = initial_world();
            for (id, workspace) in identifiers {
                world = create(&world, id, workspace).unwrap();
            }
            let before = world.clone();
            let expected_sandbox = world.sandboxes.entries().iter().map(|(id, _)| id.0).max();
            let expected_workspace = world.filesystems.entries().iter().map(|(id, _)| id.0).max();
            let enable = MaintenanceInput::Enable {
                limits: limits(),
                now_ms: 1,
            };
            let enabled = handle_maintenance(&world, enable.clone()).unwrap().next;
            let state = enabled.maintenance_state().unwrap();
            assert_eq!(state.sandbox_frontier, expected_sandbox);
            assert_eq!(state.workspace_frontier, expected_workspace);
            assert_eq!(world, before);
            let mut invalid = limits();
            invalid.max_host_slots = 0;
            let invalid = MaintenanceInput::Enable {
                limits: invalid,
                now_ms: 1,
            };
            assert_eq!(
                handle_maintenance(&world, invalid.clone()),
                Err(MaintenanceReject::InvalidLimits)
            );
            assert_eq!(
                handle_maintenance(&enabled, invalid.clone()),
                Err(MaintenanceReject::AlreadyEnabled)
            );
            let mut down = enabled;
            down.controller_up = false;
            assert_eq!(
                handle_maintenance(&down, invalid),
                Err(MaintenanceReject::ControllerDown)
            );
        }
    }
}

#[cfg(test)]
#[path = "maintenance/slot_integration_tests.rs"]
mod slot_integration_tests;

pub(crate) fn paused_release_state(
    world: &World,
    sandbox: SandboxId,
    record: &crate::SandboxRecord,
) -> bool {
    !record.terminal
        && record.current_vm.is_none()
        && record.net_slot.is_none()
        && record.phase == Phase::Paused
        && record.durable_snapshot.is_some_and(|source| {
            let authority = Authority::Paused { sandbox, source };
            record.desired == authority
                && record.effective == authority
                && record.current_operation.is_some_and(|operation| {
                    world
                        .operation_plan(crate::OperationKey {
                            sandbox,
                            operation: operation.id,
                        })
                        .is_some_and(|plan| {
                            plan.target_snapshot == Some(source)
                                && plan
                                    .capture
                                    .is_some_and(crate::capture::CaptureHandoff::may_stop_original)
                        })
                })
        })
}
