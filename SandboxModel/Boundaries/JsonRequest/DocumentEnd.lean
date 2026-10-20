import SandboxModel.Boundaries.JsonRequest.Callbacks

set_option autoImplicit false
open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
namespace SandboxModel.Boundaries.JsonRequest

-- Fixed serde_json::de::from_trait first runs Deserialize, then calls end().
-- The captured parser and cursor belong to the concrete A6 binding. A value
-- error does not start the end-check; already performed effects remain.
def finishDocument {T E : Type} (decoded : Result (core.result.Result T E))
    (endCheck : Unit → Result (core.result.Result Unit E)) :
    Result (core.result.Result T E) := do
  let value ← decoded
  match value with
  | .Err error => .ok (.Err error)
  | .Ok payload => do
    let ended ← endCheck ()
    match ended with
    | .Err error => .ok (.Err error)
    | .Ok _ => .ok (.Ok payload)

theorem decoder_error_precedes_end {T E : Type}
    (decoded : Result (core.result.Result T E))
    (endCheck : Unit → Result (core.result.Result Unit E)) (error : E)
    (failed : decoded = .ok (.Err error)) :
    finishDocument decoded endCheck = .ok (.Err error) := by
  simp [finishDocument, failed]

theorem document_value_preserved {T E : Type}
    (decoded : Result (core.result.Result T E))
    (endCheck : Unit → Result (core.result.Result Unit E)) (value : T)
    (read : decoded = .ok (.Ok value)) (ended : endCheck () = .ok (.Ok ())) :
    finishDocument decoded endCheck = .ok (.Ok value) := by
  simp [finishDocument, read, ended]

theorem end_error_after_value {T E : Type}
    (decoded : Result (core.result.Result T E))
    (endCheck : Unit → Result (core.result.Result Unit E)) (value : T) (error : E)
    (read : decoded = .ok (.Ok value)) (ended : endCheck () = .ok (.Err error)) :
    finishDocument decoded endCheck = .ok (.Err error) := by
  simp [finishDocument, read, ended]

theorem document_failure_preserved {T E : Type}
    (decoded : Result (core.result.Result T E))
    (endCheck : Unit → Result (core.result.Result Unit E)) (error : Error)
    (failed : decoded = .fail error) : finishDocument decoded endCheck = .fail error := by
  simp [finishDocument, failed]

theorem document_divergence_preserved {T E : Type}
    (decoded : Result (core.result.Result T E))
    (endCheck : Unit → Result (core.result.Result Unit E))
    (diverges : decoded = .div) : finishDocument decoded endCheck = .div := by
  simp [finishDocument, diverges]

theorem document_effects_preserved {T E : Type}
    (effect : RustEffect.Input)
    (continuation : RustEffect.Output effect → Result (core.result.Result T E))
    (endCheck : Unit → Result (core.result.Result Unit E)) :
    finishDocument (.vis effect continuation) endCheck =
      .vis effect (fun response => finishDocument (continuation response) endCheck) := by
  simp [finishDocument]

theorem zero_counter_precedes_document_end {D E : Type}
    (provider : serde_core.de.Deserializer D E) (input : D) (error : E)
    (endCheck : Unit → Result (core.result.Result Unit E))
    (read : JsonCounter.primitive provider input = .ok (.Ok 0#u64))
    (custom : provider.ErrorInst.custom JsonCounter.originalDisplay JsonCounter.invalid = .ok error) :
    finishDocument (JsonCounter.actual provider input) endCheck = .ok (.Err error) := by
  rw [JsonCounter.zero_calls_original_custom provider input read]
  simp [custom, finishDocument]

theorem invalid_identifier_precedes_document_end {D E : Type}
    (kind : OpaqueKind) (provider : serde_core.de.Deserializer D E)
    (input : D) (text : String) (error : E)
    (endCheck : Unit → Result (core.result.Result Unit E))
    (read : JsonIdentifiers.primitive provider input = .ok (.Ok text))
    (bounded : IdentifierLibrary.StringRepresentable text) (invalid : ¬opaqueValid text)
    (custom : provider.ErrorInst.custom JsonIdentifiers.originalDisplay
      sandproof_protocol.ContractError.InvalidIdentifier = .ok error) :
    finishDocument (JsonIdentifiers.actual kind provider input) endCheck = .ok (.Err error) := by
  rw [JsonIdentifiers.invalid_read_rejected kind provider input text read bounded invalid]
  simp [custom, finishDocument]

end SandboxModel.Boundaries.JsonRequest
