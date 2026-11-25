import SaveLoadDecision

open Aeneas Aeneas.Std SandproofSaveRetentionRows
set_option autoImplicit false

namespace SandboxModel.Stage1Protocol.SourceSerdeSome
open SaveRoundTripShared SaveLoadCollectorView SaveLoadCursorView SaveLoadCanonicalProof
noncomputable section

abbrev WriteResult (State : Type) := Result (core.result.Result Unit JsonError × State)
abbrev JsonBytes := Result (core.result.Result Bytes JsonError)

/-- Value-level image of the fixed original fresh compact Vec and its one
    serializer invocation. Errors retain the original reply, never claim rollback. -/
def runToVec {State Value : Type} (fresh : Result State)
    (serialize : State → Value → WriteResult State) (finish : State → Result Bytes) (value : Value) : JsonBytes := do
  let initial ← fresh
  let (reply, final) ← serialize initial value
  match reply with
  | .Err error => .ok (.Err error)
  | .Ok () => do
    let output ← finish final
    .ok (.Ok output)

/-- Explicit A6/B4 obligations for the original Option<T>::serialize,
    serde_json Compact Serializer::serialize_some, to_writer and to_vec.
    Neither the library renderer nor the nested record codec is assumed proved. -/
structure VecSymbols (json : SerdePorts) (State : Type) where
  fresh : Result State
  finish : State → Result Bytes
  record : State → LoadedRecord → WriteResult State
  optional : State → Option LoadedRecord → WriteResult State
  someDispatch : State → LoadedRecord → WriteResult State
  optionSome : ∀ writer value, optional writer (some value) = someDispatch writer value
  compactSome : ∀ writer value, someDispatch writer value = record writer value
  actualRecordToVec : ∀ value, json.recordJson value = runToVec fresh record finish value
  actualOptionToVec : ∀ value, json.optionalJson value = runToVec fresh optional finish value

theorem original_some_and_record_have_same_complete_vec {State : Type}
    (json : SerdePorts) (symbols : VecSymbols json State) (value : LoadedRecord) :
    json.optionalJson (some value) = json.recordJson value := by
  rw [symbols.actualOptionToVec, symbols.actualRecordToVec]
  unfold runToVec
  congr 1
  funext writer
  rw [symbols.optionSome, symbols.compactSome]


abbrev ReadResult (State Value : Type) := Result (core.result.Result Value JsonError × State)

/-- Exact document order from_slice → original deserialize → end. A regular
    parse error returns before end; no parser rollback or input-prefix erasure
    is asserted. The concrete inner record decoder remains a named provider. -/
def runFromSlice {State : Type} (start : Slice U8 → Result State)
    (optionRead : State → ReadResult State (Option LoadedRecord))
    (finish : State → ReadResult State Unit) (bytes : Slice U8) :
    Result (core.result.Result (Option LoadedRecord) JsonError) := do
  let initial ← start bytes
  let (decoded, final) ← optionRead initial
  match decoded with
  | .Err error => .ok (.Err error)
  | .Ok value => do
    let (ended, _) ← finish final
    match ended with
    | .Err error => .ok (.Err error)
    | .Ok () => .ok (.Ok value)

/-- Specific A6/B4 contracts for the fixed SliceRead, Option token/visitor,
    inner SourceWorkRecord dictionary, and actual document end. `writtenObject`
    is the remaining concrete nested-derive/field and full parser-state law;
    it does not assume the optional decoder or its returned value. -/
structure SliceSymbols (json : SerdePorts) (State : Type) where
  start : Slice U8 → Result State
  nonnull : State → ReadResult State Unit
  record : State → ReadResult State LoadedRecord
  optional : State → ReadResult State (Option LoadedRecord)
  finish : State → ReadResult State Unit
  writtenObject : ∀ value bytes, json.recordJson value = .ok (.Ok bytes) →
    ∃ initial before after finished,
      start bytes.deref = .ok initial ∧ nonnull initial = .ok (.Ok (), before) ∧
      record before = .ok (.Ok value, after) ∧ finish after = .ok (.Ok (), finished)
  originalSomeVisitor : ∀ initial before after value,
    nonnull initial = .ok (.Ok (), before) → record before = .ok (.Ok value, after) →
    optional initial = .ok (.Ok (some value), after)
  actualFromSlice : ∀ bytes, json.parse bytes = runFromSlice start optional finish bytes

theorem original_option_document_reads_written_record {State : Type}
    (json : SerdePorts) (symbols : SliceSymbols json State) (value : LoadedRecord) (bytes : Bytes)
    (written : json.recordJson value = .ok (.Ok bytes)) :
    json.parse bytes.deref = .ok (.Ok (some value)) := by
  obtain ⟨initial, before, after, finished, started, nonnull, parsed, ended⟩ := symbols.writtenObject value bytes written
  have optional := symbols.originalSomeVisitor initial before after value nonnull parsed
  rw [symbols.actualFromSlice]
  simp [runFromSlice, started, optional, ended]

theorem actual_named_library_symbols_provide_roundtrip_contract {W R : Type}
    (json : SerdePorts) (writer : VecSymbols json W) (reader : SliceSymbols json R) :
    SerdeSomeContract json where
  sameSome := original_some_and_record_have_same_complete_vec json writer
  readsWritten := original_option_document_reads_written_record json reader

end
end SandboxModel.Stage1Protocol.SourceSerdeSome

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeSome.original_some_and_record_have_same_complete_vec

#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeSome.original_option_document_reads_written_record
#audit_controller_proof SandboxModel.Stage1Protocol.SourceSerdeSome.actual_named_library_symbols_provide_roundtrip_contract
