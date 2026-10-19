import SandboxModel.Boundaries.DataDrain.Representation
import SandproofDataDrain.Funs

namespace SandboxModel.Boundaries.DataDrain
open Aeneas Aeneas.Std

theorem source_routes_dropped_exact (lease : SandproofDataDrainRust.data_drain.DataLease) :
    SandproofDataDrainRust.data_drain.DataLease.routes_dropped lease =
      .ok (routesDropped (view lease)) := by
  simp [SandproofDataDrainRust.data_drain.DataLease.routes_dropped,
    alloc.sync.Arc.strong_count, routesDropped, view]
  constructor
  · intro same
    exact congrArg UScalar.val same
  · intro same
    exact UScalar.eq_of_val_eq same

theorem source_read_routes_dropped_exact (lease : SandproofDataDrainRust.data_drain.DataLease) :
    SandproofDataDrainRust.read_routes_dropped lease = .ok (routesDropped (view lease)) := by
  simpa only [SandproofDataDrainRust.read_routes_dropped] using source_routes_dropped_exact lease

theorem source_drained_exact (lease : SandproofDataDrainRust.data_drain.DataLease) :
    SandproofDataDrainRust.data_drain.DataLease.drained lease = .ok (drained (view lease)) := by
  rw [SandproofDataDrainRust.data_drain.DataLease.drained, source_routes_dropped_exact]
  by_cases count : lease.unknown.observedStrongCount.val = 1 <;>
    cases value : lease.unknown.observedValue <;>
      simp [routesDropped, drained, view, alloc.sync.Arc.Insts.CoreOpsDerefDeref.deref,
        core.sync.atomic.AtomicBool.load, count, value]

theorem source_read_drained_exact (lease : SandproofDataDrainRust.data_drain.DataLease) :
    SandproofDataDrainRust.read_drained lease = .ok (drained (view lease)) := by
  simpa only [SandproofDataDrainRust.read_drained] using source_drained_exact lease

theorem source_drained_iff (lease : SandproofDataDrainRust.data_drain.DataLease) :
    SandproofDataDrainRust.read_drained lease = .ok true ↔
      (view lease).strongCount = 1 ∧ (view lease).unknown = false := by
  rw [source_read_drained_exact]
  simpa only [Result.ok.injEq] using policy_exact (view lease)

theorem source_unknown_not_drained (lease : SandproofDataDrainRust.data_drain.DataLease)
    (unknown : (view lease).unknown = true) :
    SandproofDataDrainRust.read_drained lease = .ok false := by
  rw [source_read_drained_exact]
  simp [drained, unknown]

theorem source_live_pin_not_drained (lease : SandproofDataDrainRust.data_drain.DataLease)
    (live : (view lease).strongCount ≠ 1) :
    SandproofDataDrainRust.read_drained lease = .ok false := by
  rw [source_read_drained_exact]
  simp [drained, routesDropped, live]

theorem source_read_safe : RustReadSafe SandproofDataDrainRust.read_drained := by
  intro lease answer returned
  rw [source_read_drained_exact] at returned
  have same : drained (view lease) = answer := by
    simpa only [Result.ok.injEq] using returned
  rw [← same]
  exact policy_safe (view lease)

/-- A representable normal case. Real Arc construction is tested separately. -/
theorem source_eligible_accepted (object : Nat) :
    SandproofDataDrainRust.read_drained ⟨⟨object, 1#usize, false⟩⟩ = .ok true := by
  rw [source_read_drained_exact]
  rfl

end SandboxModel.Boundaries.DataDrain
