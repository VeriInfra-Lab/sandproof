import SandboxModel.Proofs

/-! Persistence guarantees for the operation plans consumed by workers. -/

namespace SandboxModel

def OperationPlanPersistent (before after : World) : Prop :=
  ∀ key oldPlan, before.operationPlans key = some oldPlan →
    ∃ newPlan, after.operationPlans key = some newPlan

/-- Once a plan is sealed, no later protocol transition can exchange any of
    its resource identities or cleanup domain. -/
def SealedOperationPlanStable (before after : World) : Prop :=
  ∀ key oldPlan newPlan,
    before.operationPlans key = some oldPlan → oldPlan.sealed = true →
    after.operationPlans key = some newPlan → newPlan = oldPlan

def OperationPlanLedgerValid (world : World) : Prop :=
  ∀ key plan, world.operationPlans key = some plan →
    ∃ ledger,
      world.operations key = some ledger ∧
      plan.key = key ∧
      plan.epoch = ledger.operation.epoch

/-- Reserving startup identities also closes the failure-cleanup domain over
    those identities before the plan is sealed. -/
theorem reserveStartupResources_tracks_reserved_vm
    (found : (reserveStartupResources world key plan).operationPlans key =
      some reserved) :
    (⟨world.nextVmId⟩ : VmId) ∈ reserved.stopVms := by
  simp [reserveStartupResources, setOperationPlan, put] at found
  subst reserved
  by_cases present : (⟨world.nextVmId⟩ : VmId) ∈ plan.stopVms <;>
    simp [insertUnique, present]

theorem reserveStartupResources_tracks_reserved_slot
    (found : (reserveStartupResources world key plan).operationPlans key =
      some reserved) :
    (⟨world.nextSlotId⟩ : NetSlotId) ∈ reserved.closeSlots := by
  simp [reserveStartupResources, setOperationPlan, put] at found
  subst reserved
  by_cases present : (⟨world.nextSlotId⟩ : NetSlotId) ∈ plan.closeSlots <;>
    simp [insertUnique, present]

private theorem planPersistent_of_eq
    (same : after.operationPlans = before.operationPlans) :
    OperationPlanPersistent before after := by
  intro key oldPlan found
  exact ⟨oldPlan, by simpa [same] using found⟩

private theorem sealedPlanStable_of_eq
    (same : after.operationPlans = before.operationPlans) :
    SealedOperationPlanStable before after := by
  intro key oldPlan newPlan oldFound _ newFound
  rw [same] at newFound
  exact Option.some.inj (oldFound.symm.trans newFound).symm

theorem step_operationPlanPersistent
    (transition : Step world event next) :
    OperationPlanPersistent world next := by
  cases transition <;>
    simp_all [OperationPlanPersistent, beginCreate, beginPause, beginResume,
      beginKill, storeNewOperation, setOperationPlan, setOperation,
      supersedeCurrentOperation, updateOperation, setSandbox, setFilesystem,
      reserveStartupResources, reservePauseSnapshot, enqueue, put] <;>
    grind

set_option maxHeartbeats 1000000 in
theorem step_sealedOperationPlanStable
    (transition : Step world event next) :
    SealedOperationPlanStable world next := by
  cases transition <;>
    simp_all [SealedOperationPlanStable, beginCreate, beginPause, beginResume,
      beginKill, storeNewOperation, setOperationPlan, setOperation,
      supersedeCurrentOperation, updateOperation, setSandbox, setFilesystem,
      reserveStartupResources, reservePauseSnapshot, enqueue, put] <;>
    grind

end SandboxModel
