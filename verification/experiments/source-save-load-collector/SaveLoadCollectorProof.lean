import SaveLoadCollectorView

open Aeneas Aeneas.Std ControlFlow SandproofSaveRetentionRows
set_option autoImplicit false
set_option maxRecDepth 4096

namespace SandboxModel.Stage1Protocol.SaveLoadCollectorProof
open SaveLoadCollectorView SaveLoadCursorView
noncomputable section
variable (ports : Ports)

private theorem bind_normal {A B : Type} (input : Result A) (next : A → Result B)
    {output : B} (returned : input >>= next = .ok output) :
    ∃ value, input = .ok value ∧ next value = .ok output := by
  cases input <;> simp_all

private theorem bind_plain {A B : Type} (input : Result A) (next : A → Result B) :
    Std.bind input next = (do let value ← input; next value) := rfl

private theorem vec_appended {T : Type} (buffer output : alloc.vec.Vec T) (entry : T)
    (appended : buffer.push entry = .ok output) : output.val = buffer.val ++ [entry] := by
  unfold alloc.vec.Vec.push at appended
  dsimp only at appended
  split at appended
  · simp only [Result.ok.injEq] at appended
    rw [← appended]
    simp [List.concat_eq_append]
  · simp at appended


theorem decoded_record_has_full_sql_key (bytes sha : Slice U8) (key : Option Bytes)
    (record : LoadedRecord) (ran : decode ports bytes sha key = .ok (.Ok (some record))) :
    ∃ blob, key = some blob ∧ blob.val = (SaveEncoder.keyOf record).val := by
  unfold decode controller.lifecycle_inbox.source_work.codec.decode_loaded_record at ran
  obtain ⟨parsed, _, ran⟩ := bind_normal _ _ ran
  cases parsed with
  | Err error => simp [core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at ran
  | Ok value =>
    cases value with
    | none => cases key <;> simp [core.result.Result.Insts.CoreOpsTry.branch] at ran
    | some stored =>
      cases key with
      | none => simp [core.result.Result.Insts.CoreOpsTry.branch] at ran
      | some blob =>
        simp only [core.result.Result.Insts.CoreOpsTry.branch, asSlice, sliceKeyEq, sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes, bind_tc_ok] at ran
        by_cases sameKey : blob.val = (SaveEncoder.keyOf stored).val
        · have sameSlice : blob.slice.val = (SaveEncoder.keyOf stored).val := sameKey
          simp [SaveEncoder.keyOf, sameSlice] at ran
          obtain ⟨value, _, ran⟩ := bind_normal _ _ ran
          cases value with
          | Err error => simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at ran
          | Ok anchor =>
            simp only [bind_tc_ok] at ran
            obtain ⟨computed, _, ran⟩ := bind_normal _ _ ran
            simp only [SourceWorkMatch.Scalars.wire_content_digest_eq, bind_tc_ok] at ran
            split at ran
            · simp at ran
              subst stored
              exact ⟨blob, rfl, sameKey⟩
            · simp at ran
        · have mismatch : blob.slice.val ≠ stored.work.requirement.work_binding.val := sameKey
          simp [mismatch] at ran

/-- The stored-key test must reject before reaching any anchor serialization
    or digest reply when the optional decoded record is present. -/
theorem foreign_blob_is_rejected_before_anchor (bytes sha : Slice U8) (blob : Bytes)
    (record : LoadedRecord) (parsed : ports.checked bytes sha = .ok (.Ok (some record)))
    (wrong : blob.val ≠ (SaveEncoder.keyOf record).val) :
    decode ports bytes sha (some blob) = .ok (.Err .CorruptStore) := by
  have wrongSlice : blob.slice.val ≠ record.work.requirement.work_binding.val := wrong
  simp [decode, controller.lifecycle_inbox.source_work.codec.decode_loaded_record, parsed,
    core.result.Result.Insts.CoreOpsTry.branch, asSlice, sliceKeyEq, sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes, wrongSlice]

def decodeColumns (columns : Columns) : Result (core.result.Result (Option (LoadedRecord)) SaveRows.Error) :=
  decode ports columns.1.deref columns.2.1.deref columns.2.2

/-- The exact source of each successfully collected record. Successful empty
    slots contribute no record, and the list preserves the cursor order. -/
inductive Decoded : List CursorItem → List (LoadedRecord) → Prop where
  | endOfRows : Decoded [] []
  | skip {columns : Columns} {items : List CursorItem} {rows : List (LoadedRecord)}
      (checked : decodeColumns ports columns = .ok (.Ok none)) (tail : Decoded items rows) :
      Decoded (.Ok columns :: items) rows
  | add {columns : Columns} {items : List CursorItem} {rows : List (LoadedRecord)} {record : LoadedRecord}
      (checked : decodeColumns ports columns = .ok (.Ok (some record))) (tail : Decoded items rows) :
      Decoded (.Ok columns :: items) (record :: rows)

abbrev ReadReply := core.result.Result (alloc.vec.Vec (LoadedRecord)) SaveRows.Error

private theorem next_empty (cursor : Cursor) (empty : cursor.val = []) :
    rowIterator.next cursor = .ok (none, cursor) := by
  let nilCursor : Cursor := .from [] (Nat.zero_le _)
  have same : cursor = nilCursor := alloc.vec.Vec.ext _ _ (by simpa [nilCursor] using empty)
  rw [same]
  rfl

private theorem next_cons (cursor : Cursor) (item : CursorItem) (rest : List CursorItem)
    (shape : cursor.val = item :: rest) (bound : rest.length ≤ Usize.max) :
    rowIterator.next cursor = .ok (some item, alloc.vec.Vec.from rest bound) := by
  unfold rowIterator core.iter.traits.iterator.IteratorVecIntoIter alloc.vec.into_iter.IteratorIntoIter.next
  dsimp only
  split
  · rename_i empty
    rw [shape] at empty
    cases empty
  · rename_i head tail found
    rw [shape] at found
    injection found with headEq tailEq
    subst head
    subst tail
    rfl

private def rowStep (buffer : alloc.vec.Vec (LoadedRecord)) (next : Cursor) :
    Option CursorItem → Result (ControlFlow (Cursor × alloc.vec.Vec (LoadedRecord))
      (core.result.Result (alloc.vec.Vec (LoadedRecord)) SaveRows.Error))
  | none => .ok (.done (.Ok buffer))
  | some (.Err error) => .ok (.done (.Err (.Sqlite error)))
  | some (.Ok columns) => do
    let decoded ← decodeColumns ports columns
    match decoded with
    | .Err error => .ok (.done (.Err error))
    | .Ok none => .ok (.cont (next, buffer))
    | .Ok (some record) => do
      let added ← buffer.push record
      .ok (.cont (next, added))

private theorem read_step_after_next (cursor : Cursor) (buffer : alloc.vec.Vec (LoadedRecord))
    (item : Option CursorItem) (next : Cursor) (observed : rowIterator.next cursor = .ok (item, next)) :
    readStep ports rowIterator cursor buffer = rowStep ports buffer next item := by
  unfold readStep controller.lifecycle_inbox.source_work.codec.read_loaded_rows_loop.body
  rw [observed, bind_tc_ok]
  dsimp only
  cases item with
  | none => rfl
  | some replied =>
    cases replied with
    | Err error =>
      simp [rowStep, core.result.Result.Insts.CoreOpsTry.branch,
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, JournalError.Insts.CoreConvertFromError.from,
        ← bind_plain, bind_ok]
    | Ok columns =>
      obtain ⟨bytes, sha, key⟩ := columns
      simp [rowStep, core.result.Result.Insts.CoreOpsTry.branch, decodeColumns, ← bind_plain]
      congr 1
      funext decoded
      cases decoded with
      | Err error => simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]
      | Ok value => cases value <;> simp [bind_ok]

private theorem unroll (cursor : Cursor) (buffer : alloc.vec.Vec (LoadedRecord)) :
    readLoop ports rowIterator cursor buffer = Std.bind (readStep ports rowIterator cursor buffer) (fun step =>
      match step with
      | .done result => .ok result
      | .cont pair => readLoop ports rowIterator pair.1 pair.2) := by
  unfold readLoop controller.lifecycle_inbox.source_work.codec.read_loaded_rows_loop
  conv_lhs => rw [loop]
  congr 1
  funext step
  cases step with
  | cont pair => cases pair; rfl
  | done result => rfl

theorem collector_accounts_for_complete_order (cursor : Cursor) (buffer result : alloc.vec.Vec (LoadedRecord))
    (ran : readLoop ports rowIterator cursor buffer = .ok (.Ok result)) :
    ∃ added, Decoded ports cursor.val added ∧ result.val = buffer.val ++ added := by
  induction inputs : cursor.val generalizing cursor buffer with
  | nil =>
    rw [unroll] at ran
    obtain ⟨step, taken, ran⟩ := bind_normal _ _ ran
    rw [read_step_after_next ports cursor buffer none cursor (next_empty cursor inputs)] at taken
    have done : step = .done (.Ok buffer) := Result.ok_injective taken.symm
    subst step
    change Result.ok (.Ok buffer : ReadReply) = .ok (.Ok result) at ran
    simp only [Result.ok.injEq, core.result.Result.Ok.injEq] at ran
    subst result
    exact ⟨[], .endOfRows, by simp⟩
  | cons item rest ih =>
    have bound : rest.length ≤ Usize.max := by
      have valid := cursor.property
      rw [inputs] at valid
      simpa only [List.length_cons] using Nat.le_trans (Nat.le_succ rest.length) valid
    let tail : Cursor := .from rest bound
    have next : rowIterator.next cursor = .ok (some item, tail) := next_cons cursor item rest inputs bound
    rw [unroll] at ran
    obtain ⟨step, taken, ran⟩ := bind_normal _ _ ran
    rw [read_step_after_next ports cursor buffer (some item) tail next] at taken
    cases item with
    | Err error =>
      have done : step = .done (.Err (.Sqlite error)) := Result.ok_injective taken.symm
      subst step
      change Result.ok (.Err (.Sqlite error) : ReadReply) = .ok (.Ok result) at ran
      simp at ran
    | Ok columns =>
      simp only [rowStep] at taken
      obtain ⟨decoded, returned, taken⟩ := bind_normal _ _ taken
      cases decoded with
      | Err error =>
        have done : step = .done (.Err error) := Result.ok_injective taken.symm
        subst step
        change Result.ok (.Err error : ReadReply) = .ok (.Ok result) at ran
        simp at ran
      | Ok value =>
        cases value with
        | none =>
          have cont : step = .cont (tail, buffer) := Result.ok_injective taken.symm
          subst step
          have remaining : readLoop ports rowIterator tail buffer = .ok (.Ok result) := ran
          obtain ⟨added, accepted, resEq⟩ := ih tail buffer remaining (by simp [tail])
          exact ⟨added, .skip returned accepted, resEq⟩
        | some record =>
          obtain ⟨buffer1, appended, taken⟩ := bind_normal _ _ taken
          have cont : step = .cont (tail, buffer1) := Result.ok_injective taken.symm
          subst step
          have remaining : readLoop ports rowIterator tail buffer1 = .ok (.Ok result) := ran
          obtain ⟨added, accepted, resEq⟩ := ih tail buffer1 remaining (by simp [tail])
          refine ⟨record :: added, .add returned accepted, ?_⟩
          rw [resEq, vec_appended buffer buffer1 record appended, List.append_assoc]
          rfl

theorem read_ordered_rows (cursor : Cursor) (result : alloc.vec.Vec (LoadedRecord))
    (ran : reader ports rowIterator cursor = .ok (.Ok result)) :
    Decoded ports cursor.val result.val := by
  have ran : readLoop ports rowIterator cursor (alloc.vec.Vec.new (LoadedRecord)) = .ok (.Ok result) := ran
  obtain ⟨added, decoded, resEq⟩ := collector_accounts_for_complete_order ports cursor _ _ ran
  have exactRows : result.val = added := by simpa using resEq
  rw [exactRows]
  exact decoded



def FullStoredKeyContract
    (decodeActual : Ports → Slice U8 → Slice U8 → Option Bytes →
      Result (core.result.Result (Option LoadedRecord) JournalError)) : Prop :=
  ∀ (provided : Ports) (bytes sha : Slice U8) (key : Option Bytes) (record : LoadedRecord),
    decodeActual provided bytes sha key = .ok (.Ok (some record)) →
    ∃ blob, key = some blob ∧ blob.val = (SaveEncoder.keyOf record).val

theorem original_decode_enforces_full_stored_key : FullStoredKeyContract decode := by
  intro provided bytes sha key record returned
  exact decoded_record_has_full_sql_key provided bytes sha key record returned

theorem accepted_decode_preserves_actual_checked_value
    (bytes sha : Slice U8) (key : Option Bytes) (record : LoadedRecord)
    (returned : decode ports bytes sha key = .ok (.Ok (some record))) :
    ∃ (blob anchor : Bytes),
      key = some blob ∧ blob.val = (SaveEncoder.keyOf record).val ∧
      ports.checked bytes sha = .ok (.Ok (some record)) ∧
      ports.anchor record.work.anchor = .ok (.Ok anchor) ∧
      ports.digest anchor.deref = .ok record.work.requirement.work_binding := by
  unfold decode controller.lifecycle_inbox.source_work.codec.decode_loaded_record at returned
  obtain ⟨parsed, parsedReply, returned⟩ := bind_normal _ _ returned
  cases parsed with
  | Err error => simp [core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at returned
  | Ok value =>
    cases value with
    | none => cases key <;> simp [core.result.Result.Insts.CoreOpsTry.branch] at returned
    | some stored =>
      cases key with
      | none => simp [core.result.Result.Insts.CoreOpsTry.branch] at returned
      | some blob =>
        simp only [core.result.Result.Insts.CoreOpsTry.branch, asSlice, sliceKeyEq,
          sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes, bind_tc_ok] at returned
        by_cases sameKey : blob.val = (SaveEncoder.keyOf stored).val
        · have sameSlice : blob.slice.val = stored.work.requirement.work_binding.val := sameKey
          simp [sameSlice] at returned
          obtain ⟨encoded, encodedReply, returned⟩ := bind_normal _ _ returned
          cases encoded with
          | Err error =>
            simp [core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at returned
          | Ok anchor =>
            simp only [bind_tc_ok] at returned
            obtain ⟨computed, computedReply, returned⟩ := bind_normal _ _ returned
            simp only [SourceWorkMatch.Scalars.wire_content_digest_eq, bind_tc_ok] at returned
            by_cases equalDigest : stored.work.requirement.work_binding = computed
            · simp [equalDigest] at returned
              subst record
              exact ⟨blob, anchor, rfl, sameKey, parsedReply, encodedReply, by simpa [equalDigest] using computedReply⟩
            · simp [equalDigest] at returned
        · have mismatch : blob.slice.val ≠ stored.work.requirement.work_binding.val := sameKey
          simp [mismatch] at returned

theorem original_decode_accepts_exact_key_and_digest
    (bytes sha : Slice U8) (blob anchor : Bytes) (record : LoadedRecord)
    (parsed : ports.checked bytes sha = .ok (.Ok (some record)))
    (matching : blob.val = (SaveEncoder.keyOf record).val)
    (encoded : ports.anchor record.work.anchor = .ok (.Ok anchor))
    (computed : ports.digest anchor.deref = .ok record.work.requirement.work_binding) :
    decode ports bytes sha (some blob) = .ok (.Ok (some record)) := by
  have full : blob.slice.val = record.work.requirement.work_binding.val := matching
  simp [decode, controller.lifecycle_inbox.source_work.codec.decode_loaded_record,
    parsed, asSlice, sandproof_protocol.lifecycle.digest.ContentDigest.as_bytes, sliceKeyEq,
    full, encoded, computed, SourceWorkMatch.Scalars.wire_content_digest_eq,
    core.result.Result.Insts.CoreOpsTry.branch]


/-- A finite ordered cursor contract over already-mapped library replies.
    Every successful row uses the same decode and append operations. -/
def expectedRead (ports : Ports) (items : List CursorItem)
    (buffer : alloc.vec.Vec LoadedRecord) : Result ReadReply :=
  match items with
  | [] => .ok (.Ok buffer)
  | .Err error :: _ => .ok (.Err (.Sqlite error))
  | .Ok columns :: tail => do
    let decoded ← decodeColumns ports columns
    match decoded with
    | .Err error => .ok (.Err error)
    | .Ok none => expectedRead ports tail buffer
    | .Ok (some record) => do
      let added ← buffer.push record
      expectedRead ports tail added

theorem actual_loop_refines_ordered_first_error (cursor : Cursor)
    (buffer : alloc.vec.Vec LoadedRecord) :
    readLoop ports rowIterator cursor buffer = expectedRead ports cursor.val buffer := by
  induction inputs : cursor.val generalizing cursor buffer with
  | nil =>
    rw [unroll, read_step_after_next ports cursor buffer none cursor (next_empty cursor inputs)]
    simp [rowStep, expectedRead]
  | cons item rest ih =>
    have bound : rest.length ≤ Usize.max := by
      have valid := cursor.property
      rw [inputs] at valid
      simpa only [List.length_cons] using Nat.le_trans (Nat.le_succ rest.length) valid
    let tail : Cursor := .from rest bound
    have next : rowIterator.next cursor = .ok (some item, tail) := next_cons cursor item rest inputs bound
    rw [unroll, read_step_after_next ports cursor buffer (some item) tail next]
    cases item with
    | Err error => simp [rowStep, expectedRead]
    | Ok columns =>
      unfold rowStep expectedRead
      rw [bind_plain, bind_assoc_eq]
      congr 1
      funext value
      cases value with
      | Err error => simp
      | Ok value =>
        cases value with
        | none => simpa using ih tail buffer (by simp [tail])
        | some record =>
          rw [bind_assoc_eq]
          congr 1
          funext appended
          simpa using ih tail appended (by simp [tail])

theorem actual_reader_refines_ordered_first_error (cursor : Cursor) :
    reader ports rowIterator cursor =
      expectedRead ports cursor.val (alloc.vec.Vec.new LoadedRecord) :=
  actual_loop_refines_ordered_first_error ports cursor _

theorem reader_depends_only_on_next
    (left right : core.iter.traits.iterator.Iterator Cursor CursorItem)
    (sameNext : left.next = right.next) (cursor : Cursor) :
    reader ports left cursor = reader ports right cursor := by
  unfold reader controller.lifecycle_inbox.source_work.codec.read_loaded_rows
    controller.lifecycle_inbox.source_work.codec.read_loaded_rows_loop
  congr 1
  funext pair
  obtain ⟨it, buffer⟩ := pair
  unfold controller.lifecycle_inbox.source_work.codec.read_loaded_rows_loop.body
  rw [sameNext]

theorem original_mapped_cursor_reader_refines_order (raw : RawCursor) :
    reader ports cursorDictionary (mappedCursor raw) =
      expectedRead ports (raw.val.map mapRawItem) (alloc.vec.Vec.new LoadedRecord) := by
  rw [reader_depends_only_on_next ports cursorDictionary rowIterator rfl,
    actual_reader_refines_ordered_first_error]
  simp [mappedCursor, alloc.vec.Vec.from_val]


theorem every_collected_value_has_input_columns {items : List CursorItem} {records : List LoadedRecord}
    (ordered : Decoded ports items records) :
    ∀ record ∈ records, ∃ columns, .Ok columns ∈ items ∧
      decodeColumns ports columns = .ok (.Ok (some record)) := by
  induction ordered with
  | endOfRows => simp
  | skip _ _ ih =>
    intro record member
    obtain ⟨columns, present, decoded⟩ := ih record member
    exact ⟨columns, List.mem_cons_of_mem _ present, decoded⟩
  | @add cols items rows value accepted tail ih =>
    intro record member
    simp only [List.mem_cons] at member
    rcases member with rfl | later
    · exact ⟨cols, by simp, accepted⟩
    · obtain ⟨columns, present, decoded⟩ := ih record later
      exact ⟨columns, List.mem_cons_of_mem _ present, decoded⟩

theorem actual_cursor_collector_keeps_original_row_values
    (raw : RawCursor) (records : alloc.vec.Vec LoadedRecord)
    (returned : reader ports cursorDictionary (mappedCursor raw) = .ok (.Ok records)) :
    Decoded ports (raw.val.map mapRawItem) records.val ∧
      ∀ record ∈ records.val, ∃ (row : RawRow) (columns : Columns) (blob anchor : Bytes),
        .Ok row ∈ raw.val ∧
        mapper row = .ok (.Ok columns) ∧
        columns.2.2 = some blob ∧ blob.val = (SaveEncoder.keyOf record).val ∧
        ports.checked columns.1.deref columns.2.1.deref = .ok (.Ok (some record)) ∧
        ports.anchor record.work.anchor = .ok (.Ok anchor) ∧
        ports.digest anchor.deref = .ok record.work.requirement.work_binding := by
  rw [reader_depends_only_on_next ports cursorDictionary rowIterator rfl] at returned
  have ordered := read_ordered_rows ports (mappedCursor raw) records returned
  simp only [mappedCursor, alloc.vec.Vec.from_val] at ordered
  refine ⟨ordered, ?_⟩
  intro record member
  obtain ⟨columns, present, accepted⟩ := every_collected_value_has_input_columns ports ordered record member
  obtain ⟨item, stored, mapped⟩ := List.mem_map.mp present
  cases item with
  | Err error => simp [mapRawItem] at mapped
  | Ok row =>
    have actualMap : mapper row = .ok (.Ok columns) := by
      rw [SaveLoadCursorProof.mapper_reads_three_columns_in_order]
      exact congrArg Result.ok mapped
    obtain ⟨blob, anchor, key, full, checked, encoded, digest⟩ :=
      accepted_decode_preserves_actual_checked_value ports columns.1.deref columns.2.1.deref
        columns.2.2 record accepted
    exact ⟨row, columns, blob, anchor, stored, actualMap, key, full, checked, encoded, digest⟩

end
end SandboxModel.Stage1Protocol.SaveLoadCollectorProof

#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.decoded_record_has_full_sql_key
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.foreign_blob_is_rejected_before_anchor
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.collector_accounts_for_complete_order
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.read_ordered_rows
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.original_decode_enforces_full_stored_key
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.accepted_decode_preserves_actual_checked_value
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.original_decode_accepts_exact_key_and_digest
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.actual_loop_refines_ordered_first_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.actual_reader_refines_ordered_first_error
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.reader_depends_only_on_next
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.original_mapped_cursor_reader_refines_order
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.every_collected_value_has_input_columns
#audit_controller_proof SandboxModel.Stage1Protocol.SaveLoadCollectorProof.actual_cursor_collector_keeps_original_row_values
