import SandboxModel.Boundaries.BoundedWriter.Model
import SandproofBoundedWriter.Types

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.BoundedWriter.Refinement
open SandproofBoundedWriterRust
open sandproof_controller_service.template_registry.evidence

abbrev ActualWriter (B R : Type) := BoundedWriter B R
abbrev ActualWrite (B R : Type) := ActualWriter B R → Slice U8 →
  Result ((core.result.Result Usize core.io.error.Error) × ActualWriter B R)

def writerView {B R : Type} (view : B → WireBytes) (writer : ActualWriter B R) : WriterValue :=
  ⟨view writer.bytes, writer.limit.val, writer.exceeded⟩

def outcomeView : core.result.Result Usize core.io.error.Error → WriteOutcome
  | .Ok count => .accepted count.val
  | .Err _ => .refused

/-- Only the normal-result semantics of the exact library operations. -/
structure BufferLaws {B R : Type} (ops : BoundedBuffer B R) (view : B → WireBytes) : Prop where
  length_exact : ∀ b n, ops.len b = .ok n → n.val = (view b).length
  reserve_keeps_bytes : ∀ b n result next,
    ops.try_reserve_exact b n = .ok (result, next) → view next = view b
  append_exact : ∀ b chunk next,
    ops.extend_from_slice b chunk = .ok next → view next = view b ++ chunk.val
  finish_exact : ∀ b bytes, ops.into_bytes b = .ok bytes → bytes.val = view b

/-- A contract for an actual generated function, independent of its body. -/
def WriteContract {B R : Type} (view : B → WireBytes) (write : ActualWrite B R) : Prop :=
  ∀ before chunk outcome after, write before chunk = .ok (outcome, after) →
    WritePost (writerView view before) chunk.val (outcomeView outcome) (writerView view after)


def serializationView : core.result.Result Unit serde_json.error.Error → Except serde_json.error.Error Unit
  | .Ok value => .ok value
  | .Err error => .error error

def finishedView : core.result.Result (alloc.vec.Vec U8) sandproof_controller_service.JournalError →
    Option (Except (EncodeFailure serde_json.error.Error) WireBytes)
  | .Ok bytes => some (.ok bytes.val)
  | .Err (.MaintenanceCapacity) => some (.error .capacity)
  | .Err (.Json error) => some (.error (.json error))
  | .Err _ => none

def FinishContract {B R : Type} (view : B → WireBytes)
    (finish : ActualWriter B R → core.result.Result Unit serde_json.error.Error →
      Result (core.result.Result (alloc.vec.Vec U8) sandproof_controller_service.JournalError)) : Prop :=
  ∀ writer result output, finish writer result = .ok output →
    finishedView output = some (finishSpec (writerView view writer) (serializationView result))


/-- The actual caller may continue writing after any error. -/
inductive RustHistory {B R : Type} (write : ActualWrite B R) :
    ActualWriter B R → List (WireBytes × WriteOutcome) → ActualWriter B R → Prop where
  | nil (state) : RustHistory write state [] state
  | cons {initial middle final chunk outcome tail}
      (head : write initial chunk = .ok (outcome, middle))
      (rest : RustHistory write middle tail final) :
      RustHistory write initial ((chunk.val, outcomeView outcome) :: tail) final


structure ResponseLaws {P O : Type}
    (ops : sandproof_internal.server.ProofJsonResponseBuilder P O)
    (view : O → ResponseObservation) : Prop where
  empty_exact : ∀ response, ops.empty_error = .ok response →
    view response = ⟨500, false, true, []⟩
  success_exact : ∀ bytes response, ops.json_ok bytes = .ok response →
    view response = ⟨200, true, true, bytes.val⟩

def encodedView : core.result.Result (alloc.vec.Vec U8) sandproof_controller_service.JournalError →
    Except sandproof_controller_service.JournalError WireBytes
  | .Ok bytes => .ok bytes.val
  | .Err error => .error error


end SandboxModel.Boundaries.BoundedWriter.Refinement
