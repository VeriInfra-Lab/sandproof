import SandboxModel.Foundations.Model

namespace SandboxModel.Foundations

structure Contract (State Input Event : Type) where
  pre : State → Input → Prop
  post : State → Input → Event → State → Prop
  frame : State → State → Prop
  fault : State → Input → Event → State → Prop
  budget : State → Input → Event → State → Prop

def Contract.allows {S I E : Type} (c : Contract S I E)
    (s : S) (i : I) (e : E) (next : S) : Prop :=
  c.pre s i ∧ c.post s i e next ∧ c.frame s next ∧
    c.fault s i e next ∧ c.budget s i e next

-- Empty event lists are permitted internal steps. This establishes finite
-- trace safety only; it is deliberately not an eventual-completion theorem.
inductive Execution {S E : Type} (step : S → List E → S → Prop) :
    S → List E → S → Prop where
  | refl (s) : Execution step s [] s
  | cons {s mid last events tail} :
      step s events mid → Execution step mid tail last →
      Execution step s (events ++ tail) last

theorem execution_append {S E : Type} {step : S → List E → S → Prop}
    {first mid last : S} {left right : List E}
    (hleft : Execution step first left mid) (hright : Execution step mid right last) :
    Execution step first (left ++ right) last := by
  induction hleft with
  | refl => simpa using hright
  | cons h _ ih => simpa [List.append_assoc] using Execution.cons h (ih hright)

structure RefinementContract {C M CE ME : Type}
    (cInit : C → Prop) (mInit : M → Prop)
    (cStep : C → List CE → C → Prop) (mStep : M → List ME → M → Prop)
    (R : C → M → Prop) (project : CE → List ME) : Prop where
  init : ∀ c, cInit c → ∃ m, mInit m ∧ R c m
  step : ∀ c m events next, R c m → cStep c events next →
    ∃ after, Execution mStep m (events.flatMap project) after ∧ R next after

theorem refinement_contract_compose {C M CE ME : Type}
    {cInit : C → Prop} {mInit : M → Prop}
    {cStep : C → List CE → C → Prop} {mStep : M → List ME → M → Prop}
    {R : C → M → Prop} {project : CE → List ME}
    (contract : RefinementContract cInit mInit cStep mStep R project)
    {first last : C} {events : List CE}
    (run : Execution cStep first events last) :
    ∀ model, R first model →
      ∃ after, Execution mStep model (events.flatMap project) after ∧ R last after := by
  induction run with
  | refl => intro model h; exact ⟨model, Execution.refl model, h⟩
  | cons hstep _ ih =>
    intro model h
    obtain ⟨mid, hmid, hr⟩ := contract.step _ _ _ _ h hstep
    obtain ⟨after, hafter, hfinal⟩ := ih mid hr
    exact ⟨after, by simpa using execution_append hmid hafter, hfinal⟩

def Frame {S F : Type} {V : F → Type} (observe : (f : F) → S → V f)
    (allowed : F → Prop) (a b : S) : Prop :=
  ∀ f, ¬ allowed f → observe f a = observe f b

theorem frame_compose {S F : Type} {V : F → Type}
    {observe : (f : F) → S → V f} {left right : F → Prop} {a b c : S}
    (hleft : Frame observe left a b) (hright : Frame observe right b c) :
    Frame observe (fun f => left f ∨ right f) a c := by
  intro f h
  exact (hleft f (fun hl => h (Or.inl hl))).trans
    (hright f (fun hr => h (Or.inr hr)))

theorem refinement_transitive {C M A CE ME AE : Type}
    {cInit : C → Prop} {mInit : M → Prop} {aInit : A → Prop}
    {cStep : C → List CE → C → Prop} {mStep : M → List ME → M → Prop}
    {aStep : A → List AE → A → Prop}
    {Rcm : C → M → Prop} {Rma : M → A → Prop}
    {pcm : CE → List ME} {pma : ME → List AE}
    (first : RefinementContract cInit mInit cStep mStep Rcm pcm)
    (second : RefinementContract mInit aInit mStep aStep Rma pma) :
    RefinementContract cInit aInit cStep aStep
      (fun c a => ∃ m, Rcm c m ∧ Rma m a)
      (fun event => (pcm event).flatMap pma) := by
  constructor
  · intro c hc
    obtain ⟨m, hm, hcm⟩ := first.init c hc
    obtain ⟨a, ha, hma⟩ := second.init m hm
    exact ⟨a, ha, m, hcm, hma⟩
  · intro c a events next hr hs
    obtain ⟨m, hcm, hma⟩ := hr
    obtain ⟨mid, hmstep, hnext⟩ := first.step c m events next hcm hs
    obtain ⟨after, haf, hrel⟩ := refinement_contract_compose second hmstep a hma
    exact ⟨after, by simpa [List.flatMap_assoc] using haf, mid, hnext, hrel⟩

end SandboxModel.Foundations
