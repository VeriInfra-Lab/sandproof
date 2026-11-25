import SourceSaveFull
import SourceSerdeRequiredProof
import SaveRoundTripWitness

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SourceSaveFull
open SaveRoundTripEncoder

/-- The complete row-evidence condition used in the full composition is
    satisfied by the actual original encoder, and not by the prior actual
    changed encoder. The two sides share the same quantified contract. -/
theorem original_and_wrong_share_full_row_evidence_contract :
    FullEncodeEvidenceContract (@encodeCandidate (List EncoderKey)) ∧
      ¬ FullEncodeEvidenceContract (@SaveRoundTripWrong.encodeCandidate (List EncoderKey)) :=
  ⟨actual_encoder_full_evidence_contract _, SaveRoundTripWitness.wrong_encoder_breaks_same_complete_evidence⟩

end SandboxModel.Stage1Protocol.SourceSaveFull

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSaveFull.original_and_wrong_share_full_row_evidence_contract
