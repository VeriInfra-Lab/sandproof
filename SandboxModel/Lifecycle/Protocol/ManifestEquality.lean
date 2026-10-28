import SandproofStage1Manifest.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1ManifestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.identity
open sandproof_protocol.lifecycle.manifest
namespace SandboxModel.Stage1Protocol.ManifestEquality

theorem pairs_eq {T : Type} [DecidableEq T] (xs ys : List T)
    (lengths : xs.length = ys.length) :
    List.allM (fun (p : T × T) => (.ok (decide (p.1 = p.2)) : Result Bool))
      (List.zip xs ys) = .ok (decide (xs = ys)) := by
  induction xs generalizing ys with
  | nil => cases ys <;> simp_all [pure]
  | cons x xs ih =>
    cases ys with
    | nil => simp at lengths
    | cons y ys =>
      have rest : xs.length = ys.length := by simpa using lengths
      by_cases same : x = y <;> simp [pure, ih ys rest, same]

theorem digest_eq (a b : Array U8 32#usize) :
    core.array.equality.PartialEqArray.eq core.cmp.PartialEqU8 a b =
      .ok (decide (a = b)) := by
  have scalar : ∀ x y : U8, (x.val = y.val) ↔ x = y := by intro x y; scalar_tac
  simp [core.array.equality.PartialEqArray.eq, scalar, pairs_eq a.val b.val (by simp),
    ← Array.eq_iff]

theorem ok_and (p : Prop) [Decidable p] (q : Bool) :
    (if p then (Result.ok q : Result Bool) else .ok false) = .ok (decide p && q) := by
  by_cases hp : p <;> simp [hp]

theorem capture_eq (a b : CaptureBinding) :
    CaptureBinding.Insts.CoreCmpPartialEqCaptureBinding.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  rcases a with ⟨⟨av, ⟨ad, an, ag⟩, ast, ⟨akd, ako, akw, akn⟩, ae⟩, ai, ac⟩
  rcases b with ⟨⟨bv, ⟨bd, bn, bg⟩, bst, ⟨bkd, bko, bkw, bkn⟩, be⟩, bi, bc⟩
  cases ae <;> cases be <;>
    simp [CaptureBinding.Insts.CoreCmpPartialEqCaptureBinding.eq,
      OperationBinding.Insts.CoreCmpPartialEqOperationBinding.eq,
      NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq,
      OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
      ExpectedSession.Insts.CoreCmpPartialEqExpectedSession.eq,
      ExpectedSession.read_discriminant,
      SessionId.Insts.CoreCmpPartialEqSessionId.eq,
      IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq,
      CaptureId.Insts.CoreCmpPartialEqCaptureId.eq,
      RequestWindowId.Insts.CoreCmpPartialEqRequestWindowId.eq,
      RequestSequence.Insts.CoreCmpPartialEqRequestSequence.eq,
      PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
      sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq,
      sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
      sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
      sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      lift, core.cmp.impls.PartialEqU64.eq, ok_and, Bool.and_assoc]

theorem drive_eq (a b : WritableDrive) :
    WritableDrive.Insts.CoreCmpPartialEqWritableDrive.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [WritableDrive.Insts.CoreCmpPartialEqWritableDrive.eq,
    sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    ok_and, WritableDrive.mk.injEq]

theorem drives_eq (a b : alloc.vec.Vec WritableDrive) :
    alloc.vec.partial_eq.PartialEqVec.eq WritableDrive.Insts.CoreCmpPartialEqWritableDrive a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  by_cases lengths : a.length = b.length
  · simp [alloc.vec.partial_eq.PartialEqVec.eq, lengths, drive_eq,
      pairs_eq a.val b.val lengths, ← alloc.vec.Vec.eq_iff]
  · have different : a ≠ b := by intro same; subst b; exact lengths rfl
    simp [alloc.vec.partial_eq.PartialEqVec.eq, lengths, different]

theorem profile_eq (a b : VmCompatibility) :
    VmCompatibility.Insts.CoreCmpPartialEqVmCompatibility.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [VmCompatibility.Insts.CoreCmpPartialEqVmCompatibility.eq,
    sandproof_protocol.lifecycle.digest.ContentDigest.Insts.CoreCmpPartialEqContentDigest.eq,
    digest_eq, drives_eq, ok_and, VmCompatibility.mk.injEq]

end SandboxModel.Stage1Protocol.ManifestEquality

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestEquality.pairs_eq
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestEquality.digest_eq
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestEquality.ok_and
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestEquality.capture_eq
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestEquality.drive_eq
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestEquality.drives_eq
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestEquality.profile_eq
