import SandboxModel.Lifecycle.Protocol.Library
import SandboxModel.Lifecycle.Protocol.ReplayContract
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std SandproofStage1RequestRust
namespace SandboxModel.Stage1Protocol.Replay

/-! Full structural equality of the actual request extraction. Strings use the
registered A6 model; arrays and vectors use the fixed Aeneas byte comparisons.
This theorem is about value equality, not authentication or durable replay state. -/

theorem bytes_eq (xs ys : List U8) (h : xs.length = ys.length) :
    List.allM (fun (p : U8 × U8) => (.ok (decide (p.1 = p.2)) : Result Bool))
      (List.zip xs ys) = .ok (decide (xs = ys)) := by
  induction xs generalizing ys with
  | nil => cases ys <;> simp_all [pure]
  | cons x xs ih =>
    cases ys with
    | nil => simp at h
    | cons y ys =>
      have hlen : xs.length = ys.length := by simpa using h
      by_cases heq : x = y <;> simp [pure, ih ys hlen, heq]

theorem array_eq (a b : Array U8 32#usize) :
    core.array.equality.PartialEqArray.eq core.cmp.PartialEqU8 a b =
      .ok (decide (a = b)) := by
  have heq : ∀ x y : U8, (x.val = y.val) ↔ x = y := by intro x y; scalar_tac
  simp [core.array.equality.PartialEqArray.eq, heq, bytes_eq a.val b.val (by simp),
    ← Array.eq_iff]

theorem vec_eq (a b : alloc.vec.Vec U8) :
    alloc.vec.partial_eq.PartialEqVec.eq core.cmp.PartialEqU8 a b =
      .ok (decide (a = b)) := by
  by_cases h : a.length = b.length
  · simp [alloc.vec.partial_eq.PartialEqVec.eq, h, bytes_eq a.val b.val h,
      ← alloc.vec.Vec.eq_iff]
  · have hab : a ≠ b := by intro e; subst b; exact h rfl
    simp [alloc.vec.partial_eq.PartialEqVec.eq, h, hab]

theorem ok_and (p : Prop) [Decidable p] (q : Bool) :
    (if p then (Result.ok q : Result Bool) else .ok false) =
      .ok (decide p && q) := by
  by_cases hp : p <;> simp [hp]

open sandproof_protocol.internal.lifecycle
open sandproof_protocol.internal
open sandproof_protocol.lifecycle.identity
open sandproof_protocol.lifecycle.request

theorem resources_eq (a b : Resources) :
    Resources.Insts.CoreCmpPartialEqResources.eq a b = .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [Resources.Insts.CoreCmpPartialEqResources.eq, ok_and, Resources.mk.injEq]

theorem create_eq (a b : CreateBinding) :
    CreateBinding.Insts.CoreCmpPartialEqCreateBinding.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [CreateBinding.Insts.CoreCmpPartialEqCreateBinding.eq,
    sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    array_eq, resources_eq, vec_eq, ok_and, CreateBinding.mk.injEq,
    Bool.and_assoc, Bool.and_left_comm, Bool.and_comm]

theorem node_eq (a b : NodeIdentity) :
    NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq,
    PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    ok_and, NodeIdentity.mk.injEq]

theorem key_eq (a b : OperationKey) :
    OperationKey.Insts.CoreCmpPartialEqOperationKey.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
    RequestWindowId.Insts.CoreCmpPartialEqRequestWindowId.eq,
    RequestSequence.Insts.CoreCmpPartialEqRequestSequence.eq,
    PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    ok_and, OperationKey.mk.injEq]

theorem expected_eq (a b : ExpectedSession) :
    ExpectedSession.Insts.CoreCmpPartialEqExpectedSession.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a <;> cases b <;>
    simp [ExpectedSession.Insts.CoreCmpPartialEqExpectedSession.eq,
      ExpectedSession.read_discriminant, lift, core.cmp.impls.PartialEqU64.eq]

theorem binding_eq (a b : OperationBinding) :
    OperationBinding.Insts.CoreCmpPartialEqOperationBinding.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [OperationBinding.Insts.CoreCmpPartialEqOperationBinding.eq,
    node_eq, key_eq, expected_eq,
    SessionId.Insts.CoreCmpPartialEqSessionId.eq,
    sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    ok_and, OperationBinding.mk.injEq]

theorem operation_eq (a b : LifecycleOperation) :
    LifecycleOperation.Insts.CoreCmpPartialEqLifecycleOperation.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a <;> cases b <;>
    simp [LifecycleOperation.Insts.CoreCmpPartialEqLifecycleOperation.eq,
      LifecycleOperation.read_discriminant, create_eq,
      IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq,
      CaptureId.Insts.CoreCmpPartialEqCaptureId.eq,
      PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
      sandproof_protocol.lifecycle.digest.ContentDigest.Insts.CoreCmpPartialEqContentDigest.eq,
      array_eq, ok_and,
      core.cmp.impls.PartialEqI64.eq, core.cmp.impls.PartialEqBool.eq,
      lift, Bool.and_assoc, Bool.and_left_comm, Bool.and_comm]

theorem request_eq (a b : LifecycleRequest) :
    LifecycleRequest.Insts.CoreCmpPartialEqLifecycleRequest.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [LifecycleRequest.Insts.CoreCmpPartialEqLifecycleRequest.eq,
    binding_eq, operation_eq, ok_and, LifecycleRequest.mk.injEq]


open sandproof_protocol.lifecycle.authority

theorem center_eq (a b : CenterIdentity) :
    CenterIdentity.Insts.CoreCmpPartialEqCenterIdentity.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [CenterIdentity.Insts.CoreCmpPartialEqCenterIdentity.eq,
    CenterId.Insts.CoreCmpPartialEqCenterId.eq,
    sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    ok_and, CenterIdentity.mk.injEq]

theorem retention_eq (a b : RetentionBinding) :
    RetentionBinding.Insts.CoreCmpPartialEqRetentionBinding.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [RetentionBinding.Insts.CoreCmpPartialEqRetentionBinding.eq,
    RetentionWindowId.Insts.CoreCmpPartialEqRetentionWindowId.eq,
    PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq,
    ok_and, RetentionBinding.mk.injEq]

theorem accepted_eq (a b : AcceptedLifecycleIntent) :
    AcceptedLifecycleIntent.Insts.CoreCmpPartialEqAcceptedLifecycleIntent.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a; cases b
  simp [AcceptedLifecycleIntent.Insts.CoreCmpPartialEqAcceptedLifecycleIntent.eq,
    request_eq, center_eq, retention_eq, ok_and, AcceptedLifecycleIntent.mk.injEq,
    Bool.and_assoc, Bool.and_left_comm, Bool.and_comm]

theorem replay_complete : FullReplayContract replay_matches := by
  intro request original
  simp [replay_matches, LifecycleRequest.replay_matches, request_eq]

theorem replay_accepts_iff (request original : LifecycleRequest) :
    replay_matches request original = .ok true ↔ request = original := by
  classical
  rw [replay_complete request original]
  simp

theorem replay_rejects_iff (request original : LifecycleRequest) :
    replay_matches request original = .ok false ↔ request ≠ original := by
  classical
  rw [replay_complete request original]
  simp
end SandboxModel.Stage1Protocol.Replay

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.bytes_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.array_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.vec_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.ok_and

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.resources_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.create_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.node_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.key_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.expected_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.binding_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.operation_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.request_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.center_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.retention_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.accepted_eq

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.replay_complete

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.replay_accepts_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Replay.replay_rejects_iff
