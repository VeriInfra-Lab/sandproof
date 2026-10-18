import SandboxModel.Boundaries.JsonIdentifiers.Refinement
import SandboxModel.Boundaries.JsonIdentifiers.ValueReader

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
open IdentifierLibrary
namespace SandboxModel.Boundaries.JsonIdentifiers

theorem primitive_reads_text (text : String) :
    primitive (valueReader text) 0#u64 = .ok (.Ok text) := by
  rfl

theorem actual_accepts_example (kind : OpaqueKind) :
    actual kind (valueReader "id.A-1_") 0#u64 = .ok (.Ok "id.A-1_") := by
  apply valid_read_preserved
  · exact primitive_reads_text _
  · decide

theorem actual_rejects_empty (kind : OpaqueKind) :
    actual kind (valueReader "") 0#u64 = .ok (.Err ()) := by
  rw [invalid_read_rejected kind _ _ "" (primitive_reads_text "")
    (by decide) (by decide)]
  simp [valueReader, SandboxModel.Boundaries.JsonCounter.Examples.valueReader,
    SandboxModel.Boundaries.JsonCounter.Examples.unitError]




end SandboxModel.Boundaries.JsonIdentifiers
