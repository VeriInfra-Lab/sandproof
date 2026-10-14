import SandboxModel.Local.State.World

namespace SandboxModel.Local.State
open Aeneas Aeneas.Std

def scheduler.SchedulerInput.view : SandproofNetworkRust.sandbox_controller_core.scheduler.SchedulerInput → scheduler.SchedulerInput
  | .ReserveStartup v0 => .ReserveStartup (ValueView.view v0)
  | .ReservePauseSnapshot v0 => .ReservePauseSnapshot (ValueView.view v0)
  | .DispatchPrepare v0 v1 v2 => .DispatchPrepare (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .SealSnapshot v0 v1 v2 => .SealSnapshot (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .CommitSnapshot v0 v1 => .CommitSnapshot (ValueView.view v0) (ValueView.view v1)
  | .StopVm v0 v1 v2 => .StopVm (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .AbortPrepare v0 v1 v2 v3 => .AbortPrepare (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3)
  | .CleanupVm v0 v1 v2 => .CleanupVm (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .CleanupSlot v0 v1 v2 => .CleanupSlot (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)

theorem scheduler.SchedulerInput.view_injective : Function.Injective scheduler.SchedulerInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [scheduler.SchedulerInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.scheduler.SchedulerInput scheduler.SchedulerInput :=
  ⟨scheduler.SchedulerInput.view, scheduler.SchedulerInput.view_injective⟩

def request.CompensationRequest.view (value : SandproofNetworkRust.sandbox_controller_core.request.CompensationRequest) : request.CompensationRequest := {
  sandbox := ValueView.view value.sandbox,
  failure := ValueView.view value.failure,
  evidence := ValueView.view value.evidence
}

theorem request.CompensationRequest.view_injective : Function.Injective request.CompensationRequest.view := by
  intro a b h
  cases a; cases b; simp_all [request.CompensationRequest.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.request.CompensationRequest request.CompensationRequest :=
  ⟨request.CompensationRequest.view, request.CompensationRequest.view_injective⟩

def request.QueryRequest.view (value : SandproofNetworkRust.sandbox_controller_core.request.QueryRequest) : request.QueryRequest := {
  actor := ValueView.view value.actor,
  sandbox := ValueView.view value.sandbox,
  operation := ValueView.view value.operation
}

theorem request.QueryRequest.view_injective : Function.Injective request.QueryRequest.view := by
  intro a b h
  cases a; cases b; simp_all [request.QueryRequest.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.request.QueryRequest request.QueryRequest :=
  ⟨request.QueryRequest.view, request.QueryRequest.view_injective⟩

def request.ReplayRequest.view (value : SandproofNetworkRust.sandbox_controller_core.request.ReplayRequest) : request.ReplayRequest := {
  actor := ValueView.view value.actor,
  sandbox := ValueView.view value.sandbox,
  operation := ValueView.view value.operation,
  kind := ValueView.view value.kind,
  input := ValueView.view value.input
}

theorem request.ReplayRequest.view_injective : Function.Injective request.ReplayRequest.view := by
  intro a b h
  cases a; cases b; simp_all [request.ReplayRequest.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.request.ReplayRequest request.ReplayRequest :=
  ⟨request.ReplayRequest.view, request.ReplayRequest.view_injective⟩

def request.KillRequest.view (value : SandproofNetworkRust.sandbox_controller_core.request.KillRequest) : request.KillRequest := {
  actor := ValueView.view value.actor,
  sandbox := ValueView.view value.sandbox,
  operation := ValueView.view value.operation
}

theorem request.KillRequest.view_injective : Function.Injective request.KillRequest.view := by
  intro a b h
  cases a; cases b; simp_all [request.KillRequest.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.request.KillRequest request.KillRequest :=
  ⟨request.KillRequest.view, request.KillRequest.view_injective⟩

def request.ResumeRequest.view (value : SandproofNetworkRust.sandbox_controller_core.request.ResumeRequest) : request.ResumeRequest := {
  actor := ValueView.view value.actor,
  sandbox := ValueView.view value.sandbox,
  operation := ValueView.view value.operation
}

theorem request.ResumeRequest.view_injective : Function.Injective request.ResumeRequest.view := by
  intro a b h
  cases a; cases b; simp_all [request.ResumeRequest.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.request.ResumeRequest request.ResumeRequest :=
  ⟨request.ResumeRequest.view, request.ResumeRequest.view_injective⟩

def request.PauseRequest.view (value : SandproofNetworkRust.sandbox_controller_core.request.PauseRequest) : request.PauseRequest := {
  actor := ValueView.view value.actor,
  sandbox := ValueView.view value.sandbox,
  operation := ValueView.view value.operation
}

theorem request.PauseRequest.view_injective : Function.Injective request.PauseRequest.view := by
  intro a b h
  cases a; cases b; simp_all [request.PauseRequest.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.request.PauseRequest request.PauseRequest :=
  ⟨request.PauseRequest.view, request.PauseRequest.view_injective⟩

def request.CreateRequest.view (value : SandproofNetworkRust.sandbox_controller_core.request.CreateRequest) : request.CreateRequest := {
  actor := ValueView.view value.actor,
  owner := ValueView.view value.owner,
  sandbox := ValueView.view value.sandbox,
  workspace := ValueView.view value.workspace,
  file_policy := ValueView.view value.file_policy,
  policy := ValueView.view value.policy,
  operation := ValueView.view value.operation
}

theorem request.CreateRequest.view_injective : Function.Injective request.CreateRequest.view := by
  intro a b h
  cases a; cases b; simp_all [request.CreateRequest.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.request.CreateRequest request.CreateRequest :=
  ⟨request.CreateRequest.view, request.CreateRequest.view_injective⟩

def request.Request.view : SandproofNetworkRust.sandbox_controller_core.request.Request → request.Request
  | .Create v0 => .Create (ValueView.view v0)
  | .Pause v0 => .Pause (ValueView.view v0)
  | .Resume v0 => .Resume (ValueView.view v0)
  | .Kill v0 => .Kill (ValueView.view v0)
  | .Replay v0 => .Replay (ValueView.view v0)
  | .Query v0 => .Query (ValueView.view v0)
  | .Compensate v0 => .Compensate (ValueView.view v0)

theorem request.Request.view_injective : Function.Injective request.Request.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [request.Request.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.request.Request request.Request :=
  ⟨request.Request.view, request.Request.view_injective⟩

def recovery_monitor.RuntimeInput.view : SandproofNetworkRust.sandbox_controller_core.recovery_monitor.RuntimeInput → recovery_monitor.RuntimeInput
  | .Crash => .Crash
  | .Recover => .Recover
  | .DuplicateOrLateDelivery v0 => .DuplicateOrLateDelivery (ValueView.view v0)

theorem recovery_monitor.RuntimeInput.view_injective : Function.Injective recovery_monitor.RuntimeInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [recovery_monitor.RuntimeInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.recovery_monitor.RuntimeInput recovery_monitor.RuntimeInput :=
  ⟨recovery_monitor.RuntimeInput.view, recovery_monitor.RuntimeInput.view_injective⟩

def publication.PublicationInput.view : SandproofNetworkRust.sandbox_controller_core.publication.PublicationInput → publication.PublicationInput
  | .Running v0 => .Running (ValueView.view v0)
  | .Paused v0 v1 => .Paused (ValueView.view v0) (ValueView.view v1)
  | .Destroyed v0 => .Destroyed (ValueView.view v0)
  | .Failed v0 v1 => .Failed (ValueView.view v0) (ValueView.view v1)

theorem publication.PublicationInput.view_injective : Function.Injective publication.PublicationInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [publication.PublicationInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.publication.PublicationInput publication.PublicationInput :=
  ⟨publication.PublicationInput.view, publication.PublicationInput.view_injective⟩

def maintenance.MaintenanceLimits.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.MaintenanceLimits) : maintenance.MaintenanceLimits := {
  max_retained_results := ValueView.view value.max_retained_results,
  max_host_slots := ValueView.view value.max_host_slots,
  windows := ValueView.view value.windows
}

theorem maintenance.MaintenanceLimits.view_injective : Function.Injective maintenance.MaintenanceLimits.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.MaintenanceLimits.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.MaintenanceLimits maintenance.MaintenanceLimits :=
  ⟨maintenance.MaintenanceLimits.view, maintenance.MaintenanceLimits.view_injective⟩

def maintenance.windows.WindowInput.view : SandproofNetworkRust.sandbox_controller_core.maintenance.windows.WindowInput → maintenance.windows.WindowInput
  | .Issue v0 v1 v2 => .Issue (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .Close v0 v1 v2 => .Close (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .AdvanceTime v0 => .AdvanceTime (ValueView.view v0)
  | .Admit v0 v1 v2 => .Admit (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)

theorem maintenance.windows.WindowInput.view_injective : Function.Injective maintenance.windows.WindowInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [maintenance.windows.WindowInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.windows.WindowInput maintenance.windows.WindowInput :=
  ⟨maintenance.windows.WindowInput.view, maintenance.windows.WindowInput.view_injective⟩

def maintenance.slots.SlotInput.view : SandproofNetworkRust.sandbox_controller_core.maintenance.slots.SlotInput → maintenance.slots.SlotInput
  | .Bind v0 v1 v2 => .Bind (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .BeginRelease v0 v1 => .BeginRelease (ValueView.view v0) (ValueView.view v1)
  | .ConfirmRelease v0 v1 => .ConfirmRelease (ValueView.view v0) (ValueView.view v1)
  | .CheckOccupied v0 v1 => .CheckOccupied (ValueView.view v0) (ValueView.view v1)

theorem maintenance.slots.SlotInput.view_injective : Function.Injective maintenance.slots.SlotInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [maintenance.slots.SlotInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.slots.SlotInput maintenance.slots.SlotInput :=
  ⟨maintenance.slots.SlotInput.view, maintenance.slots.SlotInput.view_injective⟩

def maintenance.retirement.QuiescenceStatus.view : SandproofNetworkRust.sandbox_controller_core.maintenance.retirement.QuiescenceStatus → maintenance.retirement.QuiescenceStatus
  | .Unconfirmed => .Unconfirmed
  | .Confirmed => .Confirmed

theorem maintenance.retirement.QuiescenceStatus.view_injective : Function.Injective maintenance.retirement.QuiescenceStatus.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [maintenance.retirement.QuiescenceStatus.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.retirement.QuiescenceStatus maintenance.retirement.QuiescenceStatus :=
  ⟨maintenance.retirement.QuiescenceStatus.view, maintenance.retirement.QuiescenceStatus.view_injective⟩

def maintenance.retirement.QuiescenceCertificate.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.retirement.QuiescenceCertificate) : maintenance.retirement.QuiescenceCertificate := {
  sandbox := ValueView.view value.sandbox,
  owner := ValueView.view value.owner,
  epoch := ValueView.view value.epoch,
  operation := ValueView.view value.operation,
  workspace := ValueView.view value.workspace,
  worker_drained := ValueView.view value.worker_drained,
  data_connections_drained := ValueView.view value.data_connections_drained,
  host_resources_reclaimed := ValueView.view value.host_resources_reclaimed,
  external_pins_released := ValueView.view value.external_pins_released
}

theorem maintenance.retirement.QuiescenceCertificate.view_injective : Function.Injective maintenance.retirement.QuiescenceCertificate.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.retirement.QuiescenceCertificate.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.retirement.QuiescenceCertificate maintenance.retirement.QuiescenceCertificate :=
  ⟨maintenance.retirement.QuiescenceCertificate.view, maintenance.retirement.QuiescenceCertificate.view_injective⟩

def maintenance.retirement.RetireRequest.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.retirement.RetireRequest) : maintenance.retirement.RetireRequest := {
  sandbox := ValueView.view value.sandbox,
  owner := ValueView.view value.owner,
  epoch := ValueView.view value.epoch,
  operation := ValueView.view value.operation,
  retain_until := ValueView.view value.retain_until,
  quiescence := ValueView.view value.quiescence
}

theorem maintenance.retirement.RetireRequest.view_injective : Function.Injective maintenance.retirement.RetireRequest.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.retirement.RetireRequest.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.retirement.RetireRequest maintenance.retirement.RetireRequest :=
  ⟨maintenance.retirement.RetireRequest.view, maintenance.retirement.RetireRequest.view_injective⟩

def maintenance.MaintenanceInput.view : SandproofNetworkRust.sandbox_controller_core.maintenance.MaintenanceInput → maintenance.MaintenanceInput
  | .Enable v0 v1 => .Enable (ValueView.view v0) (ValueView.view v1)
  | .Retire v0 => .Retire (ValueView.view v0)
  | .ExpireResults v0 => .ExpireResults (ValueView.view v0)
  | .Slot v0 => .Slot (ValueView.view v0)
  | .Window v0 => .Window (ValueView.view v0)

theorem maintenance.MaintenanceInput.view_injective : Function.Injective maintenance.MaintenanceInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [maintenance.MaintenanceInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.MaintenanceInput maintenance.MaintenanceInput :=
  ⟨maintenance.MaintenanceInput.view, maintenance.MaintenanceInput.view_injective⟩

def effect.EffectInput.view : SandproofNetworkRust.sandbox_controller_core.effect.EffectInput → effect.EffectInput
  | .Dispatch v0 => .Dispatch (ValueView.view v0)
  | .Receipt v0 v1 => .Receipt (ValueView.view v0) (ValueView.view v1)
  | .Timeout v0 => .Timeout (ValueView.view v0)
  | .Retry v0 v1 => .Retry (ValueView.view v0) (ValueView.view v1)
  | .FinalizeFailure v0 v1 => .FinalizeFailure (ValueView.view v0) (ValueView.view v1)
  | .RetireActivation v0 v1 => .RetireActivation (ValueView.view v0) (ValueView.view v1)

theorem effect.EffectInput.view_injective : Function.Injective effect.EffectInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [effect.EffectInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.effect.EffectInput effect.EffectInput :=
  ⟨effect.EffectInput.view, effect.EffectInput.view_injective⟩

def controller.ControllerInput.view : SandproofNetworkRust.sandbox_controller_core.controller.ControllerInput → controller.ControllerInput
  | .Maintenance v0 => .Maintenance (ValueView.view v0)
  | .Request v0 => .Request (ValueView.view v0)
  | .Effect v0 => .Effect (ValueView.view v0)
  | .Scheduler v0 => .Scheduler (ValueView.view v0)
  | .Worker v0 => .Worker (ValueView.view v0)
  | .Publication v0 => .Publication (ValueView.view v0)
  | .RpcTimeout v0 => .RpcTimeout (ValueView.view v0)
  | .Access v0 => .Access (ValueView.view v0)
  | .Runtime v0 => .Runtime (ValueView.view v0)

theorem controller.ControllerInput.view_injective : Function.Injective controller.ControllerInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [controller.ControllerInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.controller.ControllerInput controller.ControllerInput :=
  ⟨controller.ControllerInput.view, controller.ControllerInput.view_injective⟩

def maintenance.slots.SlotReject.view : SandproofNetworkRust.sandbox_controller_core.maintenance.slots.SlotReject → maintenance.slots.SlotReject
  | .OutOfRange => .OutOfRange
  | .BindingMismatch => .BindingMismatch
  | .NotFree => .NotFree
  | .NotOccupied => .NotOccupied
  | .NotReleasing => .NotReleasing
  | .StaleIdentity => .StaleIdentity
  | .IdentityAlreadyBound => .IdentityAlreadyBound

theorem maintenance.slots.SlotReject.view_injective : Function.Injective maintenance.slots.SlotReject.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [maintenance.slots.SlotReject.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.slots.SlotReject maintenance.slots.SlotReject :=
  ⟨maintenance.slots.SlotReject.view, maintenance.slots.SlotReject.view_injective⟩

def recovery_monitor.RuntimeEvent.view : SandproofNetworkRust.sandbox_controller_core.recovery_monitor.RuntimeEvent → recovery_monitor.RuntimeEvent
  | .ControllerCrash => .ControllerCrash
  | .ControllerRecover => .ControllerRecover
  | .DuplicateOrLateDelivery v0 => .DuplicateOrLateDelivery (ValueView.view v0)

theorem recovery_monitor.RuntimeEvent.view_injective : Function.Injective recovery_monitor.RuntimeEvent.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [recovery_monitor.RuntimeEvent.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.recovery_monitor.RuntimeEvent recovery_monitor.RuntimeEvent :=
  ⟨recovery_monitor.RuntimeEvent.view, recovery_monitor.RuntimeEvent.view_injective⟩

def recovery_monitor.RuntimeReject.view : SandproofNetworkRust.sandbox_controller_core.recovery_monitor.RuntimeReject → recovery_monitor.RuntimeReject
  | .ControllerAlreadyDown => .ControllerAlreadyDown
  | .ControllerAlreadyUp => .ControllerAlreadyUp
  | .EffectNotFound => .EffectNotFound

theorem recovery_monitor.RuntimeReject.view_injective : Function.Injective recovery_monitor.RuntimeReject.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [recovery_monitor.RuntimeReject.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.recovery_monitor.RuntimeReject recovery_monitor.RuntimeReject :=
  ⟨recovery_monitor.RuntimeReject.view, recovery_monitor.RuntimeReject.view_injective⟩

def recovery_monitor.RuntimeDecision.view (value : SandproofNetworkRust.sandbox_controller_core.recovery_monitor.RuntimeDecision) : recovery_monitor.RuntimeDecision := {
  event := ValueView.view value.event,
  next := ValueView.view value.next
}

theorem recovery_monitor.RuntimeDecision.view_injective : Function.Injective recovery_monitor.RuntimeDecision.view := by
  intro a b h
  cases a; cases b; simp_all [recovery_monitor.RuntimeDecision.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.recovery_monitor.RuntimeDecision recovery_monitor.RuntimeDecision :=
  ⟨recovery_monitor.RuntimeDecision.view, recovery_monitor.RuntimeDecision.view_injective⟩

end SandboxModel.Local.State
