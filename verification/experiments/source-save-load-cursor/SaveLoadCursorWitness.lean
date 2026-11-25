import SaveLoadCursorComposition
import SaveLoadCursorWrong

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SaveLoadCursorWitness
open SaveLoadCursorView SaveLoadCursorProof

noncomputable section

/-- Both mandatory columns contain TEXT. The library gives the two distinct
    column conversion errors different identities, as in InvalidColumnType. -/
def twoTextColumns : RawRow where
  object := 0
  statement := { canonical := ⟨0, [], fun count => ⟨count⟩⟩
                 object := 1
                 columns := 3#usize
                 invalidIndex := fun index => ⟨index.val⟩ }
  column index :=
    if index.val < 2 then .Text (.from [] (Nat.zero_le _)) else .Null
  conversionError index _ _ := ⟨index.val⟩

theorem contract_witness_has_column_zero_error :
    columnsResult twoTextColumns = .Err ⟨⟨0⟩, false⟩ := by
  rfl


def payloadBytes : Bytes := alloc.vec.Vec.from [1#u8, 2#u8] (by scalar_tac)
def checksumBytes : Bytes := alloc.vec.Vec.from [3#u8, 4#u8] (by scalar_tac)

def goodColumns : RawRow :=
  { twoTextColumns with column := fun index =>
      if index.val = 0 then .Blob payloadBytes.deref
      else if index.val = 1 then .Blob checksumBytes.deref
      else .Null }

theorem correct_mapper_preserves_full_values :
    mapper goodColumns = .ok (.Ok (payloadBytes, checksumBytes, none)) := by
  rw [mapper_reads_three_columns_in_order]
  rfl

def keyColumns : RawRow :=
  { goodColumns with column := fun index =>
      if index.val = 2 then .Blob payloadBytes.deref else goodColumns.column index }

theorem correct_mapper_preserves_nullable_key :
    mapper keyColumns = .ok (.Ok (payloadBytes, checksumBytes, some payloadBytes)) := by
  rw [mapper_reads_three_columns_in_order]
  rfl

theorem original_first_error_is_column_zero :
    mapper twoTextColumns = .ok (.Err ⟨⟨0⟩, false⟩) :=
  (mapper_reads_three_columns_in_order twoTextColumns).trans
    (congrArg Result.ok contract_witness_has_column_zero_error)

theorem wrong_mapper_reports_column_one :
    SaveLoadCursorWrong.mapper twoTextColumns = .ok (.Err ⟨⟨1⟩, false⟩) := by
  simp [SaveLoadCursorWrong.mapper,
    SaveLoadCursorWrong.controller.lifecycle_inbox.source_work.codec.read_source_columns,
    twoTextColumns, rusqlite.row.Row.get, Usize.Insts.RusqliteRowRowIndex.idx, asValueRef,
    blobDictionary, SaveLoadCursorView.alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql, fromBlob,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, bind_tc_ok]

theorem checksum_first_violates_same_column_contract : ¬ ColumnContract SaveLoadCursorWrong.mapper := by
  intro contract
  have failed := contract twoTextColumns
  rw [wrong_mapper_reports_column_one, contract_witness_has_column_zero_error] at failed
  simp at failed

end
end SandboxModel.Stage1Protocol.SaveLoadCursorWitness

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorWitness.contract_witness_has_column_zero_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorWitness.correct_mapper_preserves_full_values
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorWitness.original_first_error_is_column_zero
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorWitness.wrong_mapper_reports_column_one
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorWitness.checksum_first_violates_same_column_contract
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCursorWitness.correct_mapper_preserves_nullable_key
