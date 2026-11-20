import SandboxModel.Lifecycle.Protocol.ValidationModel

/-! Decoded value contracts only. Source authority, actual materials and monotonic
renewal admission remain separate provider obligations. -/

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.observation sandproof_protocol.lifecycle.unload
namespace SandboxModel.Stage1Protocol.ReadyFacts

def Timing (facts : CurrentReadyFacts) : Prop :=
  facts.started_at_ms ≤ facts.startup_deadline_ms ∧
  facts.started_at_ms ≤ facts.observed_at_ms ∧ facts.observed_at_ms < facts.end_at_ms ∧
  facts.end_at_ms ≤ facts.max_end_at_ms

def CreateMatches (facts : CurrentReadyFacts) (intent : AcceptedLifecycleIntent) : Prop :=
  match intent.request.operation with
  | .Create create _ =>
    facts.startup_deadline_ms = create.startup_deadline_ms ∧
    facts.started_at_ms ≤ facts.startup_deadline_ms ∧
    U64.checked_add facts.started_at_ms create.timeout_ms = some facts.end_at_ms ∧
    U64.checked_add facts.started_at_ms create.max_lifetime_ms = some facts.max_end_at_ms ∧
    facts.started_at_ms ≤ facts.observed_at_ms ∧ facts.observed_at_ms < facts.end_at_ms ∧
    facts.end_at_ms ≤ facts.max_end_at_ms
  | _ => False

def ResumeSource (facts : CurrentReadyFacts) (original : AcceptedLifecycleIntent)
    (source : UnloadedFacts) : Prop :=
  match original.request.operation with
  | .Resume incarnation capture _ digest =>
    source.source_incarnation = incarnation ∧ source.capture = capture ∧
    source.manifest_digest = digest ∧ facts.started_at_ms = source.started_at_ms ∧
    facts.max_end_at_ms = source.max_end_at_ms ∧ facts.startup_deadline_ms = source.startup_deadline_ms
  | _ => False

def ResumedMatches (facts : CurrentReadyFacts) (original : AcceptedLifecycleIntent)
    (source : UnloadedFacts) : Prop :=
  ResumeSource facts original source ∧ facts.end_at_ms = source.end_at_ms ∧ Timing facts

def RenewedMatches (facts : CurrentReadyFacts) (original current : AcceptedLifecycleIntent)
    (source : UnloadedFacts) : Prop :=
  match original.request.operation, current.request.operation with
  | .Resume _ _ target _, .Renew incarnation deadline extend =>
    0 ≤ deadline.val ∧ ResumeSource facts original source ∧ incarnation = target ∧ Timing facts ∧
      (if extend then deadline.val ≤ (facts.end_at_ms.val : Int) else (facts.end_at_ms.val : Int) = deadline.val)
  | _, _ => False

def FullCreateContract (run : CurrentReadyFacts → AcceptedLifecycleIntent → Result Bool) : Prop :=
  ∀ facts intent, run facts intent = .ok (by classical exact decide (CreateMatches facts intent))

def FullResumedContract
    (run : CurrentReadyFacts → AcceptedLifecycleIntent → UnloadedFacts → Result Bool) : Prop :=
  ∀ facts intent source, run facts intent source = .ok (by classical exact decide (ResumedMatches facts intent source))

def FullRenewedContract
    (run : CurrentReadyFacts → AcceptedLifecycleIntent → AcceptedLifecycleIntent → UnloadedFacts → Result Bool) : Prop :=
  ∀ facts original current source,
    run facts original current source = .ok (by classical exact decide (RenewedMatches facts original current source))

end SandboxModel.Stage1Protocol.ReadyFacts
