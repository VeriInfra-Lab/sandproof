import SandboxModel.Foundations.Identifiers
import SandboxModel.Foundations.ByteArray

open Aeneas Aeneas.Std SandproofFoundationsRust

namespace SandboxModel.Foundations
open IdentifierLibrary

-- The default String-backed formatter contract is conditional on finite,
-- representable UTF-8 and normal allocation. This is not a JSON codec theorem.
theorem literal_string_bytes_refines (text : String)
    (bounded : text.toByteArray.size ≤ U32.max) :
    (SandproofFoundationsRust.toStr text bounded).val = stringBytes text := by
  simp [SandproofFoundationsRust.toStr, Aeneas.Std.toStr, stringBytes]
  congr 1
  exact byteArray_toList_data text.toByteArray

theorem default_string_write_refines (formatter : SandproofFoundationsRust.core.fmt.Formatter)
    (value : Str) (text : String) (represented : StrTextRep value text) :
    SandproofFoundationsRust.core.fmt.Formatter.write_str formatter value =
      .ok (.Ok (), ⟨formatter.output ++ text⟩) := by
  simp [SandproofFoundationsRust.core.fmt.Formatter.write_str, StrTextRep] at represented ⊢
  rw [represented, decodeString_stringBytes]

theorem internal_error_display_refines (error : RustError) (initialText : String) :
    sandproof_protocol.internal.InternalError.Insts.CoreFmtDisplay.fmt error ⟨initialText⟩ =
      .ok (.Ok (), ⟨initialText ++ internalFailureMessage (internalFailureView error)⟩) := by
  cases error
  all_goals simp [sandproof_protocol.internal.InternalError.Insts.CoreFmtDisplay.fmt,
    SandproofFoundationsRust.core.fmt.Formatter.write_str, literal_string_bytes_refines,
    decodeString_stringBytes, internalFailureView, internalFailureMessage]

theorem internal_error_text_refines (error : RustError) :
    internal_error_text error =
      .ok (internalFailureMessage (internalFailureView error)) := by
  simp [internal_error_text, alloc.string.ToString.Blanket.to_string,
    internal_error_display_refines]

-- Construction and formatting are separate actual calls; Serde custom and the
-- surrounding parser/error-position bridge still require their own proof.
theorem invalid_counter_rejection_message :
    counter_from_u64 0#u64 = .ok (.Err .InvalidCounter) ∧
    internal_error_text .InvalidCounter = .ok "invalid or exhausted internal counter" := by
  constructor
  · simp [counter_from_u64,
      sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from]
  · simpa [internalFailureView, internalFailureMessage] using
      internal_error_text_refines .InvalidCounter

theorem internal_failure_message_injective : Function.Injective internalFailureMessage := by
  intro left right equal
  cases left <;> cases right <;> simp_all [internalFailureMessage]

theorem default_to_string_callback_failure {T : Type}
    (display : SandproofFoundationsRust.core.fmt.Display T) (value : T) (error : Aeneas.Std.Error)
    (failed : display.fmt value ⟨""⟩ = .fail error) :
    alloc.string.ToString.Blanket.to_string display value = .fail error := by
  simp [alloc.string.ToString.Blanket.to_string, failed]

theorem default_to_string_format_error {T : Type}
    (display : SandproofFoundationsRust.core.fmt.Display T) (value : T)
    (formatError : SandproofFoundationsRust.core.fmt.Error)
    (formatter : SandproofFoundationsRust.core.fmt.Formatter)
    (failed : display.fmt value ⟨""⟩ = .ok (.Err formatError, formatter)) :
    alloc.string.ToString.Blanket.to_string display value = .fail .panic := by
  simp [alloc.string.ToString.Blanket.to_string, failed]

end SandboxModel.Foundations
