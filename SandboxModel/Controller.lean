import SandboxModel.Proofs
import SandboxModel.ExecutableChecks

/-!
Executable controller admission core.

`Step` remains the authoritative relational specification.  `handle` is the
deterministic controller-owned subset: accepting create/pause/resume/kill and
starting compensation.  Every successful `Decision` carries a proof that its
event and successor are a legal `Step`; the proof is erased when executable
code is generated.

Worker observations use the same evidence-carrying decision type below.  A
successful receipt is accepted only when its command-specific observable
postcondition is present; a failed receipt carries no invented success state.
-/

namespace SandboxModel.Controller

instance authorizedDecidable (actor : Actor) (owner : TenantId) :
    Decidable (authorized actor owner) := by
  unfold authorized
  infer_instance

/-- State records intentionally avoid global equality instances.  The
    controller only needs structural equality for the two-field fence token. -/
instance workerFenceDecidableEq : DecidableEq WorkerFence := by
  intro left right
  cases left with
  | mk leftEpoch leftTerminal =>
      cases right with
      | mk rightEpoch rightTerminal =>
          by_cases sameEpoch : leftEpoch = rightEpoch
          · by_cases sameTerminal : leftTerminal = rightTerminal
            · exact isTrue (by simp [sameEpoch, sameTerminal])
            · exact isFalse (by simp [sameEpoch, sameTerminal])
          · exact isFalse (by simp [sameEpoch])

/-- Executable form of `usableSnapshot`; the relational predicate contains an
    existential, while the concrete snapshot key identifies the only candidate
    that needs to be inspected. -/
def usableSnapshotCheck (world : World) (sid : SandboxId)
    (record : SandboxRecord) (source : RestoreSource) : Bool :=
  match world.snapshots source.snapshot with
  | none => false
  | some snapshot => decide (
      snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
      snapshot.workspace = record.workspace ∧
      snapshot.manifest = source.manifest ∧
      snapshot.stage = .durable ∧ snapshot.immutable = true)

theorem usableSnapshotCheck_spec :
    usableSnapshotCheck world sid record source = true ↔
      usableSnapshot world sid record source := by
  unfold usableSnapshotCheck usableSnapshot
  cases found : world.snapshots source.snapshot <;> simp

inductive Request where
  | create (actor : Actor) (owner : TenantId) (sid : SandboxId)
      (workspace : FsId) (filePolicy : FilePolicyRef)
      (policy : NetworkPolicy) (operation : OperationId)
  | pause (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | resume (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | kill (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | replay (actor : Actor) (sid : SandboxId) (operation : OperationId)
      (kind : OperationKind) (input : OperationInput)
  | query (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | compensate (sid : SandboxId) (failure : FailureKind)
      (evidence : Option EffectId)
  deriving DecidableEq, Repr

inductive Reject where
  | controllerDown
  | sandboxExists
  | sandboxNotFound
  | workspaceExists
  | operationExists
  | operationPlanExists
  | unauthorized
  | wrongPhase
  | terminal
  | noCurrentVm
  | snapshotUnavailable
  | operationUnavailable
  | operationNotCompensable
  | operationAlreadyFinal
  | ledgerMismatch
  | failureUnjustified
  deriving DecidableEq, Repr

/-- A successful reducer result is evidence carrying.  `legal` is erased by
    code generation but prevents any unproved state mutation from being
    represented as an accepted controller decision. -/
structure Decision (before : World) where
  event : Event
  next : World
  legal : Step before event next

def handleCreate (world : World) (actor : Actor) (owner : TenantId)
    (sid : SandboxId) (workspace : FsId) (filePolicy : FilePolicyRef)
    (policy : NetworkPolicy) (operationId : OperationId) :
    Except Reject (Decision world) :=
  if up : world.controllerUp = true then
    match sandboxFound : world.sandboxes sid with
    | some _ => .error .sandboxExists
    | none =>
        match workspaceFound : world.filesystems workspace with
        | some _ => .error .workspaceExists
        | none =>
            match operationFound : world.operations (operationKey sid operationId) with
            | some _ => .error .operationExists
            | none =>
                match planFound : world.operationPlans
                    (operationKey sid operationId) with
                | some _ => .error .operationPlanExists
                | none =>
                    if auth : authorized actor owner then
                      .ok ⟨.acceptCreate actor owner sid workspace filePolicy policy operationId,
                        beginCreate world sid owner workspace filePolicy policy operationId,
                        Step.acceptCreate up sandboxFound workspaceFound
                          operationFound planFound auth⟩
                    else .error .unauthorized
  else .error .controllerDown

def handlePause (world : World) (actor : Actor) (sid : SandboxId)
    (operationId : OperationId) : Except Reject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if running : record.phase = .running then
          if live : record.terminal = false then
            match current : record.currentVm with
            | none => .error .noCurrentVm
            | some vmId =>
                match fresh : world.operations (operationKey sid operationId) with
                | some _ => .error .operationExists
                | none =>
                    match freshPlan : world.operationPlans
                        (operationKey sid operationId) with
                    | some _ => .error .operationPlanExists
                    | none =>
                        if auth : authorized actor record.owner then
                          .ok ⟨.acceptPause actor sid operationId,
                            beginPause world sid record operationId vmId,
                            Step.acceptPause up found running live current fresh
                              freshPlan auth⟩
                        else .error .unauthorized
          else .error .terminal
        else .error .wrongPhase
  else .error .controllerDown

def handleResume (world : World) (actor : Actor) (sid : SandboxId)
    (operationId : OperationId) : Except Reject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if paused : record.phase = .paused then
          if live : record.terminal = false then
            match currentSnapshot : record.durableSnapshot with
            | none => .error .snapshotUnavailable
            | some source =>
                if checked : usableSnapshotCheck world sid record source = true then
                  let sourceValid := usableSnapshotCheck_spec.mp checked
                  match fresh : world.operations (operationKey sid operationId) with
                  | some _ => .error .operationExists
                  | none =>
                      match freshPlan : world.operationPlans
                          (operationKey sid operationId) with
                      | some _ => .error .operationPlanExists
                      | none =>
                          if auth : authorized actor record.owner then
                            .ok ⟨.acceptResume actor sid operationId,
                              beginResume world sid record operationId source,
                              Step.acceptResume up found paused live currentSnapshot
                                sourceValid fresh freshPlan auth⟩
                          else .error .unauthorized
                else .error .snapshotUnavailable
          else .error .terminal
        else .error .wrongPhase
  else .error .controllerDown

def supersedeReadyCheck (world : World) (sid : SandboxId) :
    Option Operation → Bool
  | none => true
  | some operation =>
      match operation.status with
      | .pending | .outcomeUnknown =>
          match world.operations (operationKey sid operation.id) with
          | none => false
          | some ledger => decide
              (ledger.operation = operation ∧ ledger.result = none)
      | .compensating _ | .succeeded | .failed _ => true

theorem supersedeReadyCheck_spec (world : World) (sid : SandboxId)
    (current : Option Operation) :
    supersedeReadyCheck world sid current = true ↔
      SupersedeReady world sid current := by
  cases current with
  | none => simp [supersedeReadyCheck, SupersedeReady]
  | some operation =>
      cases operation with
      | mk operationId epoch kind status =>
          cases status <;>
            simp [supersedeReadyCheck, SupersedeReady]
          all_goals
            cases found : world.operations (operationKey sid operationId) <;>
              simp

def handleKill (world : World) (actor : Actor) (sid : SandboxId)
    (operationId : OperationId) : Except Reject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if notDestroyed : record.phase ≠ .destroyed then
          if live : record.terminal = false then
            if supersedeReady : supersedeReadyCheck world sid
                record.currentOperation = true then
              match fresh : world.operations (operationKey sid operationId) with
              | some _ => .error .operationExists
              | none =>
                  match freshPlan : world.operationPlans
                      (operationKey sid operationId) with
                  | some _ => .error .operationPlanExists
                  | none =>
                      if auth : authorized actor record.owner then
                        .ok ⟨.acceptKill actor sid operationId,
                          beginKill world sid record operationId,
                          Step.acceptKill up found notDestroyed live
                            ((supersedeReadyCheck_spec world sid
                              record.currentOperation).mp supersedeReady)
                            fresh freshPlan auth⟩
                      else .error .unauthorized
            else .error .ledgerMismatch
          else .error .terminal
        else .error .terminal
  else .error .controllerDown

def handleReplay (world : World) (actor : Actor) (sid : SandboxId)
    (operationId : OperationId) (kind : OperationKind) (input : OperationInput) :
    Except Reject (Decision world) :=
  match foundSandbox : world.sandboxes sid with
  | none => .error .sandboxNotFound
  | some record =>
      if auth : authorized actor record.owner then
        match foundOperation : world.operations (operationKey sid operationId) with
        | none => .error .operationUnavailable
        | some ledger =>
            if sameKind : ledger.operation.kind = kind then
              if sameInput : ledger.input = input then
                .ok ⟨.retryOperation actor sid operationId kind input, world,
                  Step.retryOperation foundSandbox auth foundOperation
                    sameKind sameInput⟩
              else .error .ledgerMismatch
            else .error .ledgerMismatch
      else .error .unauthorized

def handleQuery (world : World) (actor : Actor) (sid : SandboxId)
    (operationId : OperationId) : Except Reject (Decision world) :=
  match foundSandbox : world.sandboxes sid with
  | none => .error .sandboxNotFound
  | some record =>
      if auth : authorized actor record.owner then
        match foundOperation : world.operations (operationKey sid operationId) with
        | none => .error .operationUnavailable
        | some ledger =>
            .ok ⟨.queryOperation actor sid operationId ledger.operation.status
                ledger.result,
              world,
              Step.queryOperation foundSandbox auth foundOperation rfl rfl⟩
      else .error .unauthorized

/-- Evidence supplied to the executable controller is finite and explicit;
    the relational specification may state the same condition existentially. -/
def SuppliedCompensationEvidence (world : World) (key : OperationKey)
    (epoch : Epoch) (failure : FailureKind) : Option EffectId → Prop
  | none => failure = .cancelled ∨ failure = .deadlineExceeded
  | some id => FinalFailedAttemptEvidence world key epoch failure id

instance commandBelongsToOperationDecidable (command : Command)
    (sid : SandboxId) (epoch : Epoch) :
    Decidable (commandBelongsToOperation command sid epoch) := by
  cases command <;> simp [commandBelongsToOperation] <;> infer_instance

instance finalFailedAttemptEvidenceDecidable (world : World)
    (key : OperationKey) (epoch : Epoch) (failure : FailureKind)
    (id : EffectId) :
    Decidable (FinalFailedAttemptEvidence world key epoch failure id) := by
  unfold FinalFailedAttemptEvidence
  cases found : world.logicalEffects id.logical <;>
    simp [found] <;> infer_instance

instance suppliedCompensationEvidenceDecidable (world : World)
    (key : OperationKey) (epoch : Epoch) (failure : FailureKind)
    (evidence : Option EffectId) :
    Decidable (SuppliedCompensationEvidence world key epoch failure evidence) := by
  cases evidence <;> simp [SuppliedCompensationEvidence] <;> infer_instance

def compensationEvidenceCheck (world : World) (key : OperationKey)
    (epoch : Epoch) (failure : FailureKind) (evidence : Option EffectId) : Bool :=
  decide (SuppliedCompensationEvidence world key epoch failure evidence)

theorem compensationEvidenceCheck_spec (world : World) (key : OperationKey)
    (epoch : Epoch) (failure : FailureKind) (evidence : Option EffectId) :
    compensationEvidenceCheck world key epoch failure evidence = true ↔
      SuppliedCompensationEvidence world key epoch failure evidence := by
  simp [compensationEvidenceCheck]

theorem suppliedCompensationEvidence_sound
    (world : World) (key : OperationKey) (epoch : Epoch)
    (failure : FailureKind) (evidence : Option EffectId)
    (supplied : SuppliedCompensationEvidence world key epoch failure evidence) :
    CompensationEvidence world key epoch failure := by
  cases evidence with
  | none =>
      rcases supplied with cancelled | deadline
      · exact Or.inl cancelled
      · exact Or.inr (Or.inl deadline)
  | some id =>
      exact Or.inr (Or.inr ⟨id, supplied⟩)

def handleCompensation (world : World) (sid : SandboxId)
    (failure : FailureKind) (evidence : Option EffectId) :
    Except Reject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if compensablePhase : record.phase = .starting ∨
            record.phase = .pausing then
          if live : record.terminal = false then
            match currentOperation : record.currentOperation with
            | none => .error .operationUnavailable
            | some operation =>
                if recoverableKind : operation.kind = .create ∨
                    operation.kind = .resume ∨ operation.kind = .pause then
                  if unfinished : operation.status = .pending ∨
                      operation.status = .outcomeUnknown then
                    match persistedOperation :
                        world.operations (operationKey sid operation.id) with
                    | none => .error .operationUnavailable
                    | some ledger =>
                        if operationMatches : ledger.sandbox = sid ∧
                            ledger.operation = operation ∧ ledger.result = none then
                          if justified : compensationEvidenceCheck world
                              (operationKey sid operation.id) operation.epoch
                              failure evidence = true then
                            .ok ⟨.beginCompensation sid failure,
                              beginCompensation world sid record operation failure,
                              Step.beginCompensation failure up found
                                compensablePhase live
                                currentOperation recoverableKind unfinished
                                persistedOperation operationMatches
                                (suppliedCompensationEvidence_sound
                                  world (operationKey sid operation.id)
                                  operation.epoch failure evidence
                                  ((compensationEvidenceCheck_spec world
                                    (operationKey sid operation.id)
                                    operation.epoch failure evidence).mp
                                      justified))⟩
                          else .error .failureUnjustified
                        else .error .ledgerMismatch
                  else .error .operationAlreadyFinal
                else .error .operationNotCompensable
          else .error .terminal
        else .error .wrongPhase
  else .error .controllerDown

def handle (world : World) : Request → Except Reject (Decision world)
  | .create actor owner sid workspace filePolicy policy operationId =>
      handleCreate world actor owner sid workspace filePolicy policy operationId
  | .pause actor sid operationId => handlePause world actor sid operationId
  | .resume actor sid operationId => handleResume world actor sid operationId
  | .kill actor sid operationId => handleKill world actor sid operationId
  | .replay actor sid operationId kind input =>
      handleReplay world actor sid operationId kind input
  | .query actor sid operationId => handleQuery world actor sid operationId
  | .compensate sid failure evidence =>
      handleCompensation world sid failure evidence

theorem accepted_is_legal
    (_accepted : handle world request = .ok decision) :
    Step world decision.event decision.next :=
  decision.legal

theorem accepted_preserves_safe
    (safe : Safe world)
    (accepted : handle world request = .ok decision) : Safe decision.next :=
  step_preserves_safe safe (accepted_is_legal accepted)

theorem accepted_is_protocol_secure
    (accepted : handle world request = .ok decision) :
    ProtocolStepSecurity world decision.event decision.next :=
  step_protocol_secure (accepted_is_legal accepted)

/-! ## Durable effect result reducer -/

/-- Executable counterpart of `commandResultPresent`.  Each existential in
    the specification is resolved by the concrete resource key carried in the
    command. -/
def commandResultPresentCheck (world : World) : Command → Bool
  | .installFence sid epoch terminal =>
      match world.workerFences sid with
      | none => false
      | some fence => decide
          (epoch ≤ fence.epoch ∧ (terminal = true → fence.terminal = true))
  | .prepareVm sid _ generation vmId slotId _ =>
      match world.vms vmId, world.netSlots slotId with
      | some vm, some slot => decide
          (vm.sandbox = sid ∧ vm.generation = generation ∧
            slot.sandbox = sid ∧ slot.generation = generation ∧
            slot.vm = vmId)
      | _, _ => false
  | .configureNetwork sid _ generation vmId slotId policy =>
      match world.netSlots slotId with
      | none => false
      | some slot => decide
          (slot.sandbox = sid ∧ slot.generation = generation ∧
            slot.vm = vmId ∧ slot.policy = policy ∧ slot.configured = true)
  | .activateVm sid _ generation vmId =>
      match world.vms vmId with
      | none => false
      | some vm => decide
          (vm.sandbox = sid ∧ vm.generation = generation ∧
            (vm.status = .executing ∨ vm.status = .frozen ∨
              vm.status = .stopped ∨ vm.status = .reclaimed))
  | .freezeVm sid _ generation vmId =>
      match world.vms vmId with
      | none => false
      | some vm => decide
          (vm.sandbox = sid ∧ vm.generation = generation ∧
            (vm.status = .frozen ∨ vm.status = .stopped ∨
              vm.status = .reclaimed))
  | .applyFilePolicy sid _ generation workspace policy =>
      match world.appliedFilePolicies sid with
      | none => false
      | some applied => decide
          (applied.sandbox = sid ∧ generation ≤ applied.generation ∧
            applied.workspace = workspace ∧ applied.policy = policy)
  | .sealSnapshot sid _ generation _ workspace snapshotId manifest =>
      match world.snapshots snapshotId with
      | none => false
      | some snapshot => decide
          (snapshot.sandbox = sid ∧ snapshot.sourceGeneration = generation ∧
            snapshot.workspace = workspace ∧ snapshot.manifest = manifest)
  | .commitSnapshot sid _ generation snapshotId manifest =>
      match world.snapshots snapshotId with
      | none => false
      | some snapshot => decide
          (snapshot.sandbox = sid ∧ snapshot.sourceGeneration = generation ∧
            snapshot.manifest = manifest ∧ snapshot.stage = .durable ∧
            snapshot.immutable = true)
  | .stopVm sid _ generation vmId =>
      match world.vms vmId with
      | none => false
      | some vm => decide
          (vm.sandbox = sid ∧ vm.generation = generation ∧
            (vm.status = .stopped ∨ vm.status = .reclaimed))
  | .cleanupVm sid generation vmId =>
      match world.vms vmId with
      | none => false
      | some vm => decide
          (vm.sandbox = sid ∧ vm.generation = generation ∧
            vm.status = .reclaimed)
  | .cleanupSlot sid generation slotId =>
      match world.netSlots slotId with
      | none => false
      | some slot => decide
          (slot.sandbox = sid ∧ slot.generation = generation ∧
            slot.reclaimed = true)

theorem commandResultPresentCheck_spec :
    commandResultPresentCheck world command = true ↔
      commandResultPresent world command := by
  cases command with
  | installFence sid epoch terminal =>
      cases found : world.workerFences sid with
      | none => simp [commandResultPresentCheck, commandResultPresent, found]
      | some fence =>
          cases terminal <;>
            simp [commandResultPresentCheck, commandResultPresent, found]
  | prepareVm sid epoch generation vmId slotId source =>
      cases foundVm : world.vms vmId <;>
        cases foundSlot : world.netSlots slotId <;>
        simp [commandResultPresentCheck, commandResultPresent, foundVm,
          foundSlot]
  | configureNetwork sid epoch generation vmId slotId policy =>
      cases found : world.netSlots slotId <;>
        simp [commandResultPresentCheck, commandResultPresent, found]
  | activateVm sid epoch generation vmId =>
      cases found : world.vms vmId <;>
        simp [commandResultPresentCheck, commandResultPresent, found]
  | freezeVm sid epoch generation vmId =>
      cases found : world.vms vmId <;>
        simp [commandResultPresentCheck, commandResultPresent, found]
  | applyFilePolicy sid epoch generation workspace policy =>
      cases found : world.appliedFilePolicies sid <;>
        simp [commandResultPresentCheck, commandResultPresent, found]
  | sealSnapshot sid epoch generation vm workspace snapshotId manifest =>
      cases found : world.snapshots snapshotId <;>
        simp [commandResultPresentCheck, commandResultPresent, found]
  | commitSnapshot sid epoch generation snapshotId manifest =>
      cases found : world.snapshots snapshotId <;>
        simp [commandResultPresentCheck, commandResultPresent, found]
  | stopVm sid epoch generation vmId =>
      cases found : world.vms vmId <;>
        simp [commandResultPresentCheck, commandResultPresent, found]
  | cleanupVm sid generation vmId =>
      cases found : world.vms vmId <;>
        simp [commandResultPresentCheck, commandResultPresent, found]
  | cleanupSlot sid generation slotId =>
      cases found : world.netSlots slotId <;>
        simp [commandResultPresentCheck, commandResultPresent, found]

def effectOutcomeValidCheck (world : World) (command : Command) :
    EffectOutcome → Bool
  | .succeeded _ => commandResultPresentCheck world command
  | .failed _ => !(commandResultPresentCheck world command)

theorem effectOutcomeValidCheck_spec :
    effectOutcomeValidCheck world command outcome = true ↔
      effectOutcomeValid world command outcome := by
  cases outcome with
  | succeeded result =>
      simp [effectOutcomeValidCheck, effectOutcomeValid,
        commandResultPresentCheck_spec]
  | failed failure =>
      unfold effectOutcomeValidCheck effectOutcomeValid
      constructor
      · intro checked present
        have success : commandResultPresentCheck world command = true :=
          commandResultPresentCheck_spec.mpr present
        simp [success] at checked
      · intro absent
        cases checked : commandResultPresentCheck world command with
        | false => rfl
        | true =>
            exact False.elim (absent (commandResultPresentCheck_spec.mp checked))

inductive EffectInput where
  | dispatch (effect : EffectId)
  | receipt (effect : EffectId) (outcome : EffectOutcome)
  | timeout (effect : EffectId)
  /-- Start the next exact attempt of an unfinished logical action.  The
      current and next attempt identities are derived from durable state, so
      callers cannot choose or skip an attempt number. -/
  | retry (logical : LogicalEffectId) (failure : FailureKind)
  /-- Permanently record the current failed attempt when policy says that no
      further retry is available. -/
  | finalizeFailure (logical : LogicalEffectId) (failure : FailureKind)
  deriving DecidableEq, Repr

inductive EffectReject where
  | controllerDown
  | effectNotFound
  | logicalEffectNotFound
  | logicalEffectAlreadyFinal
  | notDispatchable
  | notAwaitingResult
  | invalidOutcomeEvidence
  | conflictingReceipt
  | notDispatched
  | currentAttemptNotFailed
  | failureMismatch
  | failureNotRetryable
  | retryBudgetExhausted
  | nextAttemptExists
  | retryStillAvailable
  deriving DecidableEq, Repr

def handleEffectDispatch (world : World) (id : EffectId) :
    Except EffectReject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.effects id with
    | none => .error .effectNotFound
    | some .committed =>
        .ok ⟨.dispatchEffect id, dispatchEffect world id,
          Step.dispatchEffect up (Or.inl found)⟩
    | some .outcomeUnknown =>
        .ok ⟨.dispatchEffect id, dispatchEffect world id,
          Step.dispatchEffect up (Or.inr found)⟩
    | some _ => .error .notDispatchable
  else .error .controllerDown

def handleEffectReceipt (world : World) (id : EffectId)
    (outcome : EffectOutcome) : Except EffectReject (Decision world) :=
  match found : world.effects id with
  | none => .error .effectNotFound
  | some .dispatched =>
      if checked : effectOutcomeValidCheck world id.command outcome = true then
        .ok ⟨.acknowledgeEffect id outcome, acknowledgeEffect world id outcome,
          Step.acknowledgeEffect (Or.inl found)
            (effectOutcomeValidCheck_spec.mp checked)⟩
      else .error .invalidOutcomeEvidence
  | some .outcomeUnknown =>
      if checked : effectOutcomeValidCheck world id.command outcome = true then
        .ok ⟨.acknowledgeEffect id outcome, acknowledgeEffect world id outcome,
          Step.acknowledgeEffect (Or.inr found)
            (effectOutcomeValidCheck_spec.mp checked)⟩
      else .error .invalidOutcomeEvidence
  | some (.acknowledged recorded) =>
      if same : recorded = outcome then
        .ok ⟨.acknowledgeEffect id outcome, world,
          Step.replayEffectReceipt (by simpa [same] using found)⟩
      else .error .conflictingReceipt
  | some _ => .error .notAwaitingResult

def handleEffectTimeout (world : World) (id : EffectId) :
    Except EffectReject (Decision world) :=
  match found : world.effects id with
  | none => .error .effectNotFound
  | some .dispatched =>
      .ok ⟨.effectTimeout id, markEffectOutcomeUnknown world id,
        Step.effectTimeout found⟩
  | some .outcomeUnknown =>
      .ok ⟨.effectTimeout id, world, Step.repeatEffectTimeout found⟩
  | some _ => .error .notDispatched

/-- Advance an unfinished logical action by exactly one attempt.  All inputs
    to `Step.startNextAttempt` are reconstructed from durable state: the
    current failed attempt, retry classification, remaining budget, and
    absence of the next exact attempt. -/
def handleEffectRetry (world : World) (logical : LogicalEffectId)
    (failure : FailureKind) : Except EffectReject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.logicalEffects logical with
    | none => .error .logicalEffectNotFound
    | some record =>
        match unfinished : record.result with
        | some _ => .error .logicalEffectAlreadyFinal
        | none =>
            let previous : EffectId := ⟨logical, record.currentAttempt⟩
            match failed : world.effects previous with
            | some (.acknowledged (.failed observed)) =>
                if same : observed = failure then
                  if retryable : failure.retryable = true then
                    match budget : record.remainingRetries with
                    | 0 => .error .retryBudgetExhausted
                    | remaining + 1 =>
                        let next : EffectId :=
                          ⟨logical, record.currentAttempt + 1⟩
                        match fresh : world.effects next with
                        | some _ => .error .nextAttemptExists
                        | none =>
                            .ok ⟨.startNextAttempt previous next,
                              startNextAttempt world logical failure,
                              Step.startNextAttempt failure up found unfinished
                                rfl (by simpa [same] using failed) retryable
                                budget rfl fresh⟩
                  else .error .failureNotRetryable
                else .error .failureMismatch
            | _ => .error .currentAttemptNotFailed
  else .error .controllerDown

/-- Finalize the current failed attempt only if the failure is non-retryable
    or the retry budget is exhausted.  A retryable failure with budget left is
    deliberately rejected, preventing the scheduler from silently skipping a
    permitted attempt. -/
def handleEffectFinalizeFailure (world : World) (logical : LogicalEffectId)
    (failure : FailureKind) : Except EffectReject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.logicalEffects logical with
    | none => .error .logicalEffectNotFound
    | some record =>
        match unfinished : record.result with
        | some _ => .error .logicalEffectAlreadyFinal
        | none =>
            let current : EffectId := ⟨logical, record.currentAttempt⟩
            match failed : world.effects current with
            | some (.acknowledged (.failed observed)) =>
                if same : observed = failure then
                  match classification : failure.retryable with
                  | false =>
                      .ok ⟨.finalizeEffectFailure current failure,
                        finalizeLogicalFailure world logical failure,
                        Step.finalizeEffectFailure failure up found unfinished rfl
                          (by simpa [same] using failed) (Or.inl classification)⟩
                  | true =>
                      match budget : record.remainingRetries with
                      | 0 =>
                          .ok ⟨.finalizeEffectFailure current failure,
                            finalizeLogicalFailure world logical failure,
                            Step.finalizeEffectFailure failure up found unfinished
                              rfl (by simpa [same] using failed)
                              (Or.inr budget)⟩
                      | _ + 1 => .error .retryStillAvailable
                else .error .failureMismatch
            | _ => .error .currentAttemptNotFailed
  else .error .controllerDown

def handleEffect (world : World) : EffectInput →
    Except EffectReject (Decision world)
  | .dispatch id => handleEffectDispatch world id
  | .receipt id outcome => handleEffectReceipt world id outcome
  | .timeout id => handleEffectTimeout world id
  | .retry logical failure => handleEffectRetry world logical failure
  | .finalizeFailure logical failure =>
      handleEffectFinalizeFailure world logical failure

theorem accepted_effect_is_legal
    (_accepted : handleEffect world input = .ok decision) :
    Step world decision.event decision.next :=
  decision.legal

theorem accepted_effect_preserves_safe
    (safe : Safe world)
    (accepted : handleEffect world input = .ok decision) : Safe decision.next :=
  step_preserves_safe safe (accepted_effect_is_legal accepted)

theorem accepted_effect_is_protocol_secure
    (accepted : handleEffect world input = .ok decision) :
    ProtocolStepSecurity world decision.event decision.next :=
  step_protocol_secure (accepted_effect_is_legal accepted)

/-! ## Executable scheduler and stable-state publication -/

/-- The restore source is either absent, or is the sandbox's selected durable
    snapshot and passes the same ownership/manifest checks as the relational
    protocol. -/
def restoreSourceReadyCheck (world : World) (sid : SandboxId)
    (record : SandboxRecord) : Bool :=
  match record.restoreFrom with
  | none => true
  | some source =>
      decide (record.durableSnapshot = some source) &&
        usableSnapshotCheck world sid record source

theorem restoreSourceReadyCheck_spec :
    restoreSourceReadyCheck world sid record = true ↔
      ∀ source, record.restoreFrom = some source →
        record.durableSnapshot = some source ∧
          usableSnapshot world sid record source := by
  unfold restoreSourceReadyCheck
  cases selected : record.restoreFrom with
  | none => simp [selected]
  | some source =>
      simp [selected, usableSnapshotCheck_spec]

def stoppableVmStatusCheck : VmStatus → Bool
  | .prepared | .executing | .frozen => true
  | .stopped | .reclaimed => false

theorem stoppableVmStatusCheck_spec :
    stoppableVmStatusCheck status = true ↔ stoppableVmStatus status := by
  cases status <;> simp [stoppableVmStatusCheck, stoppableVmStatus]

def notCurrentVmCheck (world : World) (sid : SandboxId)
    (vmId : VmId) : Bool :=
  match world.sandboxes sid with
  | none => true
  | some record => decide (record.currentVm ≠ some vmId)

theorem notCurrentVmCheck_spec :
    notCurrentVmCheck world sid vmId = true ↔
      ∀ record, world.sandboxes sid = some record →
        record.currentVm ≠ some vmId := by
  unfold notCurrentVmCheck
  cases found : world.sandboxes sid <;> simp [found]

def notCurrentSlotCheck (world : World) (sid : SandboxId)
    (slotId : NetSlotId) : Bool :=
  match world.sandboxes sid with
  | none => true
  | some record => decide (record.netSlot ≠ some slotId)

theorem notCurrentSlotCheck_spec :
    notCurrentSlotCheck world sid slotId = true ↔
      ∀ record, world.sandboxes sid = some record →
        record.netSlot ≠ some slotId := by
  unfold notCurrentSlotCheck
  cases found : world.sandboxes sid <;> simp [found]

/-- Finite sufficient check for the global `allVmStopsSucceeded` publication
    guard.  Terminal scheduling uses one fence epoch for every indexed VM. -/
def indexedStopsSucceededCheck (world : World) (sid : SandboxId)
    (epoch : Epoch) : Bool :=
  (world.resourceIndexes sid).vms.all fun vmId =>
    match world.vms vmId with
    | none => false
    | some vm => effectSucceededCheck world
        (.stopVm sid epoch vm.generation vmId)

def IndexedStopsSucceededAt (world : World) (sid : SandboxId)
    (epoch : Epoch) : Prop :=
  ∀ vmId, vmId ∈ (world.resourceIndexes sid).vms →
    ∃ vm, world.vms vmId = some vm ∧
      effectSucceeded world (.stopVm sid epoch vm.generation vmId)

theorem indexedStopsSucceededCheck_spec :
    indexedStopsSucceededCheck world sid epoch = true ↔
      IndexedStopsSucceededAt world sid epoch := by
  unfold indexedStopsSucceededCheck IndexedStopsSucceededAt
  induction (world.resourceIndexes sid).vms with
  | nil => simp
  | cons vmId rest ih =>
      cases found : world.vms vmId <;>
        simp [List.all_cons, found, effectSucceededCheck_spec, ih]

theorem indexedStopsSucceededCheck_global
    (valid : ResourceIndexValid world)
    (checked : indexedStopsSucceededCheck world sid epoch = true) :
    allVmStopsSucceeded world sid := by
  have indexed := indexedStopsSucceededCheck_spec.mp checked
  intro vmId vm found owned
  have member := valid.2.1 sid vmId vm found owned
  rcases indexed vmId member with ⟨indexedVm, indexedFound, succeeded⟩
  have same : indexedVm = vm :=
    Option.some.inj (indexedFound.symm.trans found)
  subst indexedVm
  rcases succeeded with ⟨result, acknowledged⟩
  exact ⟨epoch, result, acknowledged⟩

/-- Check one finitely indexed command as a witness for a VM's successful
    stop.  The status lookup still follows the current exact retry head. -/
def stopReceiptCandidateCheck (world : World) (sid : SandboxId)
    (generation : Generation) (vmId : VmId) (id : EffectId) : Bool :=
  match id.command with
  | .stopVm commandSid _ commandGeneration commandVm =>
      decide (commandSid = sid ∧ commandGeneration = generation ∧
        commandVm = vmId) && effectSucceededCheck world id.command
  | _ => false

theorem stopReceiptCandidateCheck_sound (candidate : EffectId)
    (checked : stopReceiptCandidateCheck world sid generation vmId candidate = true) :
    vmStopSucceeded world sid generation vmId := by
  cases command : candidate.command <;>
    simp [stopReceiptCandidateCheck, command, effectSucceededCheck_spec] at checked
  case stopVm commandSid commandEpoch commandGeneration commandVm =>
    rcases checked with ⟨⟨sameSid, sameGeneration, sameVm⟩,
      result, acknowledged⟩
    subst commandSid
    subst commandGeneration
    subst commandVm
    exact ⟨commandEpoch, result, acknowledged⟩

def vmStopSucceededFiniteCheck (world : World) (sid : SandboxId)
    (generation : Generation) (vmId : VmId) : Bool :=
  world.effectKeys.any
    (stopReceiptCandidateCheck world sid generation vmId)

private theorem anyStopReceipt_sound (keys : List EffectId)
    (checked : keys.any
      (stopReceiptCandidateCheck world sid generation vmId) = true) :
    vmStopSucceeded world sid generation vmId := by
  induction keys with
  | nil => simp at checked
  | cons id rest ih =>
      simp only [List.any_cons, Bool.or_eq_true] at checked
      rcases checked with head | tail
      · exact stopReceiptCandidateCheck_sound id head
      · exact ih tail

theorem vmStopSucceededFiniteCheck_sound
    (checked : vmStopSucceededFiniteCheck world sid generation vmId = true) :
    vmStopSucceeded world sid generation vmId := by
  exact anyStopReceipt_sound world.effectKeys checked

def indexedAnyStopsSucceededCheck (world : World) (sid : SandboxId) : Bool :=
  (world.resourceIndexes sid).vms.all fun vmId =>
    match world.vms vmId with
    | none => false
    | some vm => vmStopSucceededFiniteCheck world sid vm.generation vmId

def IndexedAnyStopsSucceeded (world : World) (sid : SandboxId) : Prop :=
  ∀ vmId, vmId ∈ (world.resourceIndexes sid).vms →
    ∃ vm, world.vms vmId = some vm ∧
      vmStopSucceededFiniteCheck world sid vm.generation vmId = true

theorem indexedAnyStopsSucceededCheck_spec :
    indexedAnyStopsSucceededCheck world sid = true ↔
      IndexedAnyStopsSucceeded world sid := by
  unfold indexedAnyStopsSucceededCheck IndexedAnyStopsSucceeded
  induction (world.resourceIndexes sid).vms with
  | nil => simp
  | cons vmId rest ih =>
      cases found : world.vms vmId <;>
        simp [List.all_cons, found, ih]

theorem indexedAnyStopsSucceededCheck_global
    (valid : ResourceIndexValid world)
    (checked : indexedAnyStopsSucceededCheck world sid = true) :
    allVmStopsSucceeded world sid := by
  have indexed := indexedAnyStopsSucceededCheck_spec.mp checked
  intro vmId vm found owned
  have member := valid.2.1 sid vmId vm found owned
  rcases indexed vmId member with ⟨indexedVm, indexedFound, succeeded⟩
  have same : indexedVm = vm :=
    Option.some.inj (indexedFound.symm.trans found)
  subst indexedVm
  exact vmStopSucceededFiniteCheck_sound succeeded

inductive SchedulerInput where
  | reserveStartup (key : OperationKey)
  | reservePauseSnapshot (key : OperationKey)
  | dispatchPrepare (sid : SandboxId) (vm : VmId) (slot : NetSlotId)
  | sealSnapshot (sid : SandboxId) (snapshot : SnapshotId)
      (manifest : SnapshotManifestRef)
  | commitSnapshot (sid : SandboxId) (snapshot : SnapshotId)
  | stopVm (sid : SandboxId) (generation : Generation) (vm : VmId)
  | cleanupVm (sid : SandboxId) (generation : Generation) (vm : VmId)
  | cleanupSlot (sid : SandboxId) (generation : Generation)
      (slot : NetSlotId)
  deriving DecidableEq, Repr

inductive SchedulerReject where
  | controllerDown
  | sandboxNotFound
  | operationUnavailable
  | planUnavailable
  | vmNotFound
  | slotNotFound
  | snapshotNotFound
  | wrongPhase
  | terminal
  | planMismatch
  | planAlreadySealed
  | planNotOpen
  | fenceNotInstalled
  | resourceAlreadyExists
  | restoreSourceUnavailable
  | prerequisiteNotSucceeded
  | resourceMismatch
  | resourceNotPlanned
  | resourceNotReady
  | resourceStillCurrent
  | alreadyReclaimed
  deriving DecidableEq, Repr

def handleReserveStartup (world : World) (key : OperationKey) :
    Except SchedulerReject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes key.sandbox with
    | none => .error .sandboxNotFound
    | some record =>
        if starting : record.phase = .starting then
          if live : record.terminal = false then
            match currentOperation : record.currentOperation with
            | none => .error .operationUnavailable
            | some operation =>
                if operationId : operation.id = key.operation then
                  match foundPlan : world.operationPlans key with
                  | none => .error .planUnavailable
                  | some plan =>
                      if planKey : plan.key = key then
                        if planEpoch : plan.epoch = record.epoch then
                          if planGeneration : plan.generation = record.generation then
                            if planOpen : plan.sealed = false then
                              if planVmOpen : plan.vm = none then
                                if planSlotOpen : plan.slot = none then
                                  if planSource : plan.restoreFrom =
                                      record.restoreFrom then
                                    let vmId : VmId := ⟨world.nextVmId⟩
                                    let slotId : NetSlotId := ⟨world.nextSlotId⟩
                                    match freshVm : world.vms vmId with
                                    | some _ => .error .resourceAlreadyExists
                                    | none =>
                                        match freshSlot : world.netSlots slotId with
                                        | some _ => .error .resourceAlreadyExists
                                        | none =>
                                            have sameKey :
                                                operationKey key.sandbox operation.id =
                                                  key := by
                                              cases key
                                              simp [operationKey, operationId]
                                            .ok ⟨
                                              .reserveStartupResources key vmId slotId,
                                              reserveStartupResources world key plan,
                                              by
                                                simpa [sameKey] using
                                                  (Step.reserveStartupResources
                                                    up found starting live
                                                    currentOperation
                                                    (by simpa [sameKey] using foundPlan)
                                                    (by simpa [sameKey] using planKey)
                                                    planEpoch planGeneration planOpen
                                                    planVmOpen planSlotOpen planSource
                                                    freshVm freshSlot)⟩
                                  else .error .planMismatch
                                else .error .planNotOpen
                              else .error .planNotOpen
                            else .error .planAlreadySealed
                          else .error .planMismatch
                        else .error .planMismatch
                      else .error .planMismatch
                else .error .planMismatch
          else .error .terminal
        else .error .wrongPhase
  else .error .controllerDown

def handleReservePauseSnapshot (world : World) (key : OperationKey) :
    Except SchedulerReject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes key.sandbox with
    | none => .error .sandboxNotFound
    | some record =>
        if pausing : record.phase = .pausing then
          if live : record.terminal = false then
            match currentOperation : record.currentOperation with
            | none => .error .operationUnavailable
            | some operation =>
                if operationId : operation.id = key.operation then
                  match foundPlan : world.operationPlans key with
                  | none => .error .planUnavailable
                  | some plan =>
                      if planKey : plan.key = key then
                        if planEpoch : plan.epoch = record.epoch then
                          if planGeneration : plan.generation = record.generation then
                            if planOpen : plan.sealed = false then
                              if planVmOpen : plan.vm = none then
                                if targetOpen : plan.targetSnapshot = none then
                                  match currentVm : record.currentVm with
                                  | none => .error .vmNotFound
                                  | some vmId =>
                                      let snapshotId : SnapshotId :=
                                        ⟨world.nextSnapshotId⟩
                                      let manifest : SnapshotManifestRef :=
                                        ⟨⟨world.nextManifestDigest⟩⟩
                                      match fresh : world.snapshots snapshotId with
                                      | some _ => .error .resourceAlreadyExists
                                      | none =>
                                          have sameKey :
                                              operationKey key.sandbox operation.id =
                                                key := by
                                            cases key
                                            simp [operationKey, operationId]
                                          .ok ⟨
                                            .reservePauseSnapshot key snapshotId manifest,
                                            reservePauseSnapshot world key plan vmId,
                                            by
                                              simpa [sameKey] using
                                                (Step.reservePauseSnapshot up found
                                                  pausing live currentOperation
                                                  (by simpa [sameKey] using foundPlan)
                                                  (by simpa [sameKey] using planKey)
                                                  planEpoch planGeneration planOpen
                                                  planVmOpen targetOpen currentVm fresh)⟩
                                else .error .planNotOpen
                              else .error .planNotOpen
                            else .error .planAlreadySealed
                          else .error .planMismatch
                        else .error .planMismatch
                      else .error .planMismatch
                else .error .planMismatch
          else .error .terminal
        else .error .wrongPhase
  else .error .controllerDown

def handleDispatchPrepare (world : World) (sid : SandboxId)
    (vmId : VmId) (slotId : NetSlotId) :
    Except SchedulerReject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if starting : record.phase = .starting then
          if live : record.terminal = false then
            match currentOperation : record.currentOperation with
            | none => .error .operationUnavailable
            | some operation =>
                match foundPlan : world.operationPlans
                    (operationKey sid operation.id) with
                | none => .error .planUnavailable
                | some plan =>
                    if planKey : plan.key = operationKey sid operation.id then
                      if planEpoch : plan.epoch = record.epoch then
                        if planGeneration : plan.generation = record.generation then
                          if planSealed : plan.sealed = true then
                            if plannedVm : plan.vm = some vmId then
                              if plannedSlot : plan.slot = some slotId then
                                if planSource : plan.restoreFrom =
                                    record.restoreFrom then
                                  if fenced : world.workerFences sid =
                                      some ⟨record.epoch, false⟩ then
                                    match freshVm : world.vms vmId with
                                    | some _ => .error .resourceAlreadyExists
                                    | none =>
                                        match freshSlot : world.netSlots slotId with
                                        | some _ => .error .resourceAlreadyExists
                                        | none =>
                                            if sourceReady :
                                                restoreSourceReadyCheck world sid
                                                  record = true then
                                              .ok ⟨
                                                .dispatchPrepare sid vmId slotId
                                                  record.restoreFrom,
                                                enqueue world
                                                  (.prepareVm sid record.epoch
                                                    record.generation vmId slotId
                                                    record.restoreFrom),
                                                Step.dispatchPrepare up found starting
                                                  live currentOperation foundPlan
                                                  planKey planEpoch planGeneration
                                                  planSealed plannedVm plannedSlot
                                                  planSource fenced freshVm freshSlot
                                                  rfl
                                                  (restoreSourceReadyCheck_spec.mp
                                                    sourceReady)⟩
                                            else .error .restoreSourceUnavailable
                                  else .error .fenceNotInstalled
                                else .error .planMismatch
                              else .error .planMismatch
                            else .error .planMismatch
                          else .error .planNotOpen
                        else .error .planMismatch
                      else .error .planMismatch
                    else .error .planMismatch
          else .error .terminal
        else .error .wrongPhase
  else .error .controllerDown

def handleScheduleSealSnapshot (world : World) (sid : SandboxId)
    (snapshotId : SnapshotId) (manifest : SnapshotManifestRef) :
    Except SchedulerReject (Decision world) :=
  if up : world.controllerUp = true then
    match foundSandbox : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if pausing : record.phase = .pausing then
          match currentOperation : record.currentOperation with
          | none => .error .operationUnavailable
          | some operation =>
              match foundPlan : world.operationPlans
                  (operationKey sid operation.id) with
              | none => .error .planUnavailable
              | some plan =>
                  if planKey : plan.key = operationKey sid operation.id then
                    if planEpoch : plan.epoch = record.epoch then
                      if planGeneration : plan.generation = record.generation then
                        if planSealed : plan.sealed = true then
                          if plannedSnapshot : plan.targetSnapshot =
                              some ⟨snapshotId, manifest⟩ then
                            match current : record.currentVm with
                            | none => .error .vmNotFound
                            | some vmId =>
                                if plannedVm : plan.vm = some vmId then
                                  match foundVm : world.vms vmId with
                                  | none => .error .vmNotFound
                                  | some vm =>
                                      if vmIdentity : vm.owner = record.owner ∧
                                          vm.sandbox = sid ∧
                                          vm.generation = record.generation then
                                        if frozen : vm.status = .frozen then
                                          if fenced : world.workerFences sid =
                                              some ⟨record.epoch, false⟩ then
                                            if freezeSucceeded :
                                                effectSucceededCheck world
                                                  (.freezeVm sid record.epoch
                                                    record.generation vmId) = true then
                                              match fresh : world.snapshots snapshotId with
                                              | some _ => .error .resourceAlreadyExists
                                              | none =>
                                                  .ok ⟨
                                                    .scheduleSealSnapshot sid snapshotId
                                                      manifest,
                                                    enqueue world
                                                      (.sealSnapshot sid record.epoch
                                                        record.generation vmId
                                                        record.workspace snapshotId
                                                        manifest),
                                                    Step.scheduleSealSnapshot up
                                                      foundSandbox pausing
                                                      currentOperation foundPlan
                                                      planKey planEpoch planGeneration
                                                      planSealed plannedVm
                                                      plannedSnapshot foundVm current
                                                      vmIdentity frozen fenced
                                                      (effectSucceededCheck_spec.mp
                                                        freezeSucceeded)
                                                      fresh⟩
                                            else .error .prerequisiteNotSucceeded
                                          else .error .fenceNotInstalled
                                        else .error .resourceNotReady
                                      else .error .resourceMismatch
                                else .error .planMismatch
                          else .error .planMismatch
                        else .error .planNotOpen
                      else .error .planMismatch
                    else .error .planMismatch
                  else .error .planMismatch
        else .error .wrongPhase
  else .error .controllerDown

def handleScheduleCommitSnapshot (world : World) (sid : SandboxId)
    (snapshotId : SnapshotId) : Except SchedulerReject (Decision world) :=
  if up : world.controllerUp = true then
    match foundSnapshot : world.snapshots snapshotId with
    | none => .error .snapshotNotFound
    | some snapshot =>
        match foundSandbox : world.sandboxes sid with
        | none => .error .sandboxNotFound
        | some record =>
            if snapshotSandbox : snapshot.sandbox = sid then
              if pausing : record.phase = .pausing then
                match current : record.currentVm with
                | none => .error .vmNotFound
                | some vmId =>
                    if snapshotIdentity :
                        snapshot.sourceGeneration = record.generation ∧
                          snapshot.workspace = record.workspace then
                      if sealed : snapshot.stage = .localSealed then
                        if sealSucceeded : effectSucceededCheck world
                            (.sealSnapshot sid record.epoch record.generation
                              vmId record.workspace snapshotId
                              snapshot.manifest) = true then
                          .ok ⟨.scheduleCommitSnapshot sid snapshotId,
                            enqueue world (.commitSnapshot sid record.epoch
                              record.generation snapshotId snapshot.manifest),
                            Step.scheduleCommitSnapshot up foundSnapshot
                              foundSandbox snapshotSandbox pausing current
                              snapshotIdentity sealed
                              (effectSucceededCheck_spec.mp sealSucceeded)⟩
                        else .error .prerequisiteNotSucceeded
                      else .error .resourceNotReady
                    else .error .resourceMismatch
              else .error .wrongPhase
            else .error .resourceMismatch
  else .error .controllerDown

def handleScheduleStopVm (world : World) (sid : SandboxId)
    (generation : Generation) (vmId : VmId) :
    Except SchedulerReject (Decision world) :=
  if up : world.controllerUp = true then
    match foundSandbox : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if revoking : record.phase = .revoking then
          if terminal : record.terminal = true then
            match currentOperation : record.currentOperation with
            | none => .error .operationUnavailable
            | some operation =>
                match foundPlan : world.operationPlans
                    (operationKey sid operation.id) with
                | none => .error .planUnavailable
                | some plan =>
                    if planKey : plan.key = operationKey sid operation.id then
                      if planEpoch : plan.epoch = record.epoch then
                        if planGeneration : plan.generation = record.generation then
                          if planned : vmId ∈ plan.stopVms then
                            if fenced : world.workerFences sid =
                                some ⟨record.epoch, true⟩ then
                              if fenceSucceeded : effectSucceededCheck world
                                  (.installFence sid record.epoch true) = true then
                                match foundVm : world.vms vmId with
                                | none => .error .vmNotFound
                                | some vm =>
                                    if identity : vm.sandbox = sid ∧
                                        vm.generation = generation then
                                      if stoppable :
                                          stoppableVmStatusCheck vm.status = true then
                                        .ok ⟨.scheduleStopVm sid generation vmId,
                                          enqueue world
                                            (.stopVm sid record.epoch generation vmId),
                                          Step.scheduleStopVm up foundSandbox revoking
                                            terminal currentOperation foundPlan planKey
                                            planEpoch planGeneration planned fenced
                                            (effectSucceededCheck_spec.mp fenceSucceeded)
                                            foundVm identity
                                            (stoppableVmStatusCheck_spec.mp stoppable)⟩
                                      else .error .resourceNotReady
                                    else .error .resourceMismatch
                              else .error .prerequisiteNotSucceeded
                            else .error .fenceNotInstalled
                          else .error .resourceNotPlanned
                        else .error .planMismatch
                      else .error .planMismatch
                    else .error .planMismatch
          else .error .terminal
        else .error .wrongPhase
  else .error .controllerDown

def handleScheduleCleanupVm (world : World) (sid : SandboxId)
    (generation : Generation) (vmId : VmId) :
    Except SchedulerReject (Decision world) :=
  if up : world.controllerUp = true then
    match foundSandbox : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        match currentOperation : record.currentOperation with
        | none => .error .operationUnavailable
        | some operation =>
            match foundPlan : world.operationPlans
                (operationKey sid operation.id) with
            | none => .error .planUnavailable
            | some plan =>
                if planKey : plan.key = operationKey sid operation.id then
                  if planEpoch : plan.epoch = record.epoch then
                    if planGeneration : plan.generation = record.generation then
                      if planned : vmId ∈ plan.stopVms then
                        match foundVm : world.vms vmId with
                        | none => .error .vmNotFound
                        | some vm =>
                            if identity : vm.sandbox = sid ∧
                                vm.generation = generation then
                              if stopped : vm.status = .stopped then
                                if notCurrent :
                                    notCurrentVmCheck world sid vmId = true then
                                  .ok ⟨.scheduleCleanupVm sid generation vmId,
                                    enqueue world (.cleanupVm sid generation vmId),
                                    Step.scheduleCleanupVm up foundSandbox
                                      currentOperation foundPlan planKey planEpoch
                                      planGeneration planned foundVm identity stopped
                                      (notCurrentVmCheck_spec.mp notCurrent)⟩
                                else .error .resourceStillCurrent
                              else .error .resourceNotReady
                            else .error .resourceMismatch
                      else .error .resourceNotPlanned
                    else .error .planMismatch
                  else .error .planMismatch
                else .error .planMismatch
  else .error .controllerDown

def handleScheduleCleanupSlot (world : World) (sid : SandboxId)
    (generation : Generation) (slotId : NetSlotId) :
    Except SchedulerReject (Decision world) :=
  if up : world.controllerUp = true then
    match foundSandbox : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        match currentOperation : record.currentOperation with
        | none => .error .operationUnavailable
        | some operation =>
            match foundPlan : world.operationPlans
                (operationKey sid operation.id) with
            | none => .error .planUnavailable
            | some plan =>
                if planKey : plan.key = operationKey sid operation.id then
                  if planEpoch : plan.epoch = record.epoch then
                    if planGeneration : plan.generation = record.generation then
                      if planned : slotId ∈ plan.closeSlots then
                        match foundSlot : world.netSlots slotId with
                        | none => .error .slotNotFound
                        | some slot =>
                            if identity : slot.sandbox = sid ∧
                                slot.generation = generation then
                              if inactive : slot.active = false then
                                if notReclaimed : slot.reclaimed = false then
                                  if notCurrent :
                                      notCurrentSlotCheck world sid slotId = true then
                                    .ok ⟨.scheduleCleanupSlot sid generation slotId,
                                      enqueue world (.cleanupSlot sid generation slotId),
                                      Step.scheduleCleanupSlot up foundSandbox
                                        currentOperation foundPlan planKey planEpoch
                                        planGeneration planned foundSlot identity inactive
                                        notReclaimed
                                        (notCurrentSlotCheck_spec.mp notCurrent)⟩
                                  else .error .resourceStillCurrent
                                else .error .alreadyReclaimed
                              else .error .resourceNotReady
                            else .error .resourceMismatch
                      else .error .resourceNotPlanned
                    else .error .planMismatch
                  else .error .planMismatch
                else .error .planMismatch
  else .error .controllerDown

def handleScheduler (world : World) : SchedulerInput →
    Except SchedulerReject (Decision world)
  | .reserveStartup key => handleReserveStartup world key
  | .reservePauseSnapshot key => handleReservePauseSnapshot world key
  | .dispatchPrepare sid vmId slotId =>
      handleDispatchPrepare world sid vmId slotId
  | .sealSnapshot sid snapshotId manifest =>
      handleScheduleSealSnapshot world sid snapshotId manifest
  | .commitSnapshot sid snapshotId =>
      handleScheduleCommitSnapshot world sid snapshotId
  | .stopVm sid generation vmId =>
      handleScheduleStopVm world sid generation vmId
  | .cleanupVm sid generation vmId =>
      handleScheduleCleanupVm world sid generation vmId
  | .cleanupSlot sid generation slotId =>
      handleScheduleCleanupSlot world sid generation slotId

theorem accepted_scheduler_is_legal
    (_accepted : handleScheduler world input = .ok decision) :
    Step world decision.event decision.next :=
  decision.legal

theorem accepted_scheduler_preserves_safe
    (safe : Safe world)
    (accepted : handleScheduler world input = .ok decision) :
    Safe decision.next :=
  step_preserves_safe safe (accepted_scheduler_is_legal accepted)

theorem accepted_scheduler_is_protocol_secure
    (accepted : handleScheduler world input = .ok decision) :
    ProtocolStepSecurity world decision.event decision.next :=
  step_protocol_secure (accepted_scheduler_is_legal accepted)

theorem accepted_scheduler_preserves_resourceIndexValid
    (valid : ResourceIndexValid world)
    (accepted : handleScheduler world input = .ok decision) :
    ResourceIndexValid decision.next :=
  step_preserves_resourceIndexValid valid
    (accepted_scheduler_is_legal accepted)

def appliedPolicyCheck (world : World) (sid : SandboxId)
    (generation : Generation) (workspace : FsId)
    (policy : FilePolicyRef) : Bool :=
  match world.appliedFilePolicies sid with
  | none => false
  | some applied => decide
      (applied.sandbox = sid ∧ applied.generation = generation ∧
        applied.workspace = workspace ∧ applied.policy = policy)

theorem appliedPolicyCheck_spec :
    appliedPolicyCheck world sid generation workspace policy = true ↔
      world.appliedFilePolicies sid = some
        ⟨sid, generation, workspace, policy⟩ := by
  unfold appliedPolicyCheck
  cases found : world.appliedFilePolicies sid with
  | none => simp
  | some applied =>
      cases applied
      simp_all

inductive PublicationInput where
  | running (sid : SandboxId)
  | paused (sid : SandboxId) (source : RestoreSource)
  | destroyed (sid : SandboxId)
  | failed (sid : SandboxId) (failure : FailureKind)
  deriving DecidableEq, Repr

inductive PublicationReject where
  | controllerDown
  | sandboxNotFound
  | vmNotFound
  | slotNotFound
  | snapshotNotFound
  | operationUnavailable
  | wrongPhase
  | terminalMismatch
  | fenceNotInstalled
  | resourceMismatch
  | resourceNotReady
  | policyNotApplied
  | prerequisiteNotSucceeded
  | resourcesNotClosed
  | stopReceiptsMissing
  | wrongOperationKind
  | wrongOperationStatus
  | ledgerMismatch
  deriving DecidableEq, Repr

def handlePublishRunning (world : World) (valid : ResourceIndexValid world)
    (sid : SandboxId) : Except PublicationReject (Decision world) :=
  if up : world.controllerUp = true then
    match foundSandbox : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if starting : record.phase = .starting then
          if live : record.terminal = false then
            match currentVm : record.currentVm with
            | none => .error .vmNotFound
            | some vmId =>
                match foundVm : world.vms vmId with
                | none => .error .vmNotFound
                | some vm =>
                    if vmIdentity : vm.owner = record.owner ∧
                        vm.sandbox = sid ∧
                        vm.generation = record.generation then
                      if executing : vm.status = .executing then
                        match currentSlot : record.netSlot with
                        | none => .error .slotNotFound
                        | some slotId =>
                            match foundSlot : world.netSlots slotId with
                            | none => .error .slotNotFound
                            | some slot =>
                                if slotReady : slot.owner = record.owner ∧
                                    slot.sandbox = sid ∧
                                    slot.generation = record.generation ∧
                                    slot.vm = vmId ∧
                                    slot.policy = record.policy ∧
                                    slot.configured = true ∧
                                    slot.active = false then
                                  if networkClosed : indexedSlotsClosedCheck
                                      world sid = true then
                                    if appliedPolicy : appliedPolicyCheck world sid
                                        record.generation record.workspace
                                        record.filePolicy = true then
                                      if fenceSucceeded : effectSucceededCheck world
                                          (.installFence sid record.epoch false) = true then
                                        if prepareSucceeded : effectSucceededCheck world
                                            (.prepareVm sid record.epoch
                                              record.generation vmId slotId
                                              record.restoreFrom) = true then
                                          if networkSucceeded : effectSucceededCheck world
                                              (.configureNetwork sid record.epoch
                                                record.generation vmId slotId
                                                record.policy) = true then
                                            if activationSucceeded :
                                                effectSucceededCheck world
                                                  (.activateVm sid record.epoch
                                                    record.generation vmId) = true then
                                              if policyAcknowledged :
                                                  effectSucceededCheck world
                                                    (.applyFilePolicy sid record.epoch
                                                      record.generation record.workspace
                                                      record.filePolicy) = true then
                                                match currentOperation :
                                                    record.currentOperation with
                                                | none => .error .operationUnavailable
                                                | some operation =>
                                                    match persistedOperation :
                                                        world.operations
                                                          (operationKey sid
                                                            operation.id) with
                                                    | none => .error .operationUnavailable
                                                    | some ledger =>
                                                        if operationMatches :
                                                            ledger.sandbox = sid ∧
                                                            ledger.operation = operation ∧
                                                            ledger.result = none then
                                                          if fenced : world.workerFences
                                                              sid = some
                                                                ⟨record.epoch, false⟩ then
                                                            .ok ⟨.publishRunning sid,
                                                              publishRunning world sid
                                                                record slotId slot,
                                                              Step.publishRunning up
                                                                foundSandbox starting live
                                                                currentVm foundVm vmIdentity
                                                                executing foundSlot currentSlot
                                                                slotReady
                                                                ((indexedSlotsClosedCheck_global_spec
                                                                  (world := world) (sid := sid)
                                                                  valid).mp networkClosed)
                                                                (appliedPolicyCheck_spec.mp
                                                                  appliedPolicy)
                                                                (effectSucceededCheck_spec.mp
                                                                  fenceSucceeded)
                                                                (effectSucceededCheck_spec.mp
                                                                  prepareSucceeded)
                                                                (effectSucceededCheck_spec.mp
                                                                  networkSucceeded)
                                                                (effectSucceededCheck_spec.mp
                                                                  activationSucceeded)
                                                                (effectSucceededCheck_spec.mp
                                                                  policyAcknowledged)
                                                                currentOperation
                                                                persistedOperation
                                                                operationMatches fenced⟩
                                                          else .error .fenceNotInstalled
                                                        else .error .ledgerMismatch
                                              else .error .prerequisiteNotSucceeded
                                            else .error .prerequisiteNotSucceeded
                                          else .error .prerequisiteNotSucceeded
                                        else .error .prerequisiteNotSucceeded
                                      else .error .prerequisiteNotSucceeded
                                    else .error .policyNotApplied
                                  else .error .resourcesNotClosed
                                else .error .resourceNotReady
                      else .error .resourceNotReady
                    else .error .resourceMismatch
          else .error .terminalMismatch
        else .error .wrongPhase
  else .error .controllerDown

def handlePublishPaused (world : World) (valid : ResourceIndexValid world)
    (sid : SandboxId) (source : RestoreSource) :
    Except PublicationReject (Decision world) :=
  if up : world.controllerUp = true then
    match foundSandbox : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if pausing : record.phase = .pausing then
          match foundSnapshot : world.snapshots source.snapshot with
          | none => .error .snapshotNotFound
          | some snapshot =>
              if snapshotReady : snapshot.owner = record.owner ∧
                  snapshot.sandbox = sid ∧
                  snapshot.sourceGeneration = record.generation ∧
                  snapshot.workspace = record.workspace ∧
                  snapshot.manifest = source.manifest ∧
                  snapshot.stage = .durable ∧ snapshot.immutable = true then
                if fenced : world.workerFences sid =
                    some ⟨record.epoch, false⟩ then
                  match currentVm : record.currentVm with
                  | none => .error .vmNotFound
                  | some vmId =>
                      if fenceSucceeded : effectSucceededCheck world
                          (.installFence sid record.epoch false) = true then
                        if freezeSucceeded : effectSucceededCheck world
                            (.freezeVm sid record.epoch record.generation
                              vmId) = true then
                          if sealSucceeded : effectSucceededCheck world
                              (.sealSnapshot sid record.epoch record.generation
                                vmId record.workspace source.snapshot
                                source.manifest) = true then
                            if commitSucceeded : effectSucceededCheck world
                                (.commitSnapshot sid record.epoch
                                  record.generation source.snapshot
                                  source.manifest) = true then
                              if stopSucceeded : effectSucceededCheck world
                                  (.stopVm sid record.epoch record.generation
                                    vmId) = true then
                                if vmsClosed : indexedVmsClosedCheck world sid =
                                    true then
                                  if networkClosed : indexedSlotsClosedCheck world
                                      sid = true then
                                    match currentOperation :
                                        record.currentOperation with
                                    | none => .error .operationUnavailable
                                    | some operation =>
                                        match persistedOperation :
                                            world.operations
                                              (operationKey sid operation.id) with
                                        | none => .error .operationUnavailable
                                        | some ledger =>
                                            if operationMatches :
                                                ledger.sandbox = sid ∧
                                                ledger.operation = operation ∧
                                                ledger.result = none then
                                              .ok ⟨.publishPaused sid
                                                  source.snapshot,
                                                publishPaused world sid record source,
                                                Step.publishPaused up foundSandbox
                                                  pausing foundSnapshot snapshotReady
                                                  fenced currentVm
                                                  (effectSucceededCheck_spec.mp
                                                    fenceSucceeded)
                                                  (effectSucceededCheck_spec.mp
                                                    freezeSucceeded)
                                                  (effectSucceededCheck_spec.mp
                                                    sealSucceeded)
                                                  (effectSucceededCheck_spec.mp
                                                    commitSucceeded)
                                                  (effectSucceededCheck_spec.mp
                                                    stopSucceeded)
                                                  ((indexedVmsClosedCheck_global_spec
                                                    (world := world) (sid := sid)
                                                    valid).mp vmsClosed)
                                                  ((indexedSlotsClosedCheck_global_spec
                                                    (world := world) (sid := sid)
                                                    valid).mp networkClosed)
                                                  currentOperation
                                                  persistedOperation
                                                  operationMatches⟩
                                            else .error .ledgerMismatch
                                  else .error .resourcesNotClosed
                                else .error .resourcesNotClosed
                              else .error .prerequisiteNotSucceeded
                            else .error .prerequisiteNotSucceeded
                          else .error .prerequisiteNotSucceeded
                        else .error .prerequisiteNotSucceeded
                      else .error .prerequisiteNotSucceeded
                else .error .fenceNotInstalled
              else .error .resourceNotReady
        else .error .wrongPhase
  else .error .controllerDown

def handlePublishDestroyed (world : World) (valid : ResourceIndexValid world)
    (sid : SandboxId) : Except PublicationReject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if revoking : record.phase = .revoking then
          if terminal : record.terminal = true then
            if fenced : world.workerFences sid =
                some ⟨record.epoch, true⟩ then
              if fenceSucceeded : effectSucceededCheck world
                  (.installFence sid record.epoch true) = true then
                if vmsClosed : indexedVmsClosedCheck world sid = true then
                  if stopsSucceeded : indexedAnyStopsSucceededCheck world sid =
                      true then
                    if networkClosed : indexedSlotsClosedCheck world sid = true then
                      match currentOperation : record.currentOperation with
                      | none => .error .operationUnavailable
                      | some operation =>
                          if killOperation : operation.kind = .kill then
                            if unfinished : operation.status = .pending ∨
                                operation.status = .outcomeUnknown then
                              match persistedOperation : world.operations
                                  (operationKey sid operation.id) with
                              | none => .error .operationUnavailable
                              | some ledger =>
                                  if operationMatches : ledger.sandbox = sid ∧
                                      ledger.operation = operation ∧
                                      ledger.result = none then
                                    .ok ⟨.publishDestroyed sid,
                                      publishDestroyed world sid record,
                                      Step.publishDestroyed up found revoking
                                        terminal fenced
                                        (effectSucceededCheck_spec.mp
                                          fenceSucceeded)
                                        ((indexedVmsClosedCheck_global_spec
                                          (world := world) (sid := sid)
                                          valid).mp vmsClosed)
                                        (indexedAnyStopsSucceededCheck_global valid
                                          stopsSucceeded)
                                        ((indexedSlotsClosedCheck_global_spec
                                          (world := world) (sid := sid)
                                          valid).mp networkClosed)
                                        currentOperation killOperation unfinished
                                        persistedOperation operationMatches⟩
                                  else .error .ledgerMismatch
                            else .error .wrongOperationStatus
                          else .error .wrongOperationKind
                    else .error .resourcesNotClosed
                  else .error .stopReceiptsMissing
                else .error .resourcesNotClosed
              else .error .prerequisiteNotSucceeded
            else .error .fenceNotInstalled
          else .error .terminalMismatch
        else .error .wrongPhase
  else .error .controllerDown

def handlePublishFailure (world : World) (valid : ResourceIndexValid world)
    (sid : SandboxId) (failure : FailureKind) :
    Except PublicationReject (Decision world) :=
  if up : world.controllerUp = true then
    match found : world.sandboxes sid with
    | none => .error .sandboxNotFound
    | some record =>
        if revoking : record.phase = .revoking then
          if terminal : record.terminal = true then
            if fenced : world.workerFences sid =
                some ⟨record.epoch, true⟩ then
              if fenceSucceeded : effectSucceededCheck world
                  (.installFence sid record.epoch true) = true then
                if vmsClosed : indexedVmsClosedCheck world sid = true then
                  if stopsSucceeded : indexedAnyStopsSucceededCheck world sid =
                      true then
                    if networkClosed : indexedSlotsClosedCheck world sid = true then
                      match currentOperation : record.currentOperation with
                      | none => .error .operationUnavailable
                      | some operation =>
                          if compensating : operation.status =
                              .compensating failure then
                            if recoverableKind : operation.kind = .create ∨
                                operation.kind = .resume ∨
                                operation.kind = .pause then
                              match persistedOperation : world.operations
                                  (operationKey sid operation.id) with
                              | none => .error .operationUnavailable
                              | some ledger =>
                                  if operationMatches : ledger.sandbox = sid ∧
                                      ledger.operation = operation ∧
                                      ledger.result = none then
                                    .ok ⟨.publishFailure sid failure,
                                      SandboxModel.publishFailure world sid
                                        record failure,
                                      Step.publishFailure failure up found revoking
                                        terminal fenced
                                        (effectSucceededCheck_spec.mp
                                          fenceSucceeded)
                                        ((indexedVmsClosedCheck_global_spec
                                          (world := world) (sid := sid)
                                          valid).mp vmsClosed)
                                        (indexedAnyStopsSucceededCheck_global valid
                                          stopsSucceeded)
                                        ((indexedSlotsClosedCheck_global_spec
                                          (world := world) (sid := sid)
                                          valid).mp networkClosed)
                                        currentOperation compensating
                                        recoverableKind persistedOperation
                                        operationMatches⟩
                                  else .error .ledgerMismatch
                            else .error .wrongOperationKind
                          else .error .wrongOperationStatus
                    else .error .resourcesNotClosed
                  else .error .stopReceiptsMissing
                else .error .resourcesNotClosed
              else .error .prerequisiteNotSucceeded
            else .error .fenceNotInstalled
          else .error .terminalMismatch
        else .error .wrongPhase
  else .error .controllerDown

def handlePublication (world : World) (valid : ResourceIndexValid world) :
    PublicationInput → Except PublicationReject (Decision world)
  | .running sid => handlePublishRunning world valid sid
  | .paused sid source => handlePublishPaused world valid sid source
  | .destroyed sid => handlePublishDestroyed world valid sid
  | .failed sid failure => handlePublishFailure world valid sid failure

theorem accepted_publication_is_legal
    (_accepted : handlePublication world valid input = .ok decision) :
    Step world decision.event decision.next :=
  decision.legal

theorem accepted_publication_preserves_safe
    (safe : Safe world)
    (accepted : handlePublication world valid input = .ok decision) :
    Safe decision.next :=
  step_preserves_safe safe (accepted_publication_is_legal accepted)

theorem accepted_publication_is_protocol_secure
    (accepted : handlePublication world valid input = .ok decision) :
    ProtocolStepSecurity world decision.event decision.next :=
  step_protocol_secure (accepted_publication_is_legal accepted)

theorem accepted_publication_preserves_resourceIndexValid
    (accepted : handlePublication world valid input = .ok decision) :
    ResourceIndexValid decision.next :=
  step_preserves_resourceIndexValid valid
    (accepted_publication_is_legal accepted)

end SandboxModel.Controller
