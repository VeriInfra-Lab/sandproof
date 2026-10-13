import SandboxModel.RustRefinement.WorkerHandler
import SandboxModel.Foundations.FiniteScan

/-! Historical names retain their signatures; reusable mathematics lives in Foundations. -/
namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std

abbrev vectorAllBody := @SandboxModel.Foundations.vectorAllBody
abbrev vectorAnyBody := @SandboxModel.Foundations.vectorAnyBody

theorem vectorAllLoop_refines (values : alloc.vec.Vec T) (predicate : T → Bool)
    (start : Std.Usize) (startBound : start.val ≤ values.val.length) :
    loop (vectorAllBody values predicate) start
      ⦃ found => found = (values.val.drop start.val).all predicate ⦄ :=
  SandboxModel.Foundations.vectorAllLoop_refines values predicate start startBound

theorem vectorAnyLoop_refines (values : alloc.vec.Vec T) (predicate : T → Bool)
    (start : Std.Usize) (startBound : start.val ≤ values.val.length) :
    loop (vectorAnyBody values predicate) start
      ⦃ found => found = (values.val.drop start.val).any predicate ⦄ :=
  SandboxModel.Foundations.vectorAnyLoop_refines values predicate start startBound

end SandboxModel.RustRefinement
