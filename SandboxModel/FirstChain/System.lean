import SandboxModel.FirstChain.Record

/-!
Finite registered nodes, opaque complete binding keys, asynchronous messages
and independent process crashes. Records retain both durable facts and ghost
physical/holder obligations; crashing an observer never erases those effects.
This is a model invariant proof only: each action still needs its real V1–V8
microstep/evidence correspondence. In particular reaffirm is a successful
original-payload durable reconfirmation, not merely a successful Redis read.
-/
namespace SandboxModel.FirstChain

inductive Actor where
  | center | node | host
  deriving DecidableEq

def Action.actor : Action → Actor
  | .accept | .saveReady | .terminate | .saveRelease => .center
  | .effect | .stop | .clean | .drain => .host
  | _ => .node

def Action.local : Action → Bool
  | .receive | .receiveTerminal | .saveReady | .saveRelease | .confirm => false
  | _ => true

def Message.sender : Message → Actor
  | .create | .terminate | .acknowledgement => .center
  | .ready | .released => .node

structure Config (Binding Node : Type) where
  nodes : List Node
  nodeOf : Binding → Node

structure State (Binding Node : Type) where
  records : Binding → Record
  messages : List (Binding × Message)
  centerUp : Bool
  redisUp : Bool
  centerConfirmed : Bool
  nodeUp : Node → Bool

def initial : State Binding Node :=
  ⟨fun _ => {}, [], true, true, true, fun _ => true⟩

def available (cfg : Config Binding Node) (s : State Binding Node)
    (b : Binding) : Actor → Prop
  | .center => s.centerUp = true ∧ s.redisUp = true ∧ s.centerConfirmed = true
  | .node => s.nodeUp (cfg.nodeOf b) = true
  | .host => True

def put [DecidableEq Binding] (s : State Binding Node) (b : Binding)
    (r : Record) : State Binding Node :=
  { s with records := fun k => if k = b then r else s.records k }

def afterAction [DecidableEq Binding] (s : State Binding Node) (b : Binding)
    (a : Action) (r : Record) (replyKnown : Bool) : State Binding Node :=
  { put s b r with
    centerConfirmed := if a.actor == .center then replyKnown else s.centerConfirmed }

def Invariant (cfg : Config Binding Node) (s : State Binding Node) : Prop :=
  (∀ b, Safe (s.records b)) ∧
  (∀ b, (s.records b).accepted = true → cfg.nodeOf b ∈ cfg.nodes) ∧
  (∀ bm ∈ s.messages, bm.2.fact (s.records bm.1) = true)

variable {Binding Node : Type} {cfg : Config Binding Node}
  {s next : State Binding Node} {b other : Binding} {a : Action} {r : Record} {known : Bool}

theorem initial_invariant (cfg : Config Binding Node) : Invariant cfg initial := by
  simp [Invariant, initial, empty_safe]

theorem afterAction_invariant [DecidableEq Binding]
    (hi : Invariant cfg s) (hn : cfg.nodeOf b ∈ cfg.nodes)
    (ha : advance (s.records b) a = some r) :
    Invariant cfg (afterAction s b a r known) := by
  rcases hi with ⟨hs, hb, hm⟩
  refine ⟨?_, ?_, ?_⟩
  · intro k
    by_cases hk : k = b
    · subst k
      simpa [afterAction, put] using advance_safe (hs b) ha
    · simpa [afterAction, put, hk] using hs k
  · intro k h
    by_cases hk : k = b
    · simpa [hk] using hn
    · apply hb k
      simpa [afterAction, put, hk] using h
  · intro bm hmem
    have old := hm bm hmem
    by_cases hk : bm.1 = b
    · simpa [afterAction, put, hk] using
        advance_fact_mono ha (by simpa [hk] using old)
    · simpa [afterAction, put, hk] using old

inductive Step [DecidableEq Binding] [DecidableEq Node]
    (cfg : Config Binding Node) : State Binding Node → State Binding Node → Prop where
  | perform (s : State Binding Node) (b : Binding) (a : Action) (r : Record) (known : Bool)
      (registered : cfg.nodeOf b ∈ cfg.nodes)
      (actorReady : available cfg s b a.actor)
      (localAction : a.local = true)
      (action : advance (s.records b) a = some r) :
      Step cfg s (afterAction s b a r known)
  | send (s : State Binding Node) (b : Binding) (m : Message)
      (actorReady : available cfg s b m.sender)
      (fact : m.fact (s.records b) = true) :
      Step cfg s { s with messages := (b, m) :: s.messages }
  -- Delivery retains the packet, allowing duplicate and delayed deliveries.
  | deliver (s : State Binding Node) (b : Binding) (m : Message) (r : Record) (known : Bool)
      (registered : cfg.nodeOf b ∈ cfg.nodes)
      (inFlight : (b, m) ∈ s.messages)
      (actorReady : available cfg s b m.action.actor)
      (action : advance (s.records b) m.action = some r) :
      Step cfg s (afterAction s b m.action r known)
  -- Any finite loss/reordering/duplication of already sent messages; no forgery.
  | network (s : State Binding Node) (kept : List (Binding × Message)) (fromSent : ∀ bm ∈ kept, bm ∈ s.messages) :
      Step cfg s { s with messages := kept }
  | crashCenter (s : State Binding Node) : Step cfg s { s with centerUp := false, centerConfirmed := false }
  | restartCenter (s : State Binding Node) : Step cfg s { s with centerUp := true, centerConfirmed := false }
  | crashRedis (s : State Binding Node) : Step cfg s { s with redisUp := false, centerConfirmed := false }
  | restartRedis (s : State Binding Node) : Step cfg s { s with redisUp := true, centerConfirmed := false }
  | reaffirm (s : State Binding Node) (up : s.centerUp = true) (store : s.redisUp = true) :
      Step cfg s { s with centerConfirmed := true }
  | unknownNoWrite (s : State Binding Node) :
      Step cfg s { s with centerConfirmed := false }
  | crashNode (s : State Binding Node) (n : Node) :
      Step cfg s { s with nodeUp := fun k => if k = n then false else s.nodeUp k }
  -- Successful audited node recovery, not merely spawning the process.
  | restartNode (s : State Binding Node) (n : Node) :
      Step cfg s { s with nodeUp := fun k => if k = n then true else s.nodeUp k }
  | idle (s : State Binding Node) : Step cfg s s

theorem step_invariant [DecidableEq Binding] [DecidableEq Node]
    (hi : Invariant cfg s) (ht : Step cfg s next) : Invariant cfg next := by
  cases ht with
  | perform b a r known hn _ _ ha => exact afterAction_invariant hi hn ha
  | deliver b m r known hn _ _ ha => exact afterAction_invariant hi hn ha
  | send b m _ hf =>
    refine ⟨hi.1, hi.2.1, ?_⟩
    intro bm hm
    rcases List.mem_cons.mp hm with h | h
    · subst bm; exact hf
    · exact hi.2.2 bm h
  | network kept hd => exact ⟨hi.1, hi.2.1, fun bm hm => hi.2.2 bm (hd bm hm)⟩
  | _ => exact hi

inductive Reachable [DecidableEq Binding] [DecidableEq Node]
    (cfg : Config Binding Node) : State Binding Node → Prop where
  | init : Reachable cfg initial
  | next {s next : State Binding Node} : Reachable cfg s → Step cfg s next → Reachable cfg next

theorem reachable_invariant [DecidableEq Binding] [DecidableEq Node]
    (hr : Reachable cfg s) : Invariant cfg s := by
  induction hr with
  | init => exact initial_invariant cfg
  | next _ h ih => exact step_invariant ih h

theorem reachable_responsibility [DecidableEq Binding] [DecidableEq Node]
    (hr : Reachable cfg s) (hp : (s.records b).potential = true) :
    (s.records b).accepted = true ∧ (s.records b).received = true ∧
    (s.records b).released = false ∧ (s.records b).releaseSaved = false :=
  potential_has_responsibility ((reachable_invariant hr).1 b) hp

theorem reachable_release [DecidableEq Binding] [DecidableEq Node]
    (hr : Reachable cfg s) (released : (s.records b).releaseSaved = true) :
    (s.records b).nodeTerminal = true ∧ (s.records b).fenced = true ∧
    (s.records b).stopped = true ∧ (s.records b).cleaned = true ∧
    (s.records b).potential = false ∧ (s.records b).leases = 0 ∧
    (s.records b).references = 0 :=
  release_requires_quiescence ((reachable_invariant hr).1 b) released

theorem other_binding_unchanged [DecidableEq Binding] (hne : other ≠ b) :
    (afterAction s b a r known).records other = s.records other := by
  simp [afterAction, put, hne]

theorem restart_not_dispatch_ready (cfg : Config Binding Node) (s : State Binding Node)
    (b : Binding) :
    ¬ available cfg { s with centerUp := true, centerConfirmed := false } b .center := by
  simp [available]

end SandboxModel.FirstChain
