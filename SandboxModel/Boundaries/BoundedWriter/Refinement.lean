import SandproofBoundedWriter.Funs
import SandboxModel.Boundaries.BoundedWriter.Contract

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.BoundedWriter.Refinement
open SandproofBoundedWriterRust
open sandproof_controller_service.template_registry.evidence

private theorem slice_length (chunk : Slice U8) : (Slice.len chunk).val = chunk.val.length := by
  rfl

/-- All finite normal returns, including errors; no assumption that a callback stops on error. -/
theorem actual_write_contract {B R : Type} (ops : BoundedBuffer B R)
    (view : B → WireBytes) (laws : BufferLaws ops view) :
    WriteContract view (write_bounded ops) := by
  intro before chunk outcome after run
  unfold write_bounded at run
  cases hlen : ops.len before.bytes with
  | vis effect continuation => simp [hlen] at run
  | div => simp [hlen] at run
  | ret n =>
    simp only [hlen, bind_tc_ok] at run
    have exactLength := laws.length_exact before.bytes n hlen
    have arithmetic := Usize.checked_add_bv_spec n (Slice.len chunk)
    cases hadd : Usize.checked_add n (Slice.len chunk) with
    | none =>
      simp only [hadd, lift, bind_tc_ok] at run
      cases herr : ops.capacity_error with
      | vis effect continuation => simp [herr] at run
      | div => simp [herr] at run
      | ret error =>
        simp only [herr, bind_tc_ok, Result.ok.injEq, Prod.mk.injEq] at run
        rcases run with ⟨rfl, rfl⟩
        constructor <;> simp [writerView, outcomeView]
    | some length =>
      simp only [hadd] at arithmetic
      simp only [hadd, lift, bind_tc_ok] at run
      split at run
      · cases herr : ops.capacity_error with
        | vis effect continuation => simp [herr] at run
        | div => simp [herr] at run
        | ret error =>
          simp only [herr, bind_tc_ok, Result.ok.injEq, Prod.mk.injEq] at run
          rcases run with ⟨rfl, rfl⟩
          constructor <;> simp [writerView, outcomeView]
      · rename_i within
        have bound : (view before.bytes).length + chunk.val.length ≤ before.limit.val := by
          have size := slice_length chunk
          scalar_tac
        cases hcap : ops.capacity before.bytes with
        | vis effect continuation => simp [hcap] at run
        | div => simp [hcap] at run
        | ret capacity =>
          simp only [hcap, bind_tc_ok] at run
          split at run
          · cases hreserve : ops.try_reserve_exact before.bytes (Slice.len chunk) with
            | vis effect continuation => simp [hreserve] at run
            | div => simp [hreserve] at run
            | ret reserved =>
              rcases reserved with ⟨reservation, next⟩
              have preserved := laws.reserve_keeps_bytes _ _ _ _ hreserve
              simp only [hreserve, bind_tc_ok] at run
              cases reservation with
              | Ok ignored =>
                simp [core.result.Result.map_err,
                  core.result.Result.Insts.CoreOpsTry.branch] at run
                cases hwrite : ops.extend_from_slice next chunk with
                | vis effect continuation => simp [hwrite] at run
                | div => simp [hwrite] at run
                | ret appended =>
                  have content := laws.append_exact _ _ _ hwrite
                  simp only [hwrite, bind_tc_ok, Result.ok.injEq, Prod.mk.injEq] at run
                  rcases run with ⟨rfl, rfl⟩
                  constructor
                  · rfl
                  · exact id
                  · simp [writerView, outcomeView, content, preserved, IsBounded, bound]
              | Err error =>
                simp [core.result.Result.map_err,
                  write_bounded.closure.Insts.CoreOpsFunctionFnOnceTupleClause0_ReserveErrorError.call_once] at run
                cases herr : ops.allocation_error with
                | vis effect continuation => simp [herr] at run
                | div => simp [herr] at run
                | ret error =>
                  simp [herr, core.result.Result.Insts.CoreOpsTry.branch,
                    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
                    core.convert.FromSame.from] at run
                  rcases run with ⟨rfl, rfl⟩
                  constructor
                  · rfl
                  · exact id
                  · simpa [writerView, outcomeView] using preserved
          · cases hwrite : ops.extend_from_slice before.bytes chunk with
            | vis effect continuation => simp [hwrite] at run
            | div => simp [hwrite] at run
            | ret appended =>
              have content := laws.append_exact _ _ _ hwrite
              simp only [hwrite, bind_tc_ok, Result.ok.injEq, Prod.mk.injEq] at run
              rcases run with ⟨rfl, rfl⟩
              constructor
              · rfl
              · exact id
              · simp [writerView, outcomeView, content, IsBounded, bound]


theorem actual_finish_contract {B R : Type} (ops : BoundedBuffer B R)
    (view : B → WireBytes) (laws : BufferLaws ops view) :
    FinishContract view (finish_bounded ops) := by
  intro writer result output run
  cases flag : writer.exceeded with
  | true =>
    have same : output = .Err .MaintenanceCapacity := by
      simpa [finish_bounded, flag] using run.symm
    simp [same, finishedView, finishSpec, writerView, flag]
  | false =>
    cases result with
    | Err error =>
      have same : output = .Err (.Json error) := by
        simpa [finish_bounded, flag, core.result.Result.Insts.CoreOpsTry.branch,
          core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
          sandproof_controller_service.JournalError.Insts.CoreConvertFromError.from] using run.symm
      simp [same, finishedView, finishSpec, writerView, flag, serializationView]
    | Ok value =>
      cases value
      simp only [finish_bounded, flag, Bool.false_eq_true, ↓reduceIte,
        core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
      cases into : ops.into_bytes writer.bytes with
      | vis effect continuation => simp [into] at run
      | div => simp [into] at run
      | ret bytes =>
        have exactBytes := laws.finish_exact _ _ into
        have same : output = .Ok bytes := by simpa [into] using run.symm
        simp [same, finishedView, finishSpec, writerView, flag, serializationView, exactBytes]

theorem actual_finish_success_is_bounded {B R : Type} (ops : BoundedBuffer B R)
    (view : B → WireBytes) (laws : BufferLaws ops view) (writer : ActualWriter B R)
    (result : core.result.Result Unit serde_json.error.Error) (bytes : alloc.vec.Vec U8)
    (bounded : IsBounded (writerView view writer))
    (success : finish_bounded ops writer result = .ok (.Ok bytes)) :
    bytes.val.length ≤ writer.limit.val := by
  have related := actual_finish_contract ops view laws writer result (.Ok bytes) success
  have finished : finishSpec (writerView view writer) (serializationView result) = .ok bytes.val := by
    simpa [finishedView] using related.symm
  exact finish_success_is_bounded _ _ _ bounded finished

theorem actual_history_to_model {B R : Type} (view : B → WireBytes)
    (write : ActualWrite B R) (contract : WriteContract view write)
    {initial final} {entries} (history : RustHistory write initial entries final) :
    SafeHistory (writerView view initial) entries (writerView view final) := by
  induction history with
  | nil => exact .nil _
  | cons head _ ih => exact .cons (contract _ _ _ _ head) ih

theorem actual_history_preserves_bound {B R : Type} (ops : BoundedBuffer B R)
    (view : B → WireBytes) (laws : BufferLaws ops view) {initial final} {entries}
    (history : RustHistory (write_bounded ops) initial entries final)
    (bounded : IsBounded (writerView view initial)) : IsBounded (writerView view final) :=
  history_preserves_bound
    (actual_history_to_model view _ (actual_write_contract ops view laws) history) bounded

theorem actual_history_keeps_exact_accepted_bytes {B R : Type} (ops : BoundedBuffer B R)
    (view : B → WireBytes) (laws : BufferLaws ops view) {initial final} {entries}
    (history : RustHistory (write_bounded ops) initial entries final) :
    view final.bytes = view initial.bytes ++ acceptedBytes entries :=
  history_keeps_exact_accepted_bytes
    (actual_history_to_model view _ (actual_write_contract ops view laws) history)

theorem actual_response_selection {P O : Type}
    (ops : sandproof_internal.server.ProofJsonResponseBuilder P O)
    (view : O → ResponseObservation) (laws : ResponseLaws ops view) (encoded) (response)
    (run : sandproof_internal.server.proof_json_response_result ops encoded = .ok response) :
    view response = responseSpec (encodedView encoded) := by
  cases encoded with
  | Ok bytes => exact laws.success_exact bytes response run
  | Err error => exact laws.empty_exact response run

theorem actual_history_then_200_is_bounded {B R P O : Type}
    (ops : BoundedBuffer B R) (bytesView : B → WireBytes) (laws : BufferLaws ops bytesView)
    (http : sandproof_internal.server.ProofJsonResponseBuilder P O)
    (responseView : O → ResponseObservation) (headers : ResponseLaws http responseView)
    {initial final entries}
    (history : RustHistory (write_bounded ops) initial entries final)
    (bounded : IsBounded (writerView bytesView initial))
    (result : core.result.Result Unit serde_json.error.Error) (encoded) (response)
    (finished : finish_bounded ops final result = .ok encoded)
    (responded : sandproof_internal.server.proof_json_response_result http encoded = .ok response)
    (success : (responseView response).status = 200) :
    (responseView response).bytes.length ≤ initial.limit.val := by
  have lastBound := actual_history_preserves_bound ops bytesView laws history bounded
  have observation := actual_response_selection http responseView headers encoded response responded
  have sameLimit := history_preserves_limit
    (actual_history_to_model bytesView _ (actual_write_contract ops bytesView laws) history)
  cases encoded with
  | Err error => simp [observation, encodedView, responseSpec] at success
  | Ok bytes =>
    have outputBound := actual_finish_success_is_bounded ops bytesView laws final result bytes lastBound finished
    change final.limit.val = initial.limit.val at sameLimit
    rw [sameLimit] at outputBound
    simpa [observation, encodedView, responseSpec] using outputBound


theorem actual_overflow_refuses_before_reserve {B R : Type} (ops : BoundedBuffer B R)
    (writer : ActualWriter B R) (chunk : Slice U8) (length : Usize) (error : core.io.error.Error)
    (read : ops.len writer.bytes = .ok length)
    (overflow : Usize.checked_add length (Slice.len chunk) = none)
    (factory : ops.capacity_error = .ok error) :
    write_bounded ops writer chunk = .ok (.Err error, { writer with exceeded := true }) := by
  simp [write_bounded, read, overflow, factory, lift]

theorem actual_limit_refuses_before_reserve {B R : Type} (ops : BoundedBuffer B R)
    (writer : ActualWriter B R) (chunk : Slice U8) (length sum : Usize) (error : core.io.error.Error)
    (read : ops.len writer.bytes = .ok length)
    (added : Usize.checked_add length (Slice.len chunk) = some sum)
    (over : writer.limit.val < sum.val) (factory : ops.capacity_error = .ok error) :
    write_bounded ops writer chunk = .ok (.Err error, { writer with exceeded := true }) := by
  simp [write_bounded, read, added, over, factory, lift]

theorem actual_allocation_refusal_keeps_flag {B R : Type} (ops : BoundedBuffer B R)
    (writer : ActualWriter B R) (chunk : Slice U8) (length sum capacity : Usize)
    (next : B) (reserveError : R) (error : core.io.error.Error)
    (read : ops.len writer.bytes = .ok length)
    (added : Usize.checked_add length (Slice.len chunk) = some sum)
    (within : sum.val ≤ writer.limit.val) (cap : ops.capacity writer.bytes = .ok capacity)
    (needsReserve : capacity.val < sum.val)
    (refused : ops.try_reserve_exact writer.bytes (Slice.len chunk) = .ok (.Err reserveError, next))
    (factory : ops.allocation_error = .ok error) :
    write_bounded ops writer chunk = .ok (.Err error, { writer with bytes := next }) := by
  simp [write_bounded, read, added, within, cap, needsReserve, refused, factory, lift,
    core.result.Result.map_err,
    write_bounded.closure.Insts.CoreOpsFunctionFnOnceTupleClause0_ReserveErrorError.call_once,
    core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
    core.convert.FromSame.from]


end SandboxModel.Boundaries.BoundedWriter.Refinement
