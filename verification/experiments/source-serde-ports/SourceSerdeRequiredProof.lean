import SourceSerdeRequiredView
import SourceSerdeRequiredWrong
import SourceSerdeDecodeProof

open Aeneas Aeneas.Std SandproofStage1RequestRust
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SourceSerdeRequiredProof
open SourceSerdeRequiredView
noncomputable section

/-- The same complete generic forwarding contract for both actual Rust bodies. -/
def RequiredContract (actual : ∀ (_ : Ports) {D T E : Type},
    serde_core.de.Deserializer D E → serde_core.de.Deserialize T → D →
      Result (core.result.Result (Option T) E)) : Prop :=
  ∀ (ports : Ports) {D T E : Type} (inputDictionary : serde_core.de.Deserializer D E)
    (valueDictionary : serde_core.de.Deserialize T) (input : D),
    actual ports inputDictionary valueDictionary input =
      ports.optionDeserialize valueDictionary inputDictionary input

theorem original_required_option_forwards_full_value : RequiredContract @required := by
  intro ports D T E inputDictionary valueDictionary input
  rfl

theorem original_required_option_returns_same_none_or_error (ports : Ports) {D T E : Type}
    (inputDictionary : serde_core.de.Deserializer D E) (valueDictionary : serde_core.de.Deserialize T)
    (input : D) (reply : core.result.Result (Option T) E)
    (provided : ports.optionDeserialize valueDictionary inputDictionary input = .ok reply) :
    required ports inputDictionary valueDictionary input = .ok reply :=
  (original_required_option_forwards_full_value ports inputDictionary valueDictionary input).trans provided

theorem actual_wrong_discards_successful_some (ports : Ports) {D T E : Type}
    (inputDictionary : serde_core.de.Deserializer D E) (valueDictionary : serde_core.de.Deserialize T)
    (input : D) (value : T)
    (provided : ports.optionDeserialize valueDictionary inputDictionary input = .ok (.Ok (some value))) :
    SourceSerdeRequiredWrong.required ports inputDictionary valueDictionary input = .ok (.Ok none) := by
  simp [SourceSerdeRequiredWrong.required,
    SourceSerdeRequiredWrong.controller.lifecycle_inbox.source_work.required_option, provided]

/-- Finite generic provider witness, not a model of concrete serde_json. The
    actual native fixture separately uses the real JSON deserializer. -/
def samplePorts : Ports where
  optionDeserialize valueDictionary inputDictionary input := do
    let result ← valueDictionary.deserialize inputDictionary input
    match result with
    | .Ok value => .ok (.Ok (some value))
    | .Err error => .ok (.Err error)

def sampleValue : serde_core.de.Deserialize U64 where
  deserialize _ _ := .ok (.Ok 7#u64)

def sampleError : serde_core.de.Error Unit where
  coreerrorErrorInst := ⟨⟨fun _ _ => .fail .panic⟩, ⟨fun _ _ => .fail .panic⟩⟩
  custom := by intros; exact .ok ()

def sampleInput : serde_core.de.Deserializer Unit Unit where
  ErrorInst := sampleError
  deserialize_any := by intros; exact .fail .panic
  deserialize_bool := by intros; exact .fail .panic
  deserialize_i8 := by intros; exact .fail .panic
  deserialize_i16 := by intros; exact .fail .panic
  deserialize_i32 := by intros; exact .fail .panic
  deserialize_i64 := by intros; exact .fail .panic
  deserialize_u8 := by intros; exact .fail .panic
  deserialize_u16 := by intros; exact .fail .panic
  deserialize_u32 := by intros; exact .fail .panic
  deserialize_u64 := by intros; exact .fail .panic
  deserialize_f32 := by intros; exact .fail .panic
  deserialize_f64 := by intros; exact .fail .panic
  deserialize_char := by intros; exact .fail .panic
  deserialize_str := by intros; exact .fail .panic
  deserialize_string := by intros; exact .fail .panic
  deserialize_bytes := by intros; exact .fail .panic
  deserialize_byte_buf := by intros; exact .fail .panic
  deserialize_option := by intros; exact .fail .panic
  deserialize_unit := by intros; exact .fail .panic
  deserialize_unit_struct := by intros; exact .fail .panic
  deserialize_newtype_struct := by intros; exact .fail .panic
  deserialize_seq := by intros; exact .fail .panic
  deserialize_tuple := by intros; exact .fail .panic
  deserialize_tuple_struct := by intros; exact .fail .panic
  deserialize_map := by intros; exact .fail .panic
  deserialize_struct := by intros; exact .fail .panic
  deserialize_enum := by intros; exact .fail .panic
  deserialize_identifier := by intros; exact .fail .panic
  deserialize_ignored_any := by intros; exact .fail .panic

private theorem sampleProvided :
    samplePorts.optionDeserialize sampleValue sampleInput () = .ok (.Ok (some 7#u64)) := by
  simp [samplePorts, sampleValue]

theorem original_preserves_sample_some :
    required samplePorts sampleInput sampleValue () = .ok (.Ok (some 7#u64)) :=
  (original_required_option_forwards_full_value samplePorts sampleInput sampleValue ()).trans sampleProvided

theorem wrong_loses_same_sample_some :
    SourceSerdeRequiredWrong.required samplePorts sampleInput sampleValue () = .ok (.Ok none) := by
  exact actual_wrong_discards_successful_some samplePorts sampleInput sampleValue () 7#u64 sampleProvided

theorem wrong_refutes_same_complete_required_option_contract :
    ¬ RequiredContract @SourceSerdeRequiredWrong.required := by
  intro contract
  have observed := contract samplePorts sampleInput sampleValue ()
  rw [wrong_loses_same_sample_some] at observed
  rw [sampleProvided] at observed
  simp at observed

end
end SandboxModel.Stage1Protocol.SourceSerdeRequiredProof

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeRequiredProof.original_required_option_forwards_full_value
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeRequiredProof.actual_wrong_discards_successful_some
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeRequiredProof.original_preserves_sample_some
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeRequiredProof.wrong_loses_same_sample_some
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeRequiredProof.wrong_refutes_same_complete_required_option_contract

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeRequiredProof.original_required_option_returns_same_none_or_error
