import SandboxModel.RustRefinement.Values
import SandboxModel.WorkerController

/-! Total value translations for the scheduler, worker, and stable-state
publication reducers.  Keeping these maps independent from the large handler
proofs makes the finite-machine differences reviewable in one place. -/

namespace SandboxModel.RustRefinement

def schedulerInput : SandboxControllerRust.scheduler.SchedulerInput →
    SandboxModel.Controller.SchedulerInput
  | .ReserveStartup key => .reserveStartup (operationKey key)
  | .ReservePauseSnapshot key => .reservePauseSnapshot (operationKey key)
  | .DispatchPrepare sid vm slot =>
      .dispatchPrepare (sandboxId sid) (vmId vm) (netSlotId slot)
  | .SealSnapshot sid snapshot manifest =>
      .sealSnapshot (sandboxId sid) (snapshotId snapshot)
        (snapshotManifestRef manifest)
  | .CommitSnapshot sid snapshot =>
      .commitSnapshot (sandboxId sid) (snapshotId snapshot)
  | .StopVm sid generation vm =>
      .stopVm (sandboxId sid) (u64ToNat generation) (vmId vm)
  | .CleanupVm sid generation vm =>
      .cleanupVm (sandboxId sid) (u64ToNat generation) (vmId vm)
  | .CleanupSlot sid generation slot =>
      .cleanupSlot (sandboxId sid) (u64ToNat generation) (netSlotId slot)

/-- Counter exhaustion is the scheduler's only concrete-machine rejection
    which has no image in the unbounded model. -/
def schedulerReject? : SandboxControllerRust.scheduler.SchedulerReject →
    Option SandboxModel.Controller.SchedulerReject
  | .ControllerDown => some .controllerDown
  | .SandboxNotFound => some .sandboxNotFound
  | .OperationUnavailable => some .operationUnavailable
  | .PlanUnavailable => some .planUnavailable
  | .VmNotFound => some .vmNotFound
  | .SlotNotFound => some .slotNotFound
  | .SnapshotNotFound => some .snapshotNotFound
  | .WrongPhase => some .wrongPhase
  | .Terminal => some .terminal
  | .PlanMismatch => some .planMismatch
  | .PlanAlreadySealed => some .planAlreadySealed
  | .PlanNotOpen => some .planNotOpen
  | .FenceNotInstalled => some .fenceNotInstalled
  | .ResourceAlreadyExists => some .resourceAlreadyExists
  | .RestoreSourceUnavailable => some .restoreSourceUnavailable
  | .PrerequisiteNotSucceeded => some .prerequisiteNotSucceeded
  | .ResourceMismatch => some .resourceMismatch
  | .ResourceNotPlanned => some .resourceNotPlanned
  | .ResourceNotReady => some .resourceNotReady
  | .ResourceStillCurrent => some .resourceStillCurrent
  | .AlreadyReclaimed => some .alreadyReclaimed
  | .CounterExhausted => none

def workerReject : SandboxControllerRust.worker.WorkerReject →
    SandboxModel.Controller.WorkerReject
  | .StaleAttempt => .staleAttempt
  | .NotQueued => .notQueued
  | .NotAwaiting => .notAwaiting
  | .SandboxNotFound => .sandboxNotFound
  | .ResourceNotFound => .resourceNotFound
  | .OperationNotFound => .operationNotFound
  | .PlanNotFound => .planNotFound
  | .WrongPhase => .wrongPhase
  | .Terminal => .terminal
  | .IdentityMismatch => .identityMismatch
  | .PlanMismatch => .planMismatch
  | .FenceNotReady => .fenceNotReady
  | .ResourceBusy => .resourceBusy
  | .ResultAlreadyPresent => .resultAlreadyPresent
  | .SourceUnavailable => .sourceUnavailable
  | .SnapshotNotReady => .snapshotNotReady
  | .ResourceStillCurrent => .resourceStillCurrent

def publicationInput : SandboxControllerRust.publication.PublicationInput →
    SandboxModel.Controller.PublicationInput
  | .Running sid => .running (sandboxId sid)
  | .Paused sid source => .paused (sandboxId sid) (restoreSource source)
  | .Destroyed sid => .destroyed (sandboxId sid)
  | .Failed sid failure => .failed (sandboxId sid) (failureKind failure)

def publicationReject : SandboxControllerRust.publication.PublicationReject →
    SandboxModel.Controller.PublicationReject
  | .ControllerDown => .controllerDown
  | .SandboxNotFound => .sandboxNotFound
  | .VmNotFound => .vmNotFound
  | .SlotNotFound => .slotNotFound
  | .SnapshotNotFound => .snapshotNotFound
  | .OperationUnavailable => .operationUnavailable
  | .WrongPhase => .wrongPhase
  | .TerminalMismatch => .terminalMismatch
  | .FenceNotInstalled => .fenceNotInstalled
  | .ResourceMismatch => .resourceMismatch
  | .ResourceNotReady => .resourceNotReady
  | .PolicyNotApplied => .policyNotApplied
  | .PrerequisiteNotSucceeded => .prerequisiteNotSucceeded
  | .ResourcesNotClosed => .resourcesNotClosed
  | .StopReceiptsMissing => .stopReceiptsMissing
  | .WrongOperationKind => .wrongOperationKind
  | .WrongOperationStatus => .wrongOperationStatus
  | .LedgerMismatch => .ledgerMismatch

end SandboxModel.RustRefinement
