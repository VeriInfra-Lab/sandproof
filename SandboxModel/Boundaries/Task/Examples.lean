import SandboxModel.Boundaries.Task.Capacity

namespace SandboxModel.Boundaries.Task.Examples

def key (job : Nat) : Key := ⟨job, 10, 11⟩
def ordinary (phase : Phase) (held wait : Bool) (calls : Nat) (exit := Exit.none) : State :=
  ⟨key 0, .ordinary, phase, held, false, wait, calls, 0, exit⟩
def status (phase : Phase) (work held : Bool) : State :=
  ⟨key 1, .status, phase, work, held, true, 0, 0, .none⟩
def world (fw fs : Nat) (a b : State) : World :=
  ⟨10, 11, 1, 1, fw, fs, [a, b]⟩

def start : World := initialWorld 10 11 1 1 [(0, .ordinary), (1, .status)]
def completed : World := world 1 1
  (ordinary .done false false 1 .callbackOk)
  ⟨key 1, .status, .discarded, false, false, false, 0, 0, .none⟩

-- Two actual task instances, one shared work permit and one status permit:
-- run ordinary work, queue status, detach ordinary caller, finish work, grant
-- the queued reservation, cancel status before it is repolled/submitted.
theorem cancelled_wait_and_granted_queue_trace : WorldRun start completed := by
  let a0 := ordinary .idle false true 0
  let a1 := ordinary .acquired true true 0
  let a2 := ordinary .submitted true true 0
  let a3 := ordinary .running true true 1
  let a4 := ordinary .running true false 1
  let a5 := ordinary .done false false 1 .callbackOk
  let b0 := status .idle false false
  let b1 := status .waitingWork false true
  let b2 := status .acquired true true
  let b3 : State := ⟨key 1, .status, .discarded, false, false, false, 0, 0, .none⟩
  have s1 : WorldStep (world 1 1 a0 b0) (world 0 1 a1 b0) :=
    .one 10 11 1 1 1 1 0 1 [] [b0] a0 a1
      (.ordinaryAcquire (key 0) true) (.acquire 0) (.absent 1)
  have s2 : WorldStep (world 0 1 a1 b0) (world 0 1 a2 b0) :=
    .one 10 11 1 1 0 1 0 1 [] [b0] a1 a2
      (.submit (key 0) .ordinary true) (.retained 0) (.absent 1)
  have s3 : WorldStep (world 0 1 a2 b0) (world 0 1 a3 b0) :=
    .one 10 11 1 1 0 1 0 1 [] [b0] a2 a3
      (.start (key 0) .ordinary true) (.retained 0) (.absent 1)
  have s4 : WorldStep (world 0 1 a3 b0) (world 0 0 a3 b1) :=
    .one 10 11 1 1 0 1 0 0 [a3] [] b0 b1
      (.statusReserve (key 1) true) (.absent 0) (.acquire 0)
  have s5 : WorldStep (world 0 0 a3 b1) (world 0 0 a4 b1) :=
    .one 10 11 1 1 0 0 0 0 [] [b1] a3 a4
      (.cancelRunningWait (key 0) .ordinary true 0) (.retained 0) (.absent 0)
  have s6 : WorldStep (world 0 0 a4 b1) (world 1 0 a5 b1) :=
    .one 10 11 1 1 0 0 1 0 [] [b1] a4 a5
      (.finish (key 0) .ordinary false 0 .callbackOk (Or.inl rfl))
      (.release 0) (.absent 0)
  have s7 : WorldStep (world 1 0 a5 b1) (world 0 0 a5 b2) :=
    .one 10 11 1 1 1 0 0 0 [a5] [] b1 b2
      (.statusAcquireWork (key 1) true) (.acquire 0) (.retained 0)
  have s8 : WorldStep (world 0 0 a5 b2) (world 1 1 a5 b3) :=
    .one 10 11 1 1 0 0 1 1 [a5] [] b2 b3
      (.cancelAcquired (key 1) .status true) (.release 0) (.release 0)
  exact .next (.next (.next (.next (.next (.next (.next (.next (.refl _) s1) s2) s3) s4) s5) s6) s7) s8

theorem trace_has_no_remaining_reservations :
    completed.freeWork = 1 ∧ completed.freeStatus = 1 ∧
    workCount completed.tasks = 0 ∧ statusCount completed.tasks = 0 := by
  decide

theorem trace_satisfies_same_capacity :
    activeCount completed.tasks ≤ completed.workCapacity ∧
    statusCount completed.tasks ≤ completed.statusCapacity :=
  from_initial_capacity_bound (by decide) cancelled_wait_and_granted_queue_trace

theorem non_atomic_pair_drop_prefix (k) :
    Run ⟨k, .status, .running, true, true, true, 1, 0, .none⟩
      ⟨k, .status, .settled, false, true, true, 1, 0, .callbackOk⟩ :=
  .next (.next (.refl _)
    (.settle k .status true 0 .callbackOk (Or.inl rfl)))
    (.dropSettledWork k .status true true 1 0 .callbackOk)

theorem partial_drop_safe_and_remaining_status_counted (k) :
    let s : State := ⟨k, .status, .settled, false, true, true, 1, 0, .callbackOk⟩
    ResourceSafe s ∧ activeCount [s] = 0 ∧ workCount [s] = 0 ∧ statusCount [s] = 1 := by
  simp [ResourceSafe, activeCount, workCount, statusCount, tokenCount, active]

end SandboxModel.Boundaries.Task.Examples
