import SandboxModel.RustRefinement.Values

/-!
The executable initialization boundary.

Unlike `emptyRustWorld`, `SandboxControllerRust.model.initial_world` is the
Aeneas translation of the real Rust function `model::initial_world`.  These
facts connect that executable constructor to the already established complete
world representation.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

@[simp] theorem generated_initial_world :
    SandboxControllerRust.model.initial_world = ok emptyRustWorld := by
  simp [SandboxControllerRust.model.initial_world,
    SandboxControllerRust.model.World.Insts.CoreDefaultDefault.default,
    SandboxControllerRust.store.Store.Insts.CoreDefaultDefault.default,
    emptyRustWorld, emptyStore]

/-- The state returned by the actual Rust initialization function represents
    the mathematical initial state, field for field. -/
theorem initial_world_refines :
    SandboxControllerRust.model.initial_world
      ⦃ rust =>
        @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
          effectIdDecidableEq valueRelations rust SandboxModel.emptyWorld ⦄ := by
  rw [generated_initial_world]
  simpa only [Aeneas.Std.WP.spec_ok] using emptyWorld_rep

end SandboxModel.RustRefinement
