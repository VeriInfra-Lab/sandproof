import SandboxModel.Lifecycle.Protocol.ReadyFactBindings

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.observation
namespace SandboxModel.Stage1Protocol.CreateEquality

theorem facts_eq (a b : CurrentReadyFacts) :
    CurrentReadyFacts.Insts.CoreCmpPartialEqCurrentReadyFacts.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a
  cases b
  simp [CurrentReadyFacts.Insts.CoreCmpPartialEqCurrentReadyFacts.eq,
    Replay.ok_and, CurrentReadyFacts.mk.injEq]

theorem reason_eq (a b : CreateRefusalReason) :
    CreateRefusalReason.Insts.CoreCmpPartialEqCreateRefusalReason.eq a b =
      .ok (by classical exact decide (a = b)) := by
  cases a
  cases b
  simp [CreateRefusalReason.Insts.CoreCmpPartialEqCreateRefusalReason.eq]

theorem readiness_eq (a b : CurrentCreateReadiness) :
    CurrentCreateReadiness.Insts.CoreCmpPartialEqCurrentCreateReadiness.eq a b =
      .ok (by classical exact decide (a = b)) := by
  classical
  cases a <;> cases b <;>
    simp [CurrentCreateReadiness.Insts.CoreCmpPartialEqCurrentCreateReadiness.eq,
      CurrentCreateReadiness.read_discriminant, facts_eq, reason_eq]

theorem observation_eq (a b : CreateObservation) :
    create_observation_equal a b = .ok (by classical exact decide (a = b)) := by
  classical
  cases a
  cases b
  simp [create_observation_equal, CreateObservation.Insts.CoreCmpPartialEqCreateObservation.eq,
    Replay.accepted_eq, readiness_eq, Replay.ok_and, CreateObservation.mk.injEq,
    and_left_comm]

theorem observation_eq_iff (a b : CreateObservation) :
    create_observation_equal a b = .ok true ↔ a = b := by
  classical
  simp [observation_eq]

end SandboxModel.Stage1Protocol.CreateEquality

#audit_controller_proof SandboxModel.Stage1Protocol.CreateEquality.facts_eq
#audit_controller_proof SandboxModel.Stage1Protocol.CreateEquality.reason_eq
#audit_controller_proof SandboxModel.Stage1Protocol.CreateEquality.readiness_eq
#audit_controller_proof SandboxModel.Stage1Protocol.CreateEquality.observation_eq
#audit_controller_proof SandboxModel.Stage1Protocol.CreateEquality.observation_eq_iff
