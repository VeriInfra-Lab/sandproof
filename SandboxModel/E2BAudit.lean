import SandboxModel.Proofs

/-!
A minimal conformance replay for an E2B routing-catalog race fixed in
`e2b-dev/infra@2a0c0af`. E2B's ExecutionID is represented by `Generation` here.
This file proves the abstract interleaving; the source audit establishes that
pre-fix `DeleteSandbox` used the same split read/check/delete shape.
-/

namespace SandboxModel.E2BAudit

abbrev RoutingCatalog := SandboxId → Option Generation

private def routeSid : SandboxId := ⟨700⟩
private def oldExecution : Generation := 1
private def newExecution : Generation := 2

private def emptyCatalog : RoutingCatalog := fun _ => none

private def putRoute (catalog : RoutingCatalog) (sid : SandboxId)
    (execution : Generation) : RoutingCatalog :=
  fun candidate => if candidate = sid then some execution else catalog candidate

/-- The vulnerable shape: the equality check has already happened, so the
    later delete no longer checks which execution currently owns the key. -/
private def deleteAfterEarlierCheck (catalog : RoutingCatalog)
    (sid : SandboxId) : RoutingCatalog :=
  fun candidate => if candidate = sid then none else catalog candidate

/-- The repaired shape: compare the current value and delete in one atomic step. -/
private def compareAndDelete (catalog : RoutingCatalog) (sid : SandboxId)
    (expectedExecution : Generation) : RoutingCatalog :=
  if catalog sid = some expectedExecution then
    deleteAfterEarlierCheck catalog sid
  else
    catalog

private def oldCatalog : RoutingCatalog :=
  putRoute emptyCatalog routeSid oldExecution

private def replacedCatalog : RoutingCatalog :=
  putRoute oldCatalog routeSid newExecution

/-- The old teardown read generation 1, generation 2 replaced it, and the old
    unconditional delete then removed generation 2's current route. -/
theorem split_check_delete_removes_new_execution :
    oldCatalog routeSid = some oldExecution ∧
    replacedCatalog routeSid = some newExecution ∧
    (deleteAfterEarlierCheck replacedCatalog routeSid) routeSid = none := by
  simp [oldCatalog, replacedCatalog, putRoute, deleteAfterEarlierCheck,
    routeSid, oldExecution, newExecution]

/-- Atomic compare-and-delete turns the same stale teardown into a stutter. -/
theorem atomic_compare_delete_preserves_new_execution :
    (compareAndDelete replacedCatalog routeSid oldExecution) routeSid =
      some newExecution := by
  simp [compareAndDelete, replacedCatalog, putRoute, routeSid,
    oldExecution, newExecution]

/-- A matching teardown still removes the execution it actually names. -/
theorem atomic_compare_delete_removes_matching_execution :
    (compareAndDelete oldCatalog routeSid oldExecution) routeSid = none := by
  simp [compareAndDelete, oldCatalog, putRoute, deleteAfterEarlierCheck,
    routeSid, oldExecution]

end SandboxModel.E2BAudit
