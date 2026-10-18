import SandboxModel.Foundations.Composition

namespace SandboxModel.Foundations

/- These are owned classification/connection obligations, not environment
assumptions. The caller must connect observations to real effects and outputs. -/
structure ObservationContract {CE ME O : Type}
    (project : CE → List ME) (concrete : CE → List O)
    (abstract : ME → List O) (critical : CE → Prop) : Prop where
  preserves : ∀ event, (project event).flatMap abstract = concrete event
  critical_visible : ∀ event, critical event → concrete event ≠ []

theorem projection_preserves_observation {CE ME O : Type}
    {project : CE → List ME} {concrete : CE → List O}
    {abstract : ME → List O} {critical : CE → Prop}
    (contract : ObservationContract project concrete abstract critical)
    (events : List CE) :
    (events.flatMap project).flatMap abstract = events.flatMap concrete := by
  induction events with
  | nil => rfl
  | cons event rest ih => simp [contract.preserves event, ih]

theorem critical_event_not_erased {CE ME O : Type}
    {project : CE → List ME} {concrete : CE → List O}
    {abstract : ME → List O} {critical : CE → Prop}
    (contract : ObservationContract project concrete abstract critical)
    (event : CE) (important : critical event) : project event ≠ [] := by
  intro erased
  have preserved := contract.preserves event
  rw [erased] at preserved
  exact contract.critical_visible event important preserved.symm

structure ObservableRefinementContract {C M CE ME O : Type}
    (cInit : C → Prop) (mInit : M → Prop)
    (cStep : C → List CE → C → Prop) (mStep : M → List ME → M → Prop)
    (R : C → M → Prop) (project : CE → List ME)
    (concrete : CE → List O) (abstract : ME → List O) (critical : CE → Prop)
    : Prop extends RefinementContract cInit mInit cStep mStep R project where
  observations : ObservationContract project concrete abstract critical
  initial_nonempty : ∃ concrete, cInit concrete

theorem observable_initial_realizable {C M CE ME O : Type}
    {cInit : C → Prop} {mInit : M → Prop}
    {cStep : C → List CE → C → Prop} {mStep : M → List ME → M → Prop}
    {R : C → M → Prop} {project : CE → List ME}
    {concrete : CE → List O} {abstract : ME → List O} {critical : CE → Prop}
    (contract : ObservableRefinementContract cInit mInit cStep mStep R project
      concrete abstract critical) :
    ∃ actual model, cInit actual ∧ mInit model ∧ R actual model := by
  obtain ⟨actual, initial⟩ := contract.initial_nonempty
  obtain ⟨model, initialized, related⟩ := contract.init actual initial
  exact ⟨actual, model, initial, initialized, related⟩

theorem observable_execution_refines {C M CE ME O : Type}
    {cInit : C → Prop} {mInit : M → Prop}
    {cStep : C → List CE → C → Prop} {mStep : M → List ME → M → Prop}
    {R : C → M → Prop} {project : CE → List ME}
    {concrete : CE → List O} {abstract : ME → List O} {critical : CE → Prop}
    (contract : ObservableRefinementContract cInit mInit cStep mStep R project
      concrete abstract critical)
    {first last : C} {events : List CE} (run : Execution cStep first events last)
    (model : M) (represented : R first model) :
    ∃ after, Execution mStep model (events.flatMap project) after ∧ R last after ∧
      (events.flatMap project).flatMap abstract = events.flatMap concrete := by
  obtain ⟨after, execution, related⟩ :=
    refinement_contract_compose contract.toRefinementContract run model represented
  exact ⟨after, execution, related,
    projection_preserves_observation contract.observations events⟩

theorem empty_projection_rejected :
    ¬ ObservationContract (fun (_ : Unit) => ([] : List Unit))
      (fun _ => [()]) (fun _ => [()]) (fun _ => True) := by
  intro contract
  have impossible := contract.preserves ()
  simp at impossible

theorem observation_contract_transitive {CE ME AE O : Type}
    {pcm : CE → List ME} {pma : ME → List AE}
    {concrete : CE → List O} {middle : ME → List O} {abstract : AE → List O}
    {criticalC : CE → Prop} {criticalM : ME → Prop}
    (first : ObservationContract pcm concrete middle criticalC)
    (second : ObservationContract pma middle abstract criticalM) :
    ObservationContract (fun event => (pcm event).flatMap pma)
      concrete abstract criticalC := by
  constructor
  · intro event
    rw [projection_preserves_observation second, first.preserves]
  · exact first.critical_visible

theorem observable_refinement_transitive {C M A CE ME AE O : Type}
    {cInit : C → Prop} {mInit : M → Prop} {aInit : A → Prop}
    {cStep : C → List CE → C → Prop} {mStep : M → List ME → M → Prop}
    {aStep : A → List AE → A → Prop}
    {Rcm : C → M → Prop} {Rma : M → A → Prop}
    {pcm : CE → List ME} {pma : ME → List AE}
    {concrete : CE → List O} {middle : ME → List O} {abstract : AE → List O}
    {criticalC : CE → Prop} {criticalM : ME → Prop}
    (first : ObservableRefinementContract cInit mInit cStep mStep Rcm pcm
      concrete middle criticalC)
    (second : ObservableRefinementContract mInit aInit mStep aStep Rma pma
      middle abstract criticalM) :
    ObservableRefinementContract cInit aInit cStep aStep
      (fun c a => ∃ m, Rcm c m ∧ Rma m a)
      (fun event => (pcm event).flatMap pma) concrete abstract criticalC := by
  exact {
    toRefinementContract := refinement_transitive first.toRefinementContract second.toRefinementContract
    observations := observation_contract_transitive first.observations second.observations
    initial_nonempty := first.initial_nonempty
  }

theorem observable_identity_realizable :
    ObservableRefinementContract (fun (_ : Unit) => True) (fun (_ : Unit) => True)
      (fun _ (_ : List Unit) _ => True) (fun _ (_ : List Unit) _ => True)
      (fun _ _ => True) (fun event => [event]) (fun _ => [()])
      (fun _ => [()]) (fun _ => True) := by
  refine {
    init := ?_
    step := ?_
    observations := ?_
    initial_nonempty := ⟨(), True.intro⟩
  }
  · intro actual _
    exact ⟨actual, True.intro, True.intro⟩
  · intro _ _ events next _ _
    refine ⟨next, ?_, True.intro⟩
    simpa using Execution.cons (show True from True.intro) (Execution.refl next)
  · constructor
    · intro event; rfl
    · intro event _; simp

end SandboxModel.Foundations
