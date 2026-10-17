import SandproofFoundations.Funs
import SandboxModel.Boundaries.JsonCounter.Model

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandproofFoundationsRust.json
namespace SandboxModel.Boundaries.JsonCounter
open SandboxModel.Boundaries.JsonCounter

abbrev actual {D E : Type} :=
  @sandproof_protocol.internal.PositiveCounter.Insts.Serde_coreDeDeserialize.deserialize D E
abbrev primitive {D E : Type} := @U64.Insts.Serde_coreDeDeserialize.deserialize D E
abbrev invalid := sandproof_protocol.internal.InternalError.InvalidCounter
abbrev originalDisplay := sandproof_protocol.internal.InternalError.Insts.CoreFmtDisplay

theorem actual_schema {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D) :
    actual provider input = (do
      let decoded ← primitive provider input
      finish (provider.ErrorInst.custom originalDisplay invalid) decoded) := by
  unfold actual sandproof_protocol.internal.PositiveCounter.Insts.Serde_coreDeDeserialize.deserialize
  congr 1
  funext decoded
  cases decoded with
  | Err error =>
    simp [finish, core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
  | Ok value =>
    by_cases zero : value = 0#u64 <;>
      simp [finish, zero, originalDisplay, invalid,
        core.result.Result.Insts.CoreOpsTry.branch,
        sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from,
        core.result.Result.map_err,
        sandproof_protocol.internal.DeserializeDePositiveCounter.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleInternalErrorClause0_Error.call_once]

theorem nonzero_preserved {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D) (value : U64)
    (decoded : primitive provider input = .ok (.Ok value)) (nonzero : value ≠ 0#u64) :
    actual provider input = .ok (.Ok value) := by
  rw [actual_schema, decoded]
  simp [finish, nonzero]

theorem original_rejection_preserved {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D) (error : E)
    (decoded : primitive provider input = .ok (.Err error)) :
    actual provider input = .ok (.Err error) := by
  rw [actual_schema, decoded]
  simp [finish]

theorem zero_calls_original_custom {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D)
    (decoded : primitive provider input = .ok (.Ok 0#u64)) :
    actual provider input = (do
      let error ← provider.ErrorInst.custom originalDisplay invalid
      .ok (.Err error)) := by
  rw [actual_schema, decoded]
  simp [finish]

theorem original_failure_preserved {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D) (error : Error)
    (decoded : primitive provider input = .fail error) :
    actual provider input = .fail error := by
  rw [actual_schema, decoded]
  simp

theorem original_divergence_preserved {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D)
    (decoded : primitive provider input = .div) :
    actual provider input = .div := by
  rw [actual_schema, decoded]
  simp

theorem actual_safe {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D) :
    CounterSafe (actual provider input) := by
  intro value accepted
  rw [actual_schema] at accepted
  cases read : primitive provider input with
  | vis effect continuation => simp [read] at accepted
  | div => simp [read] at accepted
  | ret decoded =>
    apply finish_success_nonzero
      (provider.ErrorInst.custom originalDisplay invalid) decoded value
    simpa [read] using accepted

theorem wrapper_safe {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D) :
    CounterSafe (counter_decode provider input) := by
  exact actual_safe provider input

namespace Examples

-- A finite value-reader witness; not a serde_json parser implementation.
def unitError : serde_core.de.Error Unit := {
  coreerrorErrorInst := {
    fmtDebugInst := { fmt := fun _ formatter => .ok (.Ok (), formatter) }
    fmtDisplayInst := { fmt := fun _ formatter => .ok (.Ok (), formatter) }
  }
  custom := fun {_} _ _ => .ok ()
}

def valueReader : serde_core.de.Deserializer U64 Unit := {
  ErrorInst := unitError
  deserialize_any := fun {_ _} _ _ _ => .fail .panic
  deserialize_bool := fun {_ _} _ _ _ => .fail .panic
  deserialize_i8 := fun {_ _} _ _ _ => .fail .panic
  deserialize_i16 := fun {_ _} _ _ _ => .fail .panic
  deserialize_i32 := fun {_ _} _ _ _ => .fail .panic
  deserialize_i64 := fun {_ _} _ _ _ => .fail .panic
  deserialize_u8 := fun {_ _} _ _ _ => .fail .panic
  deserialize_u16 := fun {_ _} _ _ _ => .fail .panic
  deserialize_u32 := fun {_ _} _ _ _ => .fail .panic
  deserialize_u64 := fun {_ _} visitor value state => visitor.visit_u64 unitError state value
  deserialize_f32 := fun {_ _} _ _ _ => .fail .panic
  deserialize_f64 := fun {_ _} _ _ _ => .fail .panic
  deserialize_char := fun {_ _} _ _ _ => .fail .panic
  deserialize_str := fun {_ _} _ _ _ => .fail .panic
  deserialize_string := fun {_ _} _ _ _ => .fail .panic
  deserialize_bytes := fun {_ _} _ _ _ => .fail .panic
  deserialize_byte_buf := fun {_ _} _ _ _ => .fail .panic
  deserialize_option := fun {_ _} _ _ _ => .fail .panic
  deserialize_unit := fun {_ _} _ _ _ => .fail .panic
  deserialize_unit_struct := fun {_ _} _ _ _ _ => .fail .panic
  deserialize_newtype_struct := fun {_ _} _ _ _ _ => .fail .panic
  deserialize_seq := fun {_ _} _ _ _ => .fail .panic
  deserialize_tuple := fun {_ _} _ _ _ _ => .fail .panic
  deserialize_tuple_struct := fun {_ _} _ _ _ _ _ => .fail .panic
  deserialize_map := fun {_ _} _ _ _ => .fail .panic
  deserialize_struct := fun {_ _} _ _ _ _ _ => .fail .panic
  deserialize_enum := fun {_ _} _ _ _ _ _ => .fail .panic
  deserialize_identifier := fun {_ _} _ _ _ => .fail .panic
  deserialize_ignored_any := fun {_ _} _ _ _ => .fail .panic
}

theorem actual_accepts_one : actual valueReader 1#u64 = .ok (.Ok 1#u64) := by
  simp [actual,
    sandproof_protocol.internal.PositiveCounter.Insts.Serde_coreDeDeserialize.deserialize,
    U64.Insts.Serde_coreDeDeserialize.deserialize,
    serde_core.de.impls.DeserializeDeU64.deserialize.PrimitiveVisitor.Insts.Serde_coreDeVisitorDeU64,
    serde_core.de.impls.DeserializeDeU64.deserialize.PrimitiveVisitor.Insts.Serde_coreDeVisitorDeU64.visit_u64,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.map_err,
    sandproof_protocol.internal.DeserializeDePositiveCounter.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleInternalErrorClause0_Error.call_once,
    valueReader, unitError]

theorem actual_rejects_zero : actual valueReader 0#u64 = .ok (.Err ()) := by
  simp [actual,
    sandproof_protocol.internal.PositiveCounter.Insts.Serde_coreDeDeserialize.deserialize,
    U64.Insts.Serde_coreDeDeserialize.deserialize,
    serde_core.de.impls.DeserializeDeU64.deserialize.PrimitiveVisitor.Insts.Serde_coreDeVisitorDeU64,
    serde_core.de.impls.DeserializeDeU64.deserialize.PrimitiveVisitor.Insts.Serde_coreDeVisitorDeU64.visit_u64,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.map_err,
    sandproof_protocol.internal.DeserializeDePositiveCounter.deserialize.closure.Insts.CoreOpsFunctionFnOnceTupleInternalErrorClause0_Error.call_once,
    valueReader, unitError]

end Examples

theorem original_effect_preserved {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D)
    (effect : RustEffect.Input)
    (continuation : RustEffect.Output effect → Result (core.result.Result U64 E))
    (decoded : primitive provider input = .vis effect continuation) :
    actual provider input = .vis effect (fun result => do
      let value ← continuation result
      finish (provider.ErrorInst.custom originalDisplay invalid) value) := by
  simp [actual_schema, decoded]

end SandboxModel.Boundaries.JsonCounter
