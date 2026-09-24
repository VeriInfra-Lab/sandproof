import SandboxModel.WorkerController

/-!
Backward coverage for worker admission.

Every theorem keeps the exact transport delivery as an explicit
`WorkerInput`: the input contains a proof that the exact `EffectId` is in the
in-flight set.  The additional `head` premise says that this delivery is the
current retry attempt.  This is intentional: membership of `World.inflight`
is a proposition and is not silently assumed to be decidable by the worker
reducer.
-/

namespace SandboxModel.Controller

set_option maxRecDepth 100000
set_option maxHeartbeats 0

theorem handleWorker_cleanupVm_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (command : input.effect.command = .cleanupVm sid generation vmId)
    (head : currentEffectId world (.cleanupVm sid generation vmId) =
      input.effect)
    (transition : Step world (.cleanupVm sid generation vmId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .cleanupVm sid generation vmId ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .cleanupVm sid generation vmId at command
        subst actualCommand
        cases transition
        case cleanupVm =>
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world
                (.cleanupVm sid generation vmId) =
                { logical := { command := .cleanupVm sid generation vmId },
                  attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_cleanupSlot_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (command : input.effect.command = .cleanupSlot sid generation slotId)
    (head : currentEffectId world (.cleanupSlot sid generation slotId) =
      input.effect)
    (transition : Step world (.cleanupSlot sid generation slotId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .cleanupSlot sid generation slotId ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .cleanupSlot sid generation slotId at command
        subst actualCommand
        cases transition
        case cleanupSlot =>
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world
                (.cleanupSlot sid generation slotId) =
                { logical := { command := .cleanupSlot sid generation slotId },
                  attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_applyFilePolicy_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (found : world.sandboxes sid = some record)
    (eventGeneration : generation = record.generation)
    (command : input.effect.command = .applyFilePolicy sid record.epoch
      record.generation record.workspace record.filePolicy)
    (head : currentEffectId world (.applyFilePolicy sid record.epoch
      record.generation record.workspace record.filePolicy) = input.effect)
    (transition : Step world (.applyFilePolicy sid generation) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .applyFilePolicy sid generation ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .applyFilePolicy sid record.epoch
          record.generation record.workspace record.filePolicy at command
        subst actualCommand
        cases transition
        case applyFilePolicy =>
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world (.applyFilePolicy sid record.epoch
                record.generation record.workspace record.filePolicy) =
                { logical := { command := (.applyFilePolicy sid record.epoch
                    record.generation record.workspace record.filePolicy) },
                  attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_configureNetwork_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (found : world.sandboxes sid = some record)
    (currentVm : record.currentVm = some vmId)
    (command : input.effect.command = .configureNetwork sid record.epoch
      record.generation vmId slotId record.policy)
    (head : currentEffectId world (.configureNetwork sid record.epoch
      record.generation vmId slotId record.policy) = input.effect)
    (transition : Step world (.configureNetwork sid slotId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .configureNetwork sid slotId ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .configureNetwork sid record.epoch
          record.generation vmId slotId record.policy at command
        subst actualCommand
        cases transition
        case configureNetwork =>
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world (.configureNetwork sid record.epoch
                record.generation vmId slotId record.policy) =
                { logical := { command := (.configureNetwork sid record.epoch
                    record.generation vmId slotId record.policy) },
                  attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_freezeVm_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (found : world.sandboxes sid = some record)
    (command : input.effect.command = .freezeVm sid record.epoch
      record.generation vmId)
    (head : currentEffectId world (.freezeVm sid record.epoch
      record.generation vmId) = input.effect)
    (transition : Step world (.freezeVm sid vmId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .freezeVm sid vmId ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .freezeVm sid record.epoch
          record.generation vmId at command
        subst actualCommand
        cases transition
        case freezeVm =>
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world (.freezeVm sid record.epoch
                record.generation vmId) =
                { logical := { command := (.freezeVm sid record.epoch
                    record.generation vmId) }, attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_commitSnapshot_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (snapshotFound : world.snapshots snapshotId = some snapshot)
    (sandboxFound : world.sandboxes sid = some record)
    (snapshotOwned : snapshot.sandbox = sid)
    (command : input.effect.command = .commitSnapshot sid record.epoch
      record.generation snapshotId snapshot.manifest)
    (head : currentEffectId world (.commitSnapshot sid record.epoch
      record.generation snapshotId snapshot.manifest) = input.effect)
    (transition : Step world (.commitSnapshot snapshotId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .commitSnapshot snapshotId ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .commitSnapshot sid record.epoch
          record.generation snapshotId snapshot.manifest at command
        subst actualCommand
        cases transition
        case commitSnapshot =>
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world (.commitSnapshot sid record.epoch
                record.generation snapshotId snapshot.manifest) =
                { logical := { command := (.commitSnapshot sid record.epoch
                    record.generation snapshotId snapshot.manifest) },
                  attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_installFence_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (found : world.sandboxes sid = some record)
    (command : input.effect.command =
      .installFence sid record.epoch record.terminal)
    (head : currentEffectId world
      (.installFence sid record.epoch record.terminal) = input.effect)
    (transition : Step world (.installFence sid) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .installFence sid ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand =
          .installFence sid record.epoch record.terminal at command
        subst actualCommand
        cases transition <;> unfold handleWorker <;> dsimp
        all_goals
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            simp_all [EffectId.command]

theorem handleWorker_activateVm_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (found : world.sandboxes sid = some record)
    (command : input.effect.command =
      .activateVm sid record.epoch record.generation vmId)
    (head : currentEffectId world
      (.activateVm sid record.epoch record.generation vmId) = input.effect)
    (transition : Step world (.activateVm sid vmId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .activateVm sid vmId ∧
      decision.next = next := by
  have indexSpec := indexedNoExecutingVm_global_spec
    (world := world) (sid := sid) valid
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand =
          .activateVm sid record.epoch record.generation vmId at command
        subst actualCommand
        cases transition
        case activateVm =>
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals (try split)
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world (.activateVm sid record.epoch
                record.generation vmId) =
                { logical := { command := (.activateVm sid record.epoch
                    record.generation vmId) }, attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_prepareVm_fresh_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (found : world.sandboxes sid = some record)
    (command : input.effect.command = .prepareVm sid record.epoch
      record.generation vmId slotId none)
    (head : currentEffectId world (.prepareVm sid record.epoch
      record.generation vmId slotId none) = input.effect)
    (transition : Step world (.prepareVm sid vmId slotId none) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .prepareVm sid vmId slotId none ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .prepareVm sid record.epoch
          record.generation vmId slotId none at command
        subst actualCommand
        cases transition
        case prepareVm =>
          subst_vars
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp (config := { singlePass := true }) only [EffectId.command]
              unfold handlePrepareVmCommand
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world (.prepareVm sid record.epoch
                record.generation vmId slotId none) =
                { logical := { command := (.prepareVm sid record.epoch
                    record.generation vmId slotId none) },
                  attempt := attempt } at stale
            exact (stale head).elim

/-- Restore preparation has the same exact-attempt requirements as a fresh
    start, plus the durable, identity-matched snapshot evidence carried by the
    relational `prepareVm` step. -/
theorem handleWorker_prepareVm_restore_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (found : world.sandboxes sid = some record)
    (command : input.effect.command = .prepareVm sid record.epoch
      record.generation vmId slotId (some source))
    (head : currentEffectId world (.prepareVm sid record.epoch
      record.generation vmId slotId (some source)) = input.effect)
    (transition :
      Step world (.prepareVm sid vmId slotId (some source)) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .prepareVm sid vmId slotId (some source) ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .prepareVm sid record.epoch
          record.generation vmId slotId (some source) at command
        subst actualCommand
        cases transition
        case prepareVm =>
          rename_i recordStep operation plan starting live currentOperation
            planEpoch planGeneration planSealed planKey foundStep foundPlan
            fenced plannedVm freshVm plannedSlot freshSlot plannedSource
            selectedSource resumeSource queued dispatched
          have recordEq : recordStep = record :=
            Option.some.inj (foundStep.symm.trans found)
          subst recordStep
          have durable : record.durableSnapshot = some source :=
            (resumeSource source rfl).1
          have usable : usableSnapshot world sid record source :=
            (resumeSource source rfl).2
          have usableChecked :
              usableSnapshotCheck world sid record source = true :=
            usableSnapshotCheck_spec.mpr usable
          subst_vars
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp only [EffectId.command]
              unfold handlePrepareVmCommand
              split
              case h_1 heq =>
                rw [heq] at found
                contradiction
              case h_2 recordFound heq =>
                have same : recordFound = record :=
                  Option.some.inj (heq.symm.trans found)
                subst recordFound
                rw [dif_pos starting, dif_pos live, dif_pos rfl, dif_pos rfl]
                split
                case h_1 currentNone =>
                  rw [currentNone] at currentOperation
                  contradiction
                case h_2 operationFound operationChoice =>
                  have sameOperation : operationFound = operation :=
                    Option.some.inj (operationChoice.symm.trans currentOperation)
                  subst operationFound
                  split
                  case h_1 planNone =>
                    rw [planNone] at foundPlan
                    contradiction
                  case h_2 planFound planChoice =>
                    have samePlan : planFound = plan :=
                      Option.some.inj (planChoice.symm.trans foundPlan)
                    subst planFound
                    rw [dif_pos planKey, dif_pos planEpoch,
                      dif_pos planGeneration, dif_pos planSealed,
                      dif_pos plannedVm, dif_pos plannedSlot,
                      dif_pos plannedSource]
                    split
                    case h_1 fenceNone =>
                      rw [fenceNone] at fenced
                      contradiction
                    case h_2 fenceFound fenceChoice =>
                      have sameFence : fenceFound =
                          { epoch := record.epoch, terminal := false } :=
                        Option.some.inj (fenceChoice.symm.trans fenced)
                      subst fenceFound
                      rw [dif_pos rfl]
                      split
                      case h_1 existingVm vmFound =>
                        rw [vmFound] at freshVm
                        contradiction
                      case h_2 vmMissing =>
                        split
                        case h_1 existingSlot slotFound =>
                          rw [slotFound] at freshSlot
                          contradiction
                        case h_2 slotMissing =>
                          rw [dif_pos selectedSource]
                          simp only
                          split
                          case h_1 durableMissing =>
                            rw [durableMissing] at durable
                            contradiction
                          case h_2 durableSource durableFound =>
                            have sameDurable : durableSource = source :=
                              Option.some.inj (durableFound.symm.trans durable)
                            subst durableSource
                            rw [dif_pos rfl, dif_pos usableChecked]
                            exact ⟨_, rfl, rfl, rfl⟩
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world (.prepareVm sid record.epoch
                record.generation vmId slotId (some source)) =
                { logical := { command := (.prepareVm sid record.epoch
                    record.generation vmId slotId (some source)) },
                  attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_sealSnapshot_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (found : world.sandboxes sid = some record)
    (currentVm : record.currentVm = some vmId)
    (command : input.effect.command = .sealSnapshot sid record.epoch
      record.generation vmId record.workspace snapshotId manifest)
    (head : currentEffectId world (.sealSnapshot sid record.epoch
      record.generation vmId record.workspace snapshotId manifest) =
      input.effect)
    (transition : Step world (.sealSnapshot sid snapshotId manifest) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .sealSnapshot sid snapshotId manifest ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand = .sealSnapshot sid record.epoch
          record.generation vmId record.workspace snapshotId manifest at command
        subst actualCommand
        cases transition
        case sealSnapshot =>
          subst_vars
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp (config := { singlePass := true, maxSteps := 1000000 }) only
                [EffectId.command]
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            change ¬currentEffectId world (.sealSnapshot sid record.epoch
                record.generation vmId record.workspace snapshotId manifest) =
                { logical := { command := (.sealSnapshot sid record.epoch
                    record.generation vmId record.workspace snapshotId
                    manifest) }, attempt := attempt } at stale
            exact (stale head).elim

theorem handleWorker_stopVm_revoking_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (foundSandbox : world.sandboxes sid = some record)
    (foundVm : world.vms vmId = some vm)
    (revoking : record.phase = .revoking)
    (terminal : record.terminal = true)
    (fenced : world.workerFences sid = some ⟨record.epoch, true⟩)
    (command : input.effect.command =
      .stopVm sid record.epoch vm.generation vmId)
    (head : currentEffectId world
      (.stopVm sid record.epoch vm.generation vmId) = input.effect)
    (transition : Step world (.stopVm sid vmId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .stopVm sid vmId ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand =
          .stopVm sid record.epoch vm.generation vmId at command
        subst actualCommand
        cases transition
        case stopVm =>
          subst_vars
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp (config := { singlePass := true, maxSteps := 1000000 }) only
                [EffectId.command]
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            simp_all [EffectId.command]

theorem handleWorker_stopVm_pausing_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (foundSandbox : world.sandboxes sid = some record)
    (foundVm : world.vms vmId = some vm)
    (pausing : record.phase = .pausing)
    (current : record.currentVm = some vmId)
    (live : record.terminal = false)
    (frozen : vm.status = .frozen)
    (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
    (currentOperation : record.currentOperation = some operation)
    (foundPlan : world.operationPlans (operationKey sid operation.id) =
      some plan)
    (target : plan.targetSnapshot = some source)
    (snapshotFound : world.snapshots source.snapshot = some snapshot)
    (snapshotSandbox : snapshot.sandbox = sid)
    (snapshotGeneration : snapshot.sourceGeneration = record.generation)
    (snapshotStage : snapshot.stage = .durable)
    (snapshotImmutable : snapshot.immutable = true)
    (command : input.effect.command =
      .stopVm sid record.epoch vm.generation vmId)
    (head : currentEffectId world
      (.stopVm sid record.epoch vm.generation vmId) = input.effect)
    (transition : Step world (.stopVm sid vmId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .stopVm sid vmId ∧
      decision.next = next := by
  cases input with
  | mk effect inputQueued =>
    cases effect with
    | mk logical attempt =>
      cases logical with
      | mk actualCommand =>
        change actualCommand =
          .stopVm sid record.epoch vm.generation vmId at command
        subst actualCommand
        cases transition
        case stopVm =>
          subst_vars
          unfold handleWorker
          dsimp
          split
          case isTrue currentHead =>
            split
            case isTrue awaiting =>
              unfold handleWorkerCommand
              simp (config := { singlePass := true, maxSteps := 1000000 }) only
                [EffectId.command]
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals (try split) <;> try simp_all
              all_goals grind
            case isFalse notAwaiting =>
              simp_all [effectAwaiting, effectHasStatus]
          case isFalse stale =>
            simp_all [EffectId.command]

/-- After the relational pause-stop rule is tied to the operation plan, its
    snapshot witnesses can be recovered from the step itself; callers do not
    need to provide a second, potentially inconsistent plan choice. -/
theorem handleWorker_stopVm_pausing_step_complete
    (valid : ResourceIndexValid world)
    (input : WorkerInput world)
    (foundSandbox : world.sandboxes sid = some record)
    (foundVm : world.vms vmId = some vm)
    (pausing : record.phase = .pausing)
    (command : input.effect.command =
      .stopVm sid record.epoch vm.generation vmId)
    (head : currentEffectId world
      (.stopVm sid record.epoch vm.generation vmId) = input.effect)
    (transition : Step world (.stopVm sid vmId) next) :
    ∃ decision,
      handleWorker world valid input = .ok decision ∧
      decision.event = .stopVm sid vmId ∧
      decision.next = next := by
  have evidence :
      ∃ operation plan source snapshot,
        record.currentVm = some vmId ∧
        record.terminal = false ∧
        vm.status = .frozen ∧
        world.workerFences sid = some ⟨record.epoch, false⟩ ∧
        record.currentOperation = some operation ∧
        world.operationPlans (operationKey sid operation.id) = some plan ∧
        plan.targetSnapshot = some source ∧
        world.snapshots source.snapshot = some snapshot ∧
        snapshot.sandbox = sid ∧
        snapshot.sourceGeneration = record.generation ∧
        snapshot.stage = .durable ∧ snapshot.immutable = true := by
    cases transition
    case stopVm =>
      simp_all
  rcases evidence with
    ⟨operation, plan, source, snapshot, current, live, frozen, fenced,
      currentOperation, foundPlan, target, snapshotFound, snapshotSandbox,
      snapshotGeneration, snapshotStage, snapshotImmutable⟩
  exact handleWorker_stopVm_pausing_complete valid input foundSandbox foundVm
    pausing current live frozen fenced currentOperation foundPlan target
    snapshotFound snapshotSandbox snapshotGeneration snapshotStage
    snapshotImmutable command head transition

end SandboxModel.Controller
