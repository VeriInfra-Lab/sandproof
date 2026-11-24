import CapacityInputs
import EncoderWitness
import CapacityHistoricalSamples

open Aeneas Aeneas.Std SandproofSaveRetentionRows alloc.vec
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Policy PolicyRead)
open SandboxModel.Stage1Protocol.SourceRecordBoundsContract (sampleCapacity sampleConnection)
open SandboxModel.Stage1Protocol.SourceRecordCountSpec (BoundedPolicy)
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveCapacity.Witness
open SaveEncoder

def policy : Policy := ⟨3#u64, some sampleCapacity, 1#u64, 4194304#u64, 32#u64, 32#u64,
  32#u64, 4194304#u64, 1048576#u64, 1#u64⟩

def read : PolicyRead := fun _ => .ok (.Ok (some policy))
def db := sampleConnection 4#i64
def before : EncoderSources := SaveEncoder.Witness.sources
def row : Row := SaveEncoder.Witness.row

def overfilled : EncoderSources := { before with records := Vec.from [row, row] (by scalar_tac) }

def ports : EncoderPorts (List EncoderKey) :=
  { SaveEncoder.Witness.ports with load := fun _ => .ok (.Ok (some before)) }

theorem header : SourceSaveOrder.HeaderAccepted configuration node (effects ports read)
    db overfilled before 1#u16 := by
  refine ⟨rfl, ?_, ?_, rfl, rfl⟩
  · change core.cmp.PartialEq.ne.default
      capture.source.SourceObservationConfig.Insts.CoreCmpPartialEqSourceObservationConfig.eq
      before.config before.config = _
    simp [core.cmp.PartialEq.ne.default, SourceWorkMatch.SourceData.limits_eq]
  · change core.cmp.PartialEq.ne.default
      sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq
      before.node before.node = _
    simp [core.cmp.PartialEq.ne.default, SourceWorkMatch.RequestEquality.node_eq]

theorem policy_bounded : BoundedPolicy policy := by
  unfold BoundedPolicy
  decide

theorem exact_row_count : SourceSaveOrder.rowCount overfilled = 2#u64 := by
  apply UScalar.eq_of_val_eq
  simp [SourceSaveOrder.rowCount, overfilled]

theorem exceeds_original_capacity :
    (1#u16 : U16).val * (policy.max_operations_per_sandbox.val - 2) <
      (SourceSaveOrder.rowCount overfilled).val := by
  rw [exact_row_count]
  decide

end SandboxModel.Stage1Protocol.SaveCapacity.Witness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.header
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.policy_bounded
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.exact_row_count
#audit_controller_proof SandboxModel.Stage1Protocol.SaveCapacity.Witness.exceeds_original_capacity
