import SandboxModel.Controller

/-!
Executable admission for worker-side state changes.

The worker receives an exact `EffectId`, not just a command.  Admission first
checks that this identifier is the current retry head, is still present in the
transport set, and is awaiting a result.  Command-specific checks then mirror
the corresponding `Step` constructor.  `ResourceIndexValid` is a proved state
invariant; it is used only to turn finite index scans into the global premises
required by `Step`, and is erased from generated code.
-/

namespace SandboxModel.Controller

/-- Delivery membership is transport evidence, not a decidable field of
    `World`.  Carry it with the worker invocation; the proof is erased from the
    executable reducer. -/
structure WorkerInput (world : World) where
  effect : EffectId
  queued : world.inflight effect

inductive WorkerReject where
  | staleAttempt
  | notQueued
  | notAwaiting
  | sandboxNotFound
  | resourceNotFound
  | operationNotFound
  | planNotFound
  | wrongPhase
  | terminal
  | identityMismatch
  | planMismatch
  | fenceNotReady
  | resourceBusy
  | resultAlreadyPresent
  | sourceUnavailable
  | snapshotNotReady
  | resourceStillCurrent
  deriving DecidableEq, Repr

private instance stoppableVmStatusDecidable (status : VmStatus) :
    Decidable (stoppableVmStatus status) := by
  cases status with
  | prepared => exact isTrue trivial
  | executing => exact isTrue trivial
  | frozen => exact isTrue trivial
  | stopped => exact isFalse (fun impossible => impossible)
  | reclaimed => exact isFalse (fun impossible => impossible)

private instance fenceCanAdvanceDecidable (oldFence : Option WorkerFence)
    (epoch : Epoch) (terminal : Bool) :
    Decidable (fenceCanAdvance oldFence epoch terminal) := by
  cases oldFence with
  | none => exact isTrue trivial
  | some old =>
      unfold fenceCanAdvance
      infer_instance

def vmNotExecutingCheck (world : World) (vmId : VmId) : Bool :=
  match world.vms vmId with
  | none => false
  | some vm => decide (vm.status ≠ .executing)

def indexedNoExecutingVmCheck (world : World) (sid : SandboxId) : Bool :=
  (world.resourceIndexes sid).vms.all (vmNotExecutingCheck world)

def IndexedNoExecutingVm (world : World) (sid : SandboxId) : Prop :=
  ∀ vmId, vmId ∈ (world.resourceIndexes sid).vms →
    ∃ vm, world.vms vmId = some vm ∧ vm.status ≠ .executing

theorem vmNotExecutingCheck_spec :
    vmNotExecutingCheck world vmId = true ↔
      ∃ vm, world.vms vmId = some vm ∧ vm.status ≠ .executing := by
  unfold vmNotExecutingCheck
  cases found : world.vms vmId <;> simp

theorem indexedNoExecutingVmCheck_spec :
    indexedNoExecutingVmCheck world sid = true ↔
      IndexedNoExecutingVm world sid := by
  unfold indexedNoExecutingVmCheck IndexedNoExecutingVm
  induction (world.resourceIndexes sid).vms with
  | nil => simp
  | cons vmId rest ih =>
      simp [List.all_cons, vmNotExecutingCheck_spec, ih]

theorem indexedNoExecutingVm_global_spec
    (valid : ResourceIndexValid world) :
    indexedNoExecutingVmCheck world sid = true ↔ noExecutingVm world sid := by
  rw [indexedNoExecutingVmCheck_spec]
  constructor
  · intro indexed vmId vm found owned executing
    have member := valid.2.1 sid vmId vm found owned
    rcases indexed vmId member with ⟨indexedVm, indexedFound, notExecuting⟩
    have same : indexedVm = vm := Option.some.inj (indexedFound.symm.trans found)
    exact notExecuting (same.symm ▸ executing)
  · intro global vmId member
    rcases valid.1 sid vmId member with ⟨vm, found, owned⟩
    exact ⟨vm, found, global vmId vm found owned⟩

private theorem queued_of_exact (world : World) (id : EffectId)
    (head : currentEffectId world id.command = id)
    (queued : world.inflight id) :
    deliveryQueued world id.command := by
  simpa [deliveryQueued, head] using queued

private theorem awaiting_of_exact (world : World) (id : EffectId)
    (head : currentEffectId world id.command = id)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown) :
    effectAwaiting world id.command := by
  simpa [effectAwaiting, effectHasStatus, head] using awaiting

private theorem noCurrentVm_of_lookup_none {world : World}
    {sid : SandboxId} {vmId : VmId}
    (found : world.sandboxes sid = none) :
    ∀ record, world.sandboxes sid = some record →
      record.currentVm ≠ some vmId := by
  intro record impossible
  rw [found] at impossible
  contradiction

private theorem noCurrentVm_of_lookup_some {world : World}
    {sid : SandboxId} {vmId : VmId} {current : SandboxRecord}
    (found : world.sandboxes sid = some current)
    (notCurrent : current.currentVm ≠ some vmId) :
    ∀ record, world.sandboxes sid = some record →
      record.currentVm ≠ some vmId := by
  intro record recordFound
  have same : record = current := Option.some.inj (recordFound.symm.trans found)
  simpa [same] using notCurrent

private theorem noCurrentSlot_of_lookup_none {world : World}
    {sid : SandboxId} {slotId : NetSlotId}
    (found : world.sandboxes sid = none) :
    ∀ record, world.sandboxes sid = some record →
      record.netSlot ≠ some slotId := by
  intro record impossible
  rw [found] at impossible
  contradiction

private theorem noCurrentSlot_of_lookup_some {world : World}
    {sid : SandboxId} {slotId : NetSlotId} {current : SandboxRecord}
    (found : world.sandboxes sid = some current)
    (notCurrent : current.netSlot ≠ some slotId) :
    ∀ record, world.sandboxes sid = some record →
      record.netSlot ≠ some slotId := by
  intro record recordFound
  have same : record = current := Option.some.inj (recordFound.symm.trans found)
  simpa [same] using notCurrent

/-- The prepare path is kept as a separate executable unit because restore
    validation is substantially richer than the other worker commands.  The
    outer dispatcher still supplies the exact-attempt transport and ledger
    evidence; splitting the implementation makes the corresponding
    completeness proof tractable without changing its behavior. -/
def handlePrepareVmCommand (world : World) (id : EffectId)
    (sid : SandboxId) (epoch : Epoch) (generation : Generation)
    (vmId : VmId) (slotId : NetSlotId) (source : Option RestoreSource)
    (command : id.command =
      .prepareVm sid epoch generation vmId slotId source)
    (commandQueued : deliveryQueued world id.command)
    (commandAwaiting : effectAwaiting world id.command) :
    Except WorkerReject (Decision world) := by
  match found : world.sandboxes sid with
  | none => exact .error .sandboxNotFound
  | some record =>
      if starting : record.phase = .starting then
        if live : record.terminal = false then
          if epochMatches : epoch = record.epoch then
            if generationMatches : generation = record.generation then
              match currentOperation : record.currentOperation with
              | none => exact .error .operationNotFound
              | some operation =>
                  match foundPlan : world.operationPlans
                      (operationKey sid operation.id) with
                  | none => exact .error .planNotFound
                  | some plan =>
                      if planKey : plan.key = operationKey sid operation.id then
                        if planEpoch : plan.epoch = record.epoch then
                          if planGeneration : plan.generation = record.generation then
                            if planSealed : plan.sealed = true then
                              if plannedVm : plan.vm = some vmId then
                                if plannedSlot : plan.slot = some slotId then
                                  if plannedSource : plan.restoreFrom = source then
                                    match fenced : world.workerFences sid with
                                    | none => exact .error .fenceNotReady
                                    | some fence =>
                                        if fenceMatches :
                                            fence = ⟨record.epoch, false⟩ then
                                          match freshVm : world.vms vmId with
                                          | some _ => exact .error .resultAlreadyPresent
                                          | none =>
                                              match freshSlot : world.netSlots slotId with
                                              | some _ => exact .error .resultAlreadyPresent
                                              | none =>
                                                  if selectedSource :
                                                      source = record.restoreFrom then
                                                    match source with
                                                    | none =>
                                                        have resumeSource : ∀ selected,
                                                            (none : Option RestoreSource) =
                                                              some selected →
                                                            record.durableSnapshot =
                                                                some selected ∧
                                                              usableSnapshot world sid
                                                                record selected := by
                                                          intro selected impossible
                                                          contradiction
                                                        exact .ok ⟨
                                                          .prepareVm sid vmId slotId none,
                                                          prepareResources world sid record
                                                            vmId slotId,
                                                          Step.prepareVm
                                                            (by simpa [command, epochMatches,
                                                              generationMatches] using
                                                              commandQueued)
                                                            (by simpa [command, epochMatches,
                                                              generationMatches] using
                                                              commandAwaiting)
                                                            found starting live
                                                            (by simpa using currentOperation)
                                                            foundPlan planKey planEpoch
                                                            planGeneration planSealed plannedVm
                                                            plannedSlot plannedSource
                                                            (by simpa [fenceMatches] using fenced)
                                                            freshVm freshSlot selectedSource
                                                            resumeSource⟩
                                                    | some selected =>
                                                        match durable :
                                                            record.durableSnapshot with
                                                        | none => exact .error .sourceUnavailable
                                                        | some durableSource =>
                                                            if sameSource :
                                                                durableSource = selected then
                                                              if usable : usableSnapshotCheck
                                                                  world sid record selected = true then
                                                                have usableProof :=
                                                                  usableSnapshotCheck_spec.mp usable
                                                                have resumeSource : ∀ candidate,
                                                                    (some selected :
                                                                        Option RestoreSource) =
                                                                      some candidate →
                                                                    record.durableSnapshot =
                                                                        some candidate ∧
                                                                      usableSnapshot world sid
                                                                        record candidate := by
                                                                  intro candidate same
                                                                  have candidateEq :
                                                                      candidate = selected :=
                                                                    Option.some.inj same.symm
                                                                  subst candidate
                                                                  exact ⟨by simpa [sameSource]
                                                                    using durable, usableProof⟩
                                                                exact .ok ⟨
                                                                  .prepareVm sid vmId slotId
                                                                    (some selected),
                                                                  prepareResources world sid record
                                                                    vmId slotId,
                                                                  Step.prepareVm
                                                                    (by simpa [command, epochMatches,
                                                                      generationMatches] using
                                                                      commandQueued)
                                                                    (by simpa [command, epochMatches,
                                                                      generationMatches] using
                                                                      commandAwaiting)
                                                                    found starting live
                                                                    (by simpa using currentOperation)
                                                                    foundPlan planKey planEpoch
                                                                    planGeneration planSealed
                                                                    plannedVm plannedSlot plannedSource
                                                                    (by simpa [fenceMatches] using fenced)
                                                                    freshVm freshSlot selectedSource
                                                                    resumeSource⟩
                                                              else exact .error .sourceUnavailable
                                                            else exact .error .sourceUnavailable
                                                  else exact .error .sourceUnavailable
                                        else exact .error .fenceNotReady
                                  else exact .error .planMismatch
                                else exact .error .planMismatch
                              else exact .error .planMismatch
                            else exact .error .planMismatch
                          else exact .error .planMismatch
                        else exact .error .planMismatch
                      else exact .error .planMismatch
            else exact .error .identityMismatch
          else exact .error .identityMismatch
        else exact .error .terminal
      else exact .error .wrongPhase

def handleWorkerCommand (world : World)
    (valid : ResourceIndexValid world) (id : EffectId)
    (head : currentEffectId world id.command = id)
    (queued : world.inflight id)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown) :
    Except WorkerReject (Decision world) := by
  have commandQueued : deliveryQueued world id.command :=
    queued_of_exact world id head queued
  have commandAwaiting : effectAwaiting world id.command :=
    awaiting_of_exact world id head awaiting
  match command : id.command with
  | .installFence sid epoch terminal =>
      match found : world.sandboxes sid with
      | none => exact .error .sandboxNotFound
      | some record =>
          if epochMatches : epoch = record.epoch then
            if terminalMatches : terminal = record.terminal then
              if monotone : fenceCanAdvance (world.workerFences sid)
                  record.epoch record.terminal then
                if starting : record.phase = .starting then
                  if live : record.terminal = false then
                    exact .ok ⟨.installFence sid,
                      applyStartFence world sid record,
                      Step.installStartFence
                        (by simpa [command, epochMatches,
                          terminalMatches] using commandQueued)
                        (by simpa [command, epochMatches,
                          terminalMatches] using commandAwaiting)
                        found starting live
                        (by simpa [live] using monotone)⟩
                  else exact .error .terminal
                else if revoking : record.phase = .pausing ∨
                    record.phase = .revoking then
                  match slotChoice : record.netSlot with
                  | none =>
                      have slotIdentity : ∀ slotId,
                          record.netSlot = some slotId →
                          ∃ slot, world.netSlots slotId = some slot ∧
                            slot.sandbox = sid := by
                        intro slotId impossible
                        rw [slotChoice] at impossible
                        contradiction
                      exact .ok ⟨.installFence sid,
                        applyRevocationFence world sid record,
                        Step.installRevocationFence
                          (by simpa [command, epochMatches,
                            terminalMatches] using commandQueued)
                          (by simpa [command, epochMatches,
                            terminalMatches] using commandAwaiting)
                          found revoking monotone slotIdentity⟩
                  | some slotId =>
                      match slotFound : world.netSlots slotId with
                      | none => exact .error .resourceNotFound
                      | some slot =>
                          if owned : slot.sandbox = sid then
                            have slotIdentity : ∀ candidate,
                                record.netSlot = some candidate →
                                ∃ value, world.netSlots candidate = some value ∧
                                  value.sandbox = sid := by
                              intro candidate current
                              have same : candidate = slotId :=
                                Option.some.inj (current.symm.trans slotChoice)
                              subst candidate
                              exact ⟨slot, slotFound, owned⟩
                            exact .ok ⟨.installFence sid,
                              applyRevocationFence world sid record,
                              Step.installRevocationFence
                                (by simpa [command, epochMatches,
                                  terminalMatches] using commandQueued)
                                (by simpa [command, epochMatches,
                                  terminalMatches] using commandAwaiting)
                                found revoking monotone slotIdentity⟩
                          else exact .error .identityMismatch
                else exact .error .wrongPhase
              else exact .error .fenceNotReady
            else exact .error .identityMismatch
          else exact .error .identityMismatch

  | .prepareVm sid epoch generation vmId slotId source =>
      exact handlePrepareVmCommand world id sid epoch generation vmId slotId source
        command commandQueued commandAwaiting

  | .configureNetwork sid epoch generation vmId slotId policy =>
      match foundSandbox : world.sandboxes sid with
      | none => exact .error .sandboxNotFound
      | some record =>
          if starting : record.phase = .starting then
            if live : record.terminal = false then
              if epochMatches : epoch = record.epoch then
                if generationMatches : generation = record.generation then
                  if policyMatches : policy = record.policy then
                    match foundSlot : world.netSlots slotId with
                    | none => exact .error .resourceNotFound
                    | some slot =>
                        if current : record.netSlot = some slotId then
                          if currentVm : record.currentVm = some vmId then
                            if owned : slot.owner = record.owner ∧
                                slot.sandbox = sid ∧
                                slot.generation = record.generation ∧
                                slot.vm = vmId ∧ slot.policy = record.policy then
                              if fenced : world.workerFences sid =
                                  some ⟨record.epoch, false⟩ then
                                exact .ok ⟨.configureNetwork sid slotId,
                                  configureSlot world slotId slot,
                                  Step.configureNetwork
                                    (by simpa [command, epochMatches,
                                      generationMatches, policyMatches] using commandQueued)
                                    (by simpa [command, epochMatches,
                                      generationMatches, policyMatches] using commandAwaiting)
                                    foundSandbox starting live foundSlot current
                                    currentVm owned fenced⟩
                              else exact .error .fenceNotReady
                            else exact .error .identityMismatch
                          else exact .error .identityMismatch
                        else exact .error .identityMismatch
                  else exact .error .identityMismatch
                else exact .error .identityMismatch
              else exact .error .identityMismatch
            else exact .error .terminal
          else exact .error .wrongPhase

  | .applyFilePolicy sid epoch generation workspace policy =>
      match found : world.sandboxes sid with
      | none => exact .error .sandboxNotFound
      | some record =>
          if starting : record.phase = .starting then
            if live : record.terminal = false then
              if epochMatches : epoch = record.epoch then
                if generationMatches : generation = record.generation then
                  if workspaceMatches : workspace = record.workspace then
                    if policyMatches : policy = record.filePolicy then
                      if fenced : world.workerFences sid = some ⟨record.epoch, false⟩ then
                        exact .ok ⟨.applyFilePolicy sid record.generation,
                          applyFilePolicyResult world sid record,
                          Step.applyFilePolicy
                            (by simpa [command, epochMatches, generationMatches,
                              workspaceMatches, policyMatches] using commandQueued)
                            (by simpa [command, epochMatches, generationMatches,
                              workspaceMatches, policyMatches] using commandAwaiting)
                            found starting live fenced⟩
                      else exact .error .fenceNotReady
                    else exact .error .identityMismatch
                  else exact .error .identityMismatch
                else exact .error .identityMismatch
              else exact .error .identityMismatch
            else exact .error .terminal
          else exact .error .wrongPhase

  | .activateVm sid epoch generation vmId =>
      match foundSandbox : world.sandboxes sid with
      | none => exact .error .sandboxNotFound
      | some record =>
          if starting : record.phase = .starting then
            if live : record.terminal = false then
              if epochMatches : epoch = record.epoch then
                if generationMatches : generation = record.generation then
                  if current : record.currentVm = some vmId then
                    match foundVm : world.vms vmId with
                    | none => exact .error .resourceNotFound
                    | some vm =>
                        if ownedVm : vm.owner = record.owner ∧ vm.sandbox = sid ∧
                            vm.generation = record.generation then
                          if prepared : vm.status = .prepared then
                            match slotChoice : record.netSlot with
                            | none => exact .error .resourceNotFound
                            | some slotId =>
                                match foundSlot : world.netSlots slotId with
                                | none => exact .error .resourceNotFound
                                | some slot =>
                                    if slotReady : slot.vm = vmId ∧ slot.configured = true then
                                      if fenced : world.workerFences sid =
                                          some ⟨record.epoch, false⟩ then
                                        if exclusiveCheck : indexedNoExecutingVmCheck world sid = true then
                                          have exclusive :=
                                            (indexedNoExecutingVm_global_spec valid).mp exclusiveCheck
                                          have foundSlotProof : ∃ candidate value,
                                              record.netSlot = some candidate ∧
                                              world.netSlots candidate = some value ∧
                                              value.vm = vmId ∧ value.configured = true :=
                                            ⟨slotId, slot, slotChoice, foundSlot,
                                              slotReady.1, slotReady.2⟩
                                          exact .ok ⟨.activateVm sid vmId,
                                            activateResource world vmId vm,
                                            Step.activateVm
                                              (by simpa [command, epochMatches,
                                                generationMatches] using commandQueued)
                                              (by simpa [command, epochMatches,
                                                generationMatches] using commandAwaiting)
                                              foundSandbox starting live current foundVm
                                              ownedVm prepared foundSlotProof fenced exclusive⟩
                                        else exact .error .resourceBusy
                                      else exact .error .fenceNotReady
                                    else exact .error .identityMismatch
                          else exact .error .wrongPhase
                        else exact .error .identityMismatch
                  else exact .error .identityMismatch
                else exact .error .identityMismatch
              else exact .error .identityMismatch
            else exact .error .terminal
          else exact .error .wrongPhase

  | .freezeVm sid epoch generation vmId =>
      match foundSandbox : world.sandboxes sid with
      | none => exact .error .sandboxNotFound
      | some record =>
          if pausing : record.phase = .pausing then
            if epochMatches : epoch = record.epoch then
              if generationMatches : generation = record.generation then
                if current : record.currentVm = some vmId then
                  match foundVm : world.vms vmId with
                  | none => exact .error .resourceNotFound
                  | some vm =>
                      if vmIdentity : vm.owner = record.owner ∧ vm.sandbox = sid ∧
                          vm.generation = record.generation then
                        if executing : vm.status = .executing then
                          if fenced : world.workerFences sid = some ⟨record.epoch, false⟩ then
                            exact .ok ⟨.freezeVm sid vmId,
                              setVm world vmId { vm with status := .frozen },
                              Step.freezeVm
                                (by simpa [command, epochMatches,
                                  generationMatches] using commandQueued)
                                (by simpa [command, epochMatches,
                                  generationMatches] using commandAwaiting)
                                foundSandbox pausing current foundVm vmIdentity
                                executing fenced⟩
                          else exact .error .fenceNotReady
                        else exact .error .wrongPhase
                      else exact .error .identityMismatch
                else exact .error .identityMismatch
              else exact .error .identityMismatch
            else exact .error .identityMismatch
          else exact .error .wrongPhase

  | .sealSnapshot sid epoch generation vmId workspace snapshotId manifest =>
      match foundSandbox : world.sandboxes sid with
      | none => exact .error .sandboxNotFound
      | some record =>
          if pausing : record.phase = .pausing then
            if epochMatches : epoch = record.epoch then
              if generationMatches : generation = record.generation then
                if workspaceMatches : workspace = record.workspace then
                  match currentOperation : record.currentOperation with
                  | none => exact .error .operationNotFound
                  | some operation =>
                      match foundPlan : world.operationPlans
                          (operationKey sid operation.id) with
                      | none => exact .error .planNotFound
                      | some plan =>
                          if planKey : plan.key = operationKey sid operation.id then
                            if planEpoch : plan.epoch = record.epoch then
                              if planGeneration : plan.generation = record.generation then
                                if planSealed : plan.sealed = true then
                                  if plannedVm : plan.vm = some vmId then
                                    if plannedSnapshot : plan.targetSnapshot =
                                        some ⟨snapshotId, manifest⟩ then
                                      match foundVm : world.vms vmId with
                                      | none => exact .error .resourceNotFound
                                      | some vm =>
                                          if current : record.currentVm = some vmId then
                                            if vmIdentity : vm.owner = record.owner ∧
                                                vm.sandbox = sid ∧
                                                vm.generation = record.generation then
                                              if frozen : vm.status = .frozen then
                                                if fenced : world.workerFences sid =
                                                    some ⟨record.epoch, false⟩ then
                                                  match fresh : world.snapshots snapshotId with
                                                  | some _ => exact .error .resultAlreadyPresent
                                                  | none =>
                                                      exact .ok ⟨.sealSnapshot sid snapshotId manifest,
                                                        sealSnapshot world snapshotId manifest record sid,
                                                        Step.sealSnapshot
                                                          (by simpa [command, epochMatches,
                                                            generationMatches,
                                                            workspaceMatches] using commandQueued)
                                                          (by simpa [command, epochMatches,
                                                            generationMatches,
                                                            workspaceMatches] using commandAwaiting)
                                                          foundSandbox pausing
                                                          (by simpa using currentOperation)
                                                          foundPlan planKey planEpoch
                                                          planGeneration planSealed plannedVm
                                                          plannedSnapshot foundVm current
                                                          vmIdentity frozen fenced fresh⟩
                                                else exact .error .fenceNotReady
                                              else exact .error .wrongPhase
                                            else exact .error .identityMismatch
                                          else exact .error .identityMismatch
                                    else exact .error .planMismatch
                                  else exact .error .planMismatch
                                else exact .error .planMismatch
                              else exact .error .planMismatch
                            else exact .error .planMismatch
                          else exact .error .planMismatch
                else exact .error .identityMismatch
              else exact .error .identityMismatch
            else exact .error .identityMismatch
          else exact .error .wrongPhase

  | .commitSnapshot sid epoch generation snapshotId manifest =>
      match found : world.snapshots snapshotId with
      | none => exact .error .resourceNotFound
      | some snapshot =>
          match foundSandbox : world.sandboxes sid with
          | none => exact .error .sandboxNotFound
          | some record =>
              if epochMatches : epoch = record.epoch then
                if generationMatches : generation = record.generation then
                  if manifestMatches : manifest = snapshot.manifest then
                    if snapshotSandbox : snapshot.sandbox = sid then
                      if pausing : record.phase = .pausing then
                        match current : record.currentVm with
                        | none => exact .error .resourceNotFound
                        | some vmId =>
                            if snapshotIdentity :
                                snapshot.sourceGeneration = record.generation ∧
                                  snapshot.workspace = record.workspace then
                              if sealed : snapshot.stage = .localSealed then
                                exact .ok ⟨.commitSnapshot snapshotId,
                                  enqueue (makeSnapshotDurable world snapshotId snapshot)
                                    (.stopVm sid record.epoch record.generation vmId),
                                  Step.commitSnapshot
                                    (by simpa [command, epochMatches,
                                      generationMatches, manifestMatches] using commandQueued)
                                    (by simpa [command, epochMatches,
                                      generationMatches, manifestMatches] using commandAwaiting)
                                    found foundSandbox snapshotSandbox pausing
                                    (by simpa using current) snapshotIdentity sealed⟩
                              else exact .error .snapshotNotReady
                            else exact .error .identityMismatch
                      else exact .error .wrongPhase
                    else exact .error .identityMismatch
                  else exact .error .identityMismatch
                else exact .error .identityMismatch
              else exact .error .identityMismatch

  | .stopVm sid epoch generation vmId =>
      match foundSandbox : world.sandboxes sid with
      | none => exact .error .sandboxNotFound
      | some record =>
          match found : world.vms vmId with
          | none => exact .error .resourceNotFound
          | some vm =>
              if epochMatches : epoch = record.epoch then
                if generationMatches : generation = vm.generation then
                  if owned : vm.sandbox = sid then
                    if stoppable : stoppableVmStatus vm.status then
                      if pausing : record.phase = .pausing then
                        if current : record.currentVm = some vmId then
                          if live : record.terminal = false then
                            if frozen : vm.status = .frozen then
                              if fenced : world.workerFences sid =
                                  some ⟨record.epoch, false⟩ then
                                match operationChoice : record.currentOperation with
                                | none => exact .error .operationNotFound
                                | some operation =>
                                    match foundPlan : world.operationPlans
                                        (operationKey sid operation.id) with
                                    | none => exact .error .planNotFound
                                    | some plan =>
                                        match target : plan.targetSnapshot with
                                        | none => exact .error .snapshotNotReady
                                        | some source =>
                                            match snapshotFound : world.snapshots source.snapshot with
                                            | none => exact .error .snapshotNotReady
                                            | some snapshot =>
                                                if ready : snapshot.sandbox = sid ∧
                                                    snapshot.sourceGeneration = record.generation ∧
                                                    snapshot.stage = .durable ∧
                                                    snapshot.immutable = true then
                                                  have authorizedStop :
                                                      (record.phase = .pausing ∧
                                                        record.currentVm = some vmId ∧
                                                        record.terminal = false ∧
                                                        vm.status = .frozen ∧
                                                        world.workerFences sid =
                                                          some ⟨record.epoch, false⟩ ∧
                                                        ∃ operation plan source snapshot,
                                                          record.currentOperation =
                                                            some operation ∧
                                                          world.operationPlans
                                                              (operationKey sid
                                                                operation.id) =
                                                            some plan ∧
                                                          plan.targetSnapshot =
                                                            some source ∧
                                                          world.snapshots
                                                              source.snapshot =
                                                            some snapshot ∧
                                                          snapshot.sandbox = sid ∧
                                                          snapshot.sourceGeneration = record.generation ∧
                                                          snapshot.stage = .durable ∧
                                                          snapshot.immutable = true) ∨
                                                      (record.phase = .revoking ∧
                                                        record.terminal = true ∧
                                                        world.workerFences sid =
                                                          some ⟨record.epoch, true⟩) :=
                                                    Or.inl ⟨pausing, current, live, frozen,
                                                      fenced, operation, plan, source, snapshot,
                                                      operationChoice, foundPlan, target,
                                                      snapshotFound, ready⟩
                                                  exact .ok ⟨.stopVm sid vmId,
                                                    stopResource world vmId vm,
                                                    Step.stopVm
                                                      (by simpa [command, epochMatches,
                                                        generationMatches] using commandQueued)
                                                      (by simpa [command, epochMatches,
                                                        generationMatches] using commandAwaiting)
                                                      foundSandbox found owned stoppable
                                                      authorizedStop⟩
                                                else exact .error .snapshotNotReady
                              else exact .error .fenceNotReady
                            else exact .error .wrongPhase
                          else exact .error .terminal
                        else exact .error .identityMismatch
                      else if revoking : record.phase = .revoking then
                        if terminalState : record.terminal = true then
                          if fenced : world.workerFences sid =
                              some ⟨record.epoch, true⟩ then
                            exact .ok ⟨.stopVm sid vmId,
                              stopResource world vmId vm,
                              Step.stopVm
                                (by simpa [command, epochMatches,
                                  generationMatches] using commandQueued)
                                (by simpa [command, epochMatches,
                                  generationMatches] using commandAwaiting)
                                foundSandbox found owned stoppable
                                (Or.inr ⟨revoking, terminalState, fenced⟩)⟩
                          else exact .error .fenceNotReady
                        else exact .error .terminal
                      else exact .error .wrongPhase
                    else exact .error .wrongPhase
                  else exact .error .identityMismatch
                else exact .error .identityMismatch
              else exact .error .identityMismatch

  | .cleanupVm sid generation vmId =>
      match foundVm : world.vms vmId with
      | none => exact .error .resourceNotFound
      | some vm =>
          if identity : vm.sandbox = sid ∧ vm.generation = generation then
            if stopped : vm.status = .stopped then
              match sandboxFound : world.sandboxes sid with
              | none =>
                  have notCurrent := noCurrentVm_of_lookup_none
                    (vmId := vmId) sandboxFound
                  exact .ok ⟨.cleanupVm sid generation vmId,
                    reclaimVm world vmId vm,
                    Step.cleanupVm
                      (by simpa [command] using commandQueued)
                      (by simpa [command] using commandAwaiting)
                      foundVm identity stopped notCurrent⟩
              | some record =>
                  if pointerClear : record.currentVm ≠ some vmId then
                    have notCurrent := noCurrentVm_of_lookup_some
                      (vmId := vmId) sandboxFound pointerClear
                    exact .ok ⟨.cleanupVm sid generation vmId,
                      reclaimVm world vmId vm,
                      Step.cleanupVm
                        (by simpa [command] using commandQueued)
                        (by simpa [command] using commandAwaiting)
                        foundVm identity stopped notCurrent⟩
                  else exact .error .resourceStillCurrent
            else exact .error .wrongPhase
          else exact .error .identityMismatch

  | .cleanupSlot sid generation slotId =>
      match foundSlot : world.netSlots slotId with
      | none => exact .error .resourceNotFound
      | some slot =>
          if identity : slot.sandbox = sid ∧ slot.generation = generation then
            if inactive : slot.active = false then
              if notReclaimed : slot.reclaimed = false then
                match sandboxFound : world.sandboxes sid with
                | none =>
                    have notCurrent := noCurrentSlot_of_lookup_none
                      (slotId := slotId) sandboxFound
                    exact .ok ⟨.cleanupSlot sid generation slotId,
                      reclaimSlot world slotId slot,
                      Step.cleanupSlot
                        (by simpa [command] using commandQueued)
                        (by simpa [command] using commandAwaiting)
                        foundSlot identity inactive notReclaimed notCurrent⟩
                | some record =>
                    if pointerClear : record.netSlot ≠ some slotId then
                      have notCurrent := noCurrentSlot_of_lookup_some
                        (slotId := slotId) sandboxFound pointerClear
                      exact .ok ⟨.cleanupSlot sid generation slotId,
                        reclaimSlot world slotId slot,
                        Step.cleanupSlot
                          (by simpa [command] using commandQueued)
                          (by simpa [command] using commandAwaiting)
                          foundSlot identity inactive notReclaimed notCurrent⟩
                    else exact .error .resourceStillCurrent
              else exact .error .resultAlreadyPresent
            else exact .error .resourceBusy
          else exact .error .identityMismatch

def handleWorker (world : World) (valid : ResourceIndexValid world)
    (input : WorkerInput world) : Except WorkerReject (Decision world) :=
  let id := input.effect
  if head : currentEffectId world id.command = id then
    if awaiting : world.effects id = some .dispatched ∨
        world.effects id = some .outcomeUnknown then
      handleWorkerCommand world valid id head input.queued awaiting
    else .error .notAwaiting
  else .error .staleAttempt

end SandboxModel.Controller
