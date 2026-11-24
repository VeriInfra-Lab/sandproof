import EncoderSpec
import RetentionProviderCases

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec WP
open controller.lifecycle_inbox.source_work
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveEncoder.Witness

open SourceWorkMatch.Values

def row : Row := { SourceRetentionProviders.Cases.old with work := changedKey }

def sources : EncoderSources :=
  ⟨limits, node, 1#u16, Vec.from [row] (by scalar_tac)⟩

-- Explicit provider outputs for this transport witness. JSON encoding and the
-- hash value of those bytes are separate provider claims.
def anchorBytes : Vec U8 := Vec.from [123#u8, 125#u8] (by scalar_tac)
def recordBytes : Vec U8 := Vec.from [91#u8, 93#u8] (by scalar_tac)
def has (keys : List EncoderKey) (key : EncoderKey) : Bool := decide (key ∈ keys)

def ports : EncoderPorts (List EncoderKey) where
  empty := .ok []
  order :=
    let equality : core.cmp.PartialEq EncoderKey EncoderKey := ⟨fun a b => .ok (decide (a = b)), fun a b => .ok (decide (a ≠ b))⟩
    let order (a b : EncoderKey) := compare (a.val.map UScalar.val) (b.val.map UScalar.val)
    { eqInst := { partialEqInst := equality }
      partialOrdInst := { partialEqInst := equality, partial_cmp := fun a b => .ok (some (order a b)) }
      cmp := fun a b => .ok (order a b) }
  insert keys key := .ok (if has keys key then (false, keys) else (true, key :: keys))
  contains keys key := .ok (has keys key)
  anchorJson _ := .ok (.Ok anchorBytes)
  recordJson _ := .ok (.Ok recordBytes)
  digest _ := .ok otherDigest
  load _ := .ok (.Ok none)
  limit _ _ := .ok (.Ok 1#u64)

theorem key_set : KeySetSpec ports has where
  empty keys equal := by simp [ports] at equal; subst keys; simp [has]
  added before key after result := by
    change Result.ok (if has before key then (false, before) else (true, key :: before)) = _ at result
    cases present : has before key
    · simp [present] at result
      subst after
      refine ⟨rfl, fun item => ?_⟩
      simp only [has, List.mem_cons, Bool.decide_or, Bool.or_comm]
    · simp [present] at result
  contains _ _ := rfl

theorem nonzero : Array.repeat 32#usize (0#u8) ≠ keyOf row := by
  change _ ≠ otherDigest
  decide

theorem reserve : capture.source.SourceObservationConfig.required_bytes sources.config 1#u64 =
    .ok 258048#u64 := by
  have good : capture.source.SourceObservationConfig.required_bytes sources.config 1#u64
      ⦃ bytes => bytes.val = 258048 ⦄ := by
    unfold capture.source.SourceObservationConfig.required_bytes
      capture.retirement.MATERIAL_RETIREMENT_RECORD_MAX_BYTES
      capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES
      capture.retirement.MATERIAL_RELEASE_RECORD_MAX_BYTES
    simp only [sources, limits, lift, bind_tc_ok]
    step*
  obtain ⟨bytes, ran, exactBytes⟩ := WP.spec_imp_exists good
  rw [ran, UScalar.eq_of_val_eq (show bytes.val = (258048#u64).val from exactBytes)]

def rows (key : EncoderKey) : EncoderRows :=
  Vec.from [(key, recordBytes)] (by scalar_tac)

theorem complete : CompleteEncoding ports has sources [keyOf row] (rows (keyOf row)) := by
  refine ⟨?_, ?_, ?_⟩
  · simpa only [sources, rows, Vec.from_val] using
      (List.Forall₂.cons (show EncodedRowOf ports row (keyOf row, recordBytes) from ⟨rfl, rfl⟩)
        List.Forall₂.nil)
  · rfl
  · intro key
    rfl

theorem wrong_key_breaks_complete : ¬ CompleteEncoding ports has sources [keyOf row]
    (rows (Array.repeat 32#usize 0#u8)) := by
  intro checked
  have equal := checked.2.1
  change [Array.repeat 32#usize (0#u8)] = [keyOf row] at equal
  exact nonzero ((List.cons.inj equal).1)

end SandboxModel.Stage1Protocol.SaveEncoder.Witness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.Witness.key_set
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.Witness.nonzero
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.Witness.reserve
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.Witness.complete
#audit_controller_proof SandboxModel.Stage1Protocol.SaveEncoder.Witness.wrong_key_breaks_complete
