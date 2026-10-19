import SandproofTunnel.Contract
open Aeneas Aeneas.Std
open SandboxModel.Foundations
set_option autoImplicit false
namespace SandproofTunnelRust.Contract

theorem trace_prefixes {Binding Byte End E : Type} {context : Context Byte End}
    (binding : Binding) {last : State Binding Byte End E} {events : List (Event Byte End E)}
    (run : Trace context (initial binding) events last) :
    (last.upload.written <+: last.upload.read ∧
      last.upload.written <+: context.uploadSource) ∧
    (last.download.written <+: last.download.read ∧
      last.download.written <+: context.downloadSource) := by
  have safe := trace_preserves run (initial_safe context binding)
  exact ⟨StreamHistory.written_prefix_read_and_source safe.1,
    StreamHistory.written_prefix_read_and_source safe.2⟩

theorem trace_pending_bounds {Binding Byte End E : Type} {context : Context Byte End}
    (binding : Binding) {last : State Binding Byte End E} {events : List (Event Byte End E)}
    (run : Trace context (initial binding) events last) :
    last.upload.pending.length ≤ context.uploadBudget.inflight ∧
      last.download.pending.length ≤ context.downloadBudget.inflight := by
  have safe := trace_preserves run (initial_safe context binding)
  exact ⟨safe.1.inflight, safe.2.inflight⟩

-- Only raw copy completion is concluded. Protocol/business success needs
-- independent framing, authorization and endpoint evidence supplied by V8.
theorem step_return_ok_requires_download {Binding Byte End E : Type}
    {context : Context Byte End} {s after : State Binding Byte End E}
    {event : Event Byte End E} (move : Step context s event after)
    (returned : after.stage = .returned (.Ok ())) : after.download.phase = .completed := by
  cases move with
  | uploadIO selecting _ => simp_all
  | downloadIO running _ => simp_all [active]
  | selectedUpload _ _ => simp_all
  | selectedDownload _ allowed => simp_all [replyAllowed]
  | downloadAfterUpload _ allowed =>
    simp only [Stage.returned.injEq] at returned
    unfold joined at returned
    split at returned <;> simp_all [replyAllowed]
  | interrupt _ => simp_all

theorem trace_return_ok_requires_download {Binding Byte End E : Type}
    {context : Context Byte End} {first last : State Binding Byte End E}
    {events : List (Event Byte End E)} (run : Trace context first events last) :
    (first.stage = .returned (.Ok ()) → first.download.phase = .completed) →
    last.stage = .returned (.Ok ()) → last.download.phase = .completed := by
  induction run with
  | refl => exact id
  | cons move _ ih =>
    intro _
    obtain ⟨_, _, single⟩ := move
    exact ih (step_return_ok_requires_download single)

theorem successful_download_exact {Binding Byte End E : Type}
    {context : Context Byte End} (binding : Binding)
    {last : State Binding Byte End E} {events : List (Event Byte End E)}
    (run : Trace context (initial binding) events last)
    (returned : last.stage = .returned (.Ok ())) :
    last.download.written = context.downloadSource := by
  have safe := trace_preserves run (initial_safe context binding)
  have done := trace_return_ok_requires_download run (by simp [initial]) returned
  obtain ⟨_, _, _, _, written, source⟩ := StreamHistory.completed_exact safe.2 done
  exact written.trans source

theorem permitted_selection_results :
    tunnel.control.decide (.Upload (.Err (7 : Nat))) = .ok (.AwaitDownload (.Err 7)) ∧
    tunnel.control.finish_upload (.Err (7 : Nat)) (.Ok ()) = .ok (.Err 7) ∧
    tunnel.control.decide (.Download (.Ok () : Reply Nat)) = .ok (.Finish (.Ok ())) :=
  ⟨decide_upload _, upload_error_kept _, decide_download _⟩

def rawEofContext : Context UInt8 Unit :=
  ⟨[1], [], ⟨64 * 1024, none⟩, ⟨64 * 1024, none⟩, fun _ => True⟩

theorem raw_success_with_unread_upload :
    ∃ (events : List (Event UInt8 Unit Nat)) (last : State Nat UInt8 Unit Nat),
      Trace rawEofContext (initial 5) events last ∧
      last.stage = .returned (.Ok ()) ∧ last.upload.written = [] ∧
      rawEofContext.uploadSource ≠ [] ∧ last.upload.phase = .open := by
  obtain ⟨ending, done, endStep, finishStep, completed⟩ :=
    StreamHistory.empty_complete_realizable (Byte := UInt8)
      (rawEofContext.downloadBudget) rawEofContext.validEnd () (by trivial)
  let s0 : State Nat UInt8 Unit Nat := initial 5
  let s1 := { s0 with download := ending }
  let s2 := { s1 with download := done }
  let s3 : State Nat UInt8 Unit Nat := { s2 with stage := .returned (.Ok ()) }
  refine ⟨[.downloadIO (.sourceEnd ()), .downloadIO .finish, .selectedDownload (.Ok ())],
    s3, ?_, rfl, rfl, by decide, rfl⟩
  have firstMove : historyStep rawEofContext s0 [.downloadIO (.sourceEnd ())] s1 :=
    ⟨_, rfl, Step.downloadIO (by trivial) endStep⟩
  have secondMove : historyStep rawEofContext s1 [.downloadIO .finish] s2 :=
    ⟨_, rfl, Step.downloadIO (by trivial) finishStep⟩
  have lastMove : historyStep rawEofContext s2 [.selectedDownload (.Ok ())] s3 :=
    ⟨_, rfl, Step.selectedDownload rfl completed⟩
  exact Execution.cons firstMove
    (Execution.cons secondMove (Execution.cons lastMove (Execution.refl _)))


theorem lift_upload_trace {Binding Byte End E : Type} {context : Context Byte End}
    (base : State Binding Byte End E) (selecting : base.stage = .selecting)
    {first last : StreamHistory.State Byte End}
    {events : List (StreamHistory.Event Byte End)}
    (run : StreamHistory.Trace context.uploadSource context.uploadBudget context.validEnd
      first events last) :
    Trace context { base with upload := first } (events.map Event.uploadIO)
      { base with upload := last } := by
  induction run with
  | refl => exact Execution.refl _
  | @cons first mid last events tail move _ ih =>
    obtain ⟨event, rfl, single⟩ := move
    simpa using Execution.cons
      (show historyStep context { base with upload := first } [.uploadIO event]
        { base with upload := mid } from ⟨_, rfl, Step.uploadIO selecting single⟩) ih

theorem lift_download_trace {Binding Byte End E : Type} {context : Context Byte End}
    (base : State Binding Byte End E) (running : active base.stage)
    {first last : StreamHistory.State Byte End}
    {events : List (StreamHistory.Event Byte End)}
    (run : StreamHistory.Trace context.downloadSource context.downloadBudget context.validEnd
      first events last) :
    Trace context { base with download := first } (events.map Event.downloadIO)
      { base with download := last } := by
  induction run with
  | refl => exact Execution.refl _
  | @cons first mid last events tail move _ ih =>
    obtain ⟨event, rfl, single⟩ := move
    simpa using Execution.cons
      (show historyStep context { base with download := first } [.downloadIO event]
        { base with download := mid } from ⟨_, rfl, Step.downloadIO running single⟩) ih

theorem single_step_trace {Binding Byte End E : Type} {context : Context Byte End}
    {first last : State Binding Byte End E} {event : Event Byte End E}
    (move : Step context first event last) : Trace context first [event] last :=
  Execution.cons (show historyStep context first [event] last from ⟨event, rfl, move⟩)
    (Execution.refl last)

-- Existence of an allowed finite execution, not fairness or guaranteed service
-- progress. Sources may be arbitrarily large; only resident queues are bounded.
theorem full_transfer_realizable {Binding Byte End E : Type}
    (context : Context Byte End) (binding : Binding) (upEnd downEnd : End)
    (validUp : context.validEnd upEnd) (validDown : context.validEnd downEnd)
    (upBuffer : 1 ≤ context.uploadBudget.inflight)
    (downBuffer : 1 ≤ context.downloadBudget.inflight)
    (upTotal : StreamHistory.cumulativeAllows context.uploadBudget context.uploadSource.length)
    (downTotal : StreamHistory.cumulativeAllows context.downloadBudget context.downloadSource.length) :
    ∃ (events : List (Event Byte End E)) (last : State Binding Byte End E),
      Trace context (initial binding) events last ∧ last.stage = .returned (.Ok ()) ∧
      last.upload.written = context.uploadSource ∧
      last.download.written = context.downloadSource := by
  obtain ⟨upEvents, upLast, upRun, _, upDone, upExact⟩ :=
    StreamHistory.bounded_resident_completion_realizable context.uploadSource
      context.uploadBudget context.validEnd upEnd validUp upBuffer upTotal
  obtain ⟨downEvents, downLast, downRun, _, downDone, downExact⟩ :=
    StreamHistory.bounded_resident_completion_realizable context.downloadSource
      context.downloadBudget context.validEnd downEnd validDown downBuffer downTotal
  let c0 : State Binding Byte End E := initial binding
  let c1 : State Binding Byte End E := { c0 with upload := upLast }
  let c2 : State Binding Byte End E := { c1 with stage := .awaitDownload (.Ok ()) }
  let c3 : State Binding Byte End E := { c2 with download := downLast }
  let c4 : State Binding Byte End E := { c3 with stage := .returned (.Ok ()) }
  have upCopy : Trace context c0 (upEvents.map Event.uploadIO) c1 := by
    simpa [c0, c1, initial] using lift_upload_trace c0 rfl upRun
  have selected : Trace context c1 [.selectedUpload (.Ok ())] c2 :=
    single_step_trace (Step.selectedUpload rfl upDone)
  have downCopy : Trace context c2 (downEvents.map Event.downloadIO) c3 := by
    simpa [c0, c1, c2, c3, initial] using lift_download_trace c2 (by trivial) downRun
  have finished : Trace context c3 [.downloadAfterUpload (.Ok ())] c4 :=
    single_step_trace (Step.downloadAfterUpload rfl downDone)
  exact ⟨_, c4, execution_append upCopy (execution_append selected
    (execution_append downCopy finished)), rfl, upExact, downExact⟩

end SandproofTunnelRust.Contract
