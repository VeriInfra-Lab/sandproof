import SandboxModel.Boundaries.Task.Model

/-! Finite, arbitrarily large task rosters sharing the same two semaphore pools.
The budget rules describe elementary token acquisition/return, not the capacity
conclusion. Jobs remain present after completion, so ledger conservation does
not discard active work when an HTTP waiter goes away. -/
namespace SandboxModel.Boundaries.Task

def tokenCount (held : Bool) : Nat := if held then 1 else 0

inductive BudgetStep : Bool → Bool → Nat → Nat → Prop
  | absent (n) : BudgetStep false false n n
  | retained (n) : BudgetStep true true n n
  | acquire (n) : BudgetStep false true (n + 1) n
  | release (n) : BudgetStep true false n (n + 1)

theorem budget_conserves {a b x y} (step : BudgetStep a b x y) :
    x + tokenCount a = y + tokenCount b := by
  cases step <;> simp [tokenCount]

def workCount (tasks : List State) : Nat :=
  (tasks.map (fun task => tokenCount task.workHeld)).sum

def statusCount (tasks : List State) : Nat :=
  (tasks.map (fun task => tokenCount task.statusHeld)).sum

def active (s : State) : Bool :=
  s.phase == .acquired || s.phase == .submitted || s.phase == .running

def activeCount (tasks : List State) : Nat :=
  (tasks.map (fun task => tokenCount (active task))).sum

structure World where
  workPool : Nat
  statusPool : Nat
  workCapacity : Nat
  statusCapacity : Nat
  freeWork : Nat
  freeStatus : Nat
  tasks : List State

def BoundTo (w : World) (task : State) : Prop :=
  task.key.workPool = w.workPool ∧ task.key.statusPool = w.statusPool

def WorldSafe (w : World) : Prop :=
  w.workPool ≠ w.statusPool ∧
  (∀ task ∈ w.tasks, ResourceSafe task ∧ BoundTo w task) ∧
  w.freeWork + workCount w.tasks = w.workCapacity ∧
  w.freeStatus + statusCount w.tasks = w.statusCapacity

def initialWorld (wp sp wc sc : Nat) (jobs : List (Nat × Mode)) : World :=
  ⟨wp, sp, wc, sc, wc, sc,
    jobs.map (fun (job, mode) => initial ⟨job, wp, sp⟩ mode)⟩

inductive WorldStep : World → World → Prop
  | one (wp sp wc sc fw fs gw gs before after s t)
      (localStep : Step s t)
      (work : BudgetStep s.workHeld t.workHeld fw gw)
      (status : BudgetStep s.statusHeld t.statusHeld fs gs) :
      WorldStep ⟨wp, sp, wc, sc, fw, fs, before ++ s :: after⟩
        ⟨wp, sp, wc, sc, gw, gs, before ++ t :: after⟩

inductive WorldRun : World → World → Prop
  | refl (w) : WorldRun w w
  | next : WorldRun w x → WorldStep x y → WorldRun w y

theorem initial_world_safe (wp sp wc sc jobs) (distinct : wp ≠ sp) :
    WorldSafe (initialWorld wp sp wc sc jobs) := by
  refine ⟨distinct, ?_⟩
  constructor
  · intro task mem
    simp only [initialWorld, List.mem_map] at mem
    obtain ⟨⟨job, mode⟩, _, rfl⟩ := mem
    exact ⟨initial_safe _ _, by simp [BoundTo, initialWorld, initial]⟩
  · have zero : (jobs.map (fun _ => (0 : Nat))).sum = 0 := by
      induction jobs with
      | nil => rfl
      | cons _ _ ih => simpa using ih
    simpa [initialWorld, workCount, statusCount, initial, tokenCount,
      List.map_map, Function.comp_def] using zero

theorem world_step_safe {w x} (step : WorldStep w x) (safe : WorldSafe w) :
    WorldSafe x := by
  cases step with
  | one wp sp wc sc fw fs gw gs before after s t localStep work status =>
    obtain ⟨distinct, localSafe, workSafe, statusSafe⟩ := safe
    have old := localSafe s (by simp)
    have new := step_safe localStep old.1
    have bound : t.key = s.key := (pool_binding_preserved localStep).1
    refine ⟨distinct, ?_⟩
    constructor
    · intro task mem
      simp only [List.mem_append, List.mem_cons] at mem
      rcases mem with mem | rfl | mem
      · exact localSafe task (by simp [mem])
      · exact ⟨new, by simpa only [BoundTo, bound] using old.2⟩
      · exact localSafe task (by simp [mem])
    · have bw := budget_conserves work
      have bs := budget_conserves status
      simp only [workCount, statusCount, List.map_append, List.map_cons,
        List.sum_append, List.sum_cons] at workSafe statusSafe ⊢
      omega

theorem world_run_safe {w x} (run : WorldRun w x) (safe : WorldSafe w) :
    WorldSafe x := by
  induction run with
  | refl => exact safe
  | next _ step ih => exact world_step_safe step ih

theorem active_implies_work {s} (safe : ResourceSafe s) :
    tokenCount (active s) ≤ tokenCount s.workHeld := by
  by_cases h : s.phase = .acquired ∨ s.phase = .submitted ∨ s.phase = .running
  · have held := (safe.2.1 h).1
    cases active s <;> simp [tokenCount, held]
  · have inactive : active s = false := by simp_all [active]
    simp [tokenCount, inactive]

theorem active_count_le_work {tasks : List State}
    (safe : ∀ task ∈ tasks, ResourceSafe task) : activeCount tasks ≤ workCount tasks := by
  induction tasks with
  | nil => simp [activeCount, workCount]
  | cons task rest ih =>
    have head := active_implies_work (safe task (by simp))
    have tail := ih (fun s mem => safe s (by simp [mem]))
    simp only [activeCount, workCount, List.map_cons, List.sum_cons] at head tail ⊢
    omega

theorem world_capacity_bound {w} (safe : WorldSafe w) :
    activeCount w.tasks ≤ w.workCapacity ∧
    statusCount w.tasks ≤ w.statusCapacity := by
  have activeBound := active_count_le_work (fun t mem => (safe.2.1 t mem).1)
  obtain ⟨_, _, work, status⟩ := safe
  omega

theorem reachable_capacity_bound {w x} (run : WorldRun w x) (safe : WorldSafe w) :
    activeCount x.tasks ≤ x.workCapacity ∧
    statusCount x.tasks ≤ x.statusCapacity :=
  world_capacity_bound (world_run_safe run safe)

theorem from_initial_capacity_bound {wp sp wc sc jobs x}
    (distinct : wp ≠ sp) (run : WorldRun (initialWorld wp sp wc sc jobs) x) :
    activeCount x.tasks ≤ x.workCapacity ∧
    statusCount x.tasks ≤ x.statusCapacity :=
  reachable_capacity_bound run (initial_world_safe wp sp wc sc jobs distinct)

theorem reachable_pool_binding {wp sp wc sc jobs x}
    (distinct : wp ≠ sp) (run : WorldRun (initialWorld wp sp wc sc jobs) x)
    {task} (member : task ∈ x.tasks) :
    task.key.workPool = x.workPool ∧ task.key.statusPool = x.statusPool :=
  ((world_run_safe run (initial_world_safe wp sp wc sc jobs distinct)).2.1 task member).2

theorem no_acquire_without_free : ¬ BudgetStep false true 0 0 := by
  intro step
  cases step

-- Same aggregate capacity predicate detects the real dangerous shape: one
-- running task released its token, and a second task acquired that token.
theorem early_release_allows_two_and_breaks_same_capacity :
    let k : Key := ⟨0, 10, 11⟩
    let a : State := ⟨k, .ordinary, .running, false, false, false, 1, 0, .none⟩
    let b : State := ⟨⟨1, 10, 11⟩, .ordinary, .running, true, false, true, 1, 0, .none⟩
    ¬ (activeCount [a, b] ≤ 1 ∧ statusCount [a, b] ≤ 1) := by
  decide

theorem foreign_pool_breaks_same_world_property :
    ¬ WorldSafe ⟨10, 11, 1, 1, 0, 1,
      [⟨⟨0, 99, 11⟩, .ordinary, .running, true, false, true, 1, 0, .none⟩]⟩ := by
  simp [WorldSafe, BoundTo]

end SandboxModel.Boundaries.Task
