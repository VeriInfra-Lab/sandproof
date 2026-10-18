import SandproofTunnel.Funs
import SandboxModel.Foundations.StreamHistory

open Aeneas Aeneas.Std
set_option autoImplicit false
namespace SandproofTunnelRust.Contract

abbrev Reply (E : Type) := core.result.Result Unit E

def joined {E : Type} (upload download : Reply E) : Reply E :=
  match download with
  | .Ok _ => upload
  | .Err error => .Err error

theorem decide_upload {E : Type} (result : Reply E) :
    tunnel.control.decide (.Upload result) = .ok (.AwaitDownload result) := by rfl

theorem decide_download {E : Type} (result : Reply E) :
    tunnel.control.decide (.Download result) = .ok (.Finish result) := by rfl

theorem finish_download_ok {E : Type} (upload : Reply E) :
    tunnel.control.finish_upload upload (.Ok ()) = .ok upload := by
  simp [tunnel.control.finish_upload, core.result.Result.Insts.CoreOpsTry.branch]

theorem finish_download_error {E : Type} (upload : Reply E) (error : E) :
    tunnel.control.finish_upload upload (.Err error) = .ok (.Err error) := by
  simp [tunnel.control.finish_upload, core.result.Result.Insts.CoreOpsTry.branch,
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual]

theorem upload_error_kept {E : Type} (error : E) :
    tunnel.control.finish_upload (.Err error) (.Ok ()) = .ok (.Err error) :=
  finish_download_ok (.Err error)

theorem download_error_priority {E : Type} (uploadError downloadError : E) :
    tunnel.control.finish_upload (.Err uploadError) (.Err downloadError) =
      .ok (.Err downloadError) :=
  finish_download_error (.Err uploadError) downloadError

theorem finish_upload_refines {E : Type} (upload download : Reply E) :
    tunnel.control.finish_upload upload download = .ok (joined upload download) := by
  cases download with
  | Ok value => cases value; exact finish_download_ok upload
  | Err error => exact finish_download_error upload error

theorem success_after_upload_requires_both {E : Type} (upload download : Reply E) :
    tunnel.control.finish_upload upload download = .ok (.Ok ()) ↔
      upload = .Ok () ∧ download = .Ok () := by
  rw [finish_upload_refines]
  cases upload <;> cases download <;> simp [joined]

open SandboxModel.Foundations

/- Byte histories and EOF evidence concern the original raw streams.
   They do not represent HTTP/Connect completion, a filesystem transaction,
   subprocess exit, or authorization. Actual I/O correspondence remains A6/B3.
   Discarding a future cannot erase already accepted byte prefixes. -/
structure Context (Byte End : Type) where
  uploadSource : List Byte
  downloadSource : List Byte
  uploadBudget : StreamHistory.Budgets
  downloadBudget : StreamHistory.Budgets
  validEnd : End → Prop

inductive Stage (E : Type) where
  | selecting
  | awaitDownload (upload : Reply E)
  | returned (result : Reply E)
  | interrupted

def active {E : Type} : Stage E → Prop
  | .selecting | .awaitDownload _ => True
  | .returned _ | .interrupted => False

structure State (Binding Byte End E : Type) where
  binding : Binding
  upload : StreamHistory.State Byte End
  download : StreamHistory.State Byte End
  stage : Stage E

def initial {Binding Byte End E : Type} (binding : Binding) : State Binding Byte End E :=
  ⟨binding, StreamHistory.initial, StreamHistory.initial, .selecting⟩

def Safe {Binding Byte End E : Type} (context : Context Byte End)
    (s : State Binding Byte End E) : Prop :=
  StreamHistory.Safe context.uploadSource context.uploadBudget context.validEnd s.upload ∧
  StreamHistory.Safe context.downloadSource context.downloadBudget context.validEnd s.download

-- A successful original copy/flush or copy/shutdown is allowed only after its
-- raw byte history completed. An error preserves arbitrary prior prefixes.
-- This projection is supplied by the registered library/driver contract.
def replyAllowed {Byte End E : Type} (s : StreamHistory.State Byte End) : Reply E → Prop
  | .Ok _ => s.phase = .completed
  | .Err _ => True

inductive Event (Byte End E : Type) where
  | uploadIO (event : StreamHistory.Event Byte End)
  | downloadIO (event : StreamHistory.Event Byte End)
  | selectedUpload (reply : Reply E)
  | selectedDownload (reply : Reply E)
  | downloadAfterUpload (reply : Reply E)
  | interrupt

inductive Step {Binding Byte End E : Type} (context : Context Byte End) :
    State Binding Byte End E → Event Byte End E → State Binding Byte End E → Prop where
  | uploadIO {s event after} (selecting : s.stage = .selecting)
      (move : StreamHistory.Step context.uploadSource context.uploadBudget context.validEnd
        s.upload event after) :
      Step context s (.uploadIO event) { s with upload := after }
  | downloadIO {s event after} (running : active s.stage)
      (move : StreamHistory.Step context.downloadSource context.downloadBudget context.validEnd
        s.download event after) :
      Step context s (.downloadIO event) { s with download := after }
  | selectedUpload {s result} (selecting : s.stage = .selecting)
      (allowed : replyAllowed s.upload result) :
      Step context s (.selectedUpload result) { s with stage := .awaitDownload result }
  | selectedDownload {s result} (selecting : s.stage = .selecting)
      (allowed : replyAllowed s.download result) :
      Step context s (.selectedDownload result) { s with stage := .returned result }
  | downloadAfterUpload {s upload download} (waiting : s.stage = .awaitDownload upload)
      (allowed : replyAllowed s.download download) :
      Step context s (.downloadAfterUpload download)
        { s with stage := .returned (joined upload download) }
  | interrupt {s} (running : active s.stage) :
      Step context s .interrupt { s with stage := .interrupted }

def historyStep {Binding Byte End E : Type} (context : Context Byte End)
    (s : State Binding Byte End E) (events : List (Event Byte End E))
    (after : State Binding Byte End E) : Prop :=
  ∃ event, events = [event] ∧ Step context s event after

abbrev Trace {Binding Byte End E : Type} (context : Context Byte End) :=
  SandboxModel.Foundations.Execution (historyStep (Binding := Binding) (E := E) context)

theorem initial_safe {Binding Byte End E : Type} (context : Context Byte End)
    (binding : Binding) : Safe context (initial (Byte := Byte) (End := End) (E := E) binding) :=
  ⟨StreamHistory.initial_safe _ _ _, StreamHistory.initial_safe _ _ _⟩

theorem step_preserves {Binding Byte End E : Type} {context : Context Byte End}
    {s after : State Binding Byte End E} {event : Event Byte End E}
    (safe : Safe context s) (move : Step context s event after) : Safe context after := by
  cases move with
  | uploadIO _ changed => exact ⟨StreamHistory.step_preserves safe.1 changed, safe.2⟩
  | downloadIO _ changed => exact ⟨safe.1, StreamHistory.step_preserves safe.2 changed⟩
  | selectedUpload _ _ => exact safe
  | selectedDownload _ _ => exact safe
  | downloadAfterUpload _ _ => exact safe
  | interrupt _ => exact safe

theorem step_binding {Binding Byte End E : Type} {context : Context Byte End}
    {s after : State Binding Byte End E} {event : Event Byte End E}
    (move : Step context s event after) : after.binding = s.binding := by
  cases move <;> rfl

theorem trace_preserves {Binding Byte End E : Type} {context : Context Byte End}
    {first last : State Binding Byte End E} {events : List (Event Byte End E)}
    (run : Trace context first events last) : Safe context first → Safe context last := by
  induction run with
  | refl => exact fun safe => safe
  | cons move _ ih =>
    intro safe
    obtain ⟨_, _, single⟩ := move
    exact ih (step_preserves safe single)

theorem trace_binding {Binding Byte End E : Type} {context : Context Byte End}
    {first last : State Binding Byte End E} {events : List (Event Byte End E)}
    (run : Trace context first events last) : last.binding = first.binding := by
  induction run with
  | refl => rfl
  | cons move _ ih =>
    obtain ⟨_, _, single⟩ := move
    exact ih.trans (step_binding single)

theorem step_starts_active {Binding Byte End E : Type} {context : Context Byte End}
    {s after : State Binding Byte End E} {event : Event Byte End E}
    (move : Step context s event after) : active s.stage := by
  cases move <;> simp_all [active]

theorem terminal_trace_unchanged {Binding Byte End E : Type} {context : Context Byte End}
    {first last : State Binding Byte End E} {events : List (Event Byte End E)}
    (terminal : ¬ active first.stage) (run : Trace context first events last) :
    last = first ∧ events = [] := by
  cases run with
  | refl => exact ⟨rfl, rfl⟩
  | cons move _ =>
    obtain ⟨_, _, single⟩ := move
    exact False.elim (terminal (step_starts_active single))

theorem actual_upload_action {Binding Byte End E : Type} {context : Context Byte End}
    (s : State Binding Byte End E) (result : Reply E)
    (selecting : s.stage = .selecting) (allowed : replyAllowed s.upload result) :
    tunnel.control.decide (.Upload result) = .ok (.AwaitDownload result) ∧
    Step context s (.selectedUpload result) { s with stage := .awaitDownload result } :=
  ⟨decide_upload result, Step.selectedUpload selecting allowed⟩

theorem actual_download_action {Binding Byte End E : Type} {context : Context Byte End}
    (s : State Binding Byte End E) (result : Reply E)
    (selecting : s.stage = .selecting) (allowed : replyAllowed s.download result) :
    tunnel.control.decide (.Download result) = .ok (.Finish result) ∧
    Step context s (.selectedDownload result) { s with stage := .returned result } :=
  ⟨decide_download result, Step.selectedDownload selecting allowed⟩

theorem actual_after_upload {Binding Byte End E : Type} {context : Context Byte End}
    (s : State Binding Byte End E) (upload download : Reply E)
    (waiting : s.stage = .awaitDownload upload) (allowed : replyAllowed s.download download) :
    tunnel.control.finish_upload upload download = .ok (joined upload download) ∧
    Step context s (.downloadAfterUpload download)
      { s with stage := .returned (joined upload download) } :=
  ⟨finish_upload_refines upload download, Step.downloadAfterUpload waiting allowed⟩

end SandproofTunnelRust.Contract
