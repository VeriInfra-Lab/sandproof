import SandboxModel.Controller

/-!
Executable reducers for the remaining controller-owned protocol actions.

Lifecycle requests, effects, scheduling, publication, and worker commands live
in the dedicated controller modules.  This file closes the smaller executable
surface: RPC outcome uncertainty and capability admission.  Resource tokens
are explicit inputs even where the audit event intentionally omits them; this
lets the reducer construct exactly the capability witness required by `Step`.
-/

namespace SandboxModel.Controller

private instance authorityAllowsDecidable (authority : Authority)
    (capability : Capability) : Decidable (authority.allows capability) := by
  cases authority <;> simp [Authority.allows] <;> infer_instance

inductive AuxiliaryReject where
  | sandboxNotFound
  | operationUnavailable
  | operationAlreadyFinal
  | ledgerMismatch
  | unauthorized
  | wrongPhase
  | wrongGeneration
  | capabilityDenied
  | destinationDenied
  deriving DecidableEq, Repr

def handleRpcTimeout (world : World) (sid : SandboxId) :
    Except AuxiliaryReject (Decision world) :=
  match foundSandbox : world.sandboxes sid with
  | none => .error .sandboxNotFound
  | some record =>
      match currentOperation : record.currentOperation with
      | none => .error .operationUnavailable
      | some operation =>
          if pending : operation.status = .pending then
            match persistedOperation :
                world.operations (operationKey sid operation.id) with
            | none => .error .operationUnavailable
            | some ledger =>
                if operationMatches : ledger.sandbox = sid ∧
                    ledger.operation = operation ∧ ledger.result = none then
                  .ok ⟨.rpcTimeout sid,
                    markOutcomeUnknown world sid record operation,
                    Step.rpcTimeout foundSandbox currentOperation pending
                      persistedOperation operationMatches⟩
                else .error .ledgerMismatch
          else .error .operationAlreadyFinal

inductive AccessInput where
  | exec (actor : Actor) (sid : SandboxId) (vm : VmId)
  | fileRead (actor : Actor) (sid : SandboxId) (generation : Generation)
  | fileWrite (actor : Actor) (sid : SandboxId) (generation : Generation)
  | ingress (actor : Actor) (sid : SandboxId) (vm : VmId) (slot : NetSlotId)
  | egress (sid : SandboxId) (vm : VmId) (slot : NetSlotId)
      (destination : Endpoint)
  deriving DecidableEq, Repr

def handleExec (world : World) (actor : Actor) (sid : SandboxId)
    (vm : VmId) : Except AuxiliaryReject (Decision world) :=
  match found : world.sandboxes sid with
  | none => .error .sandboxNotFound
  | some record =>
      if auth : authorized actor record.owner then
        if capability : record.effective.allows
            (.exec sid record.generation vm) then
          .ok ⟨.exec actor sid, world, Step.exec found auth capability⟩
        else .error .capabilityDenied
      else .error .unauthorized

def handleFileRead (world : World) (actor : Actor) (sid : SandboxId)
    (generation : Generation) : Except AuxiliaryReject (Decision world) :=
  match found : world.sandboxes sid with
  | none => .error .sandboxNotFound
  | some record =>
      if running : record.phase = .running then
        if currentGeneration : generation = record.generation then
          if auth : authorized actor record.owner then
            if capability : record.effective.allows
                (.fileRead sid generation record.workspace
                  record.workspaceAccess) then
              .ok ⟨.fileRead actor sid generation, world,
                Step.fileRead found running currentGeneration auth capability⟩
            else .error .capabilityDenied
          else .error .unauthorized
        else .error .wrongGeneration
      else .error .wrongPhase

def handleFileWrite (world : World) (actor : Actor) (sid : SandboxId)
    (generation : Generation) : Except AuxiliaryReject (Decision world) :=
  match found : world.sandboxes sid with
  | none => .error .sandboxNotFound
  | some record =>
      if running : record.phase = .running then
        if currentGeneration : generation = record.generation then
          if auth : authorized actor record.owner then
            if capability : record.effective.allows
                (.fileWrite sid generation record.workspace
                  record.workspaceAccess) then
              .ok ⟨.fileWrite actor sid generation, world,
                Step.fileWrite found running currentGeneration auth capability⟩
            else .error .capabilityDenied
          else .error .unauthorized
        else .error .wrongGeneration
      else .error .wrongPhase

def handleIngress (world : World) (actor : Actor) (sid : SandboxId)
    (vm : VmId) (slot : NetSlotId) :
    Except AuxiliaryReject (Decision world) :=
  match found : world.sandboxes sid with
  | none => .error .sandboxNotFound
  | some record =>
      if auth : authorized actor record.owner then
        if capability : record.effective.allows
            (.ingress sid record.generation vm slot) then
          .ok ⟨.ingress actor sid, world, Step.ingress found auth capability⟩
        else .error .capabilityDenied
      else .error .unauthorized

def handleEgress (world : World) (sid : SandboxId) (vm : VmId)
    (slot : NetSlotId) (destination : Endpoint) :
    Except AuxiliaryReject (Decision world) :=
  match found : world.sandboxes sid with
  | none => .error .sandboxNotFound
  | some record =>
      if capability : record.effective.allows
          (.egress sid record.generation vm slot record.policy) then
        match policyCase : record.policy with
        | .denyAll => .error .destinationDenied
        | .staticAllowlist endpoints =>
            if member : destination ∈ endpoints then
              .ok ⟨.egress sid destination, world,
                Step.egress found capability ⟨policyCase, member⟩⟩
            else .error .destinationDenied
      else .error .capabilityDenied

def handleAccess (world : World) : AccessInput →
    Except AuxiliaryReject (Decision world)
  | .exec actor sid vm => handleExec world actor sid vm
  | .fileRead actor sid generation =>
      handleFileRead world actor sid generation
  | .fileWrite actor sid generation =>
      handleFileWrite world actor sid generation
  | .ingress actor sid vm slot => handleIngress world actor sid vm slot
  | .egress sid vm slot destination =>
      handleEgress world sid vm slot destination

theorem accepted_rpcTimeout_is_legal
    (accepted : handleRpcTimeout world sid = .ok decision) :
    Step world decision.event decision.next :=
  decision.legal

theorem accepted_access_is_legal
    (accepted : handleAccess world input = .ok decision) :
    Step world decision.event decision.next :=
  decision.legal

theorem handleRpcTimeout_complete
    (transition : Step world (.rpcTimeout sid) next) :
    ∃ decision, handleRpcTimeout world sid = .ok decision ∧
      decision.event = .rpcTimeout sid ∧ decision.next = next := by
  cases transition
  case rpcTimeout =>
    rename_i record operation ledger current pending operationMatches found
      persisted
    let decision : Decision world :=
      ⟨.rpcTimeout sid, markOutcomeUnknown world sid record operation,
        Step.rpcTimeout found current pending persisted operationMatches⟩
    refine ⟨decision, ?_, rfl, rfl⟩
    unfold handleRpcTimeout
    split
    · rename_i heq
      have impossible : (none : Option SandboxRecord) = some record :=
        heq.symm.trans found
      cases impossible
    · rename_i record' heq
      have same : record' = record := Option.some.inj (heq.symm.trans found)
      subst record'
      split
      · rename_i heqCurrent
        have impossible : (none : Option Operation) = some operation :=
          heqCurrent.symm.trans current
        cases impossible
      · rename_i operation' heqCurrent
        have same : operation' = operation :=
          Option.some.inj (heqCurrent.symm.trans current)
        subst operation'
        rw [dif_pos pending]
        split
        · rename_i heqPersisted
          have impossible : (none : Option OperationLedgerRecord) =
              some ledger := heqPersisted.symm.trans persisted
          cases impossible
        · rename_i ledger' heqPersisted
          have same : ledger' = ledger :=
            Option.some.inj (heqPersisted.symm.trans persisted)
          subst ledger'
          rw [dif_pos operationMatches]

theorem handleExec_complete
    (transition : Step world (.exec actor sid) next) :
    ∃ vm decision, handleExec world actor sid vm = .ok decision ∧
      decision.event = .exec actor sid ∧ decision.next = next := by
  cases transition
  case exec =>
    rename_i record vm auth capability found
    let decision : Decision world :=
      ⟨.exec actor sid, world, Step.exec found auth capability⟩
    refine ⟨vm, decision, ?_, rfl, rfl⟩
    unfold handleExec
    split
    · rename_i heq
      have impossible : (none : Option SandboxRecord) = some record :=
        heq.symm.trans found
      cases impossible
    · rename_i record' heq
      have same : record' = record := Option.some.inj (heq.symm.trans found)
      subst record'
      rw [dif_pos auth, dif_pos capability]

theorem handleFileRead_complete
    (transition : Step world (.fileRead actor sid generation) next) :
    ∃ decision, handleFileRead world actor sid generation = .ok decision ∧
      decision.event = .fileRead actor sid generation ∧ decision.next = next := by
  cases transition
  case fileRead =>
    rename_i record running auth found currentGeneration capability
    let decision : Decision world :=
      ⟨.fileRead actor sid generation, world,
        Step.fileRead found running currentGeneration auth capability⟩
    refine ⟨decision, ?_, rfl, rfl⟩
    unfold handleFileRead
    split
    · rename_i heq
      have impossible : (none : Option SandboxRecord) = some record :=
        heq.symm.trans found
      cases impossible
    · rename_i record' heq
      have same : record' = record := Option.some.inj (heq.symm.trans found)
      subst record'
      rw [dif_pos running, dif_pos currentGeneration, dif_pos auth,
        dif_pos capability]

theorem handleFileWrite_complete
    (transition : Step world (.fileWrite actor sid generation) next) :
    ∃ decision, handleFileWrite world actor sid generation = .ok decision ∧
      decision.event = .fileWrite actor sid generation ∧ decision.next = next := by
  cases transition
  case fileWrite =>
    rename_i record running auth found currentGeneration capability
    let decision : Decision world :=
      ⟨.fileWrite actor sid generation, world,
        Step.fileWrite found running currentGeneration auth capability⟩
    refine ⟨decision, ?_, rfl, rfl⟩
    unfold handleFileWrite
    split
    · rename_i heq
      have impossible : (none : Option SandboxRecord) = some record :=
        heq.symm.trans found
      cases impossible
    · rename_i record' heq
      have same : record' = record := Option.some.inj (heq.symm.trans found)
      subst record'
      rw [dif_pos running, dif_pos currentGeneration, dif_pos auth,
        dif_pos capability]

theorem handleIngress_complete
    (transition : Step world (.ingress actor sid) next) :
    ∃ vm slot decision,
      handleIngress world actor sid vm slot = .ok decision ∧
      decision.event = .ingress actor sid ∧ decision.next = next := by
  cases transition
  case ingress =>
    rename_i record vm slot auth capability found
    let decision : Decision world :=
      ⟨.ingress actor sid, world, Step.ingress found auth capability⟩
    refine ⟨vm, slot, decision, ?_, rfl, rfl⟩
    unfold handleIngress
    split
    · rename_i heq
      have impossible : (none : Option SandboxRecord) = some record :=
        heq.symm.trans found
      cases impossible
    · rename_i record' heq
      have same : record' = record := Option.some.inj (heq.symm.trans found)
      subst record'
      rw [dif_pos auth, dif_pos capability]

theorem handleEgress_complete
    (transition : Step world (.egress sid destination) next) :
    ∃ vm slot decision,
      handleEgress world sid vm slot destination = .ok decision ∧
      decision.event = .egress sid destination ∧ decision.next = next := by
  cases transition
  case egress =>
    rename_i record endpoints vm slot capability found allowed
    rcases allowed with ⟨policyCase, member⟩
    let decision : Decision world :=
      ⟨.egress sid destination, world,
        Step.egress found capability ⟨policyCase, member⟩⟩
    refine ⟨vm, slot, decision, ?_, rfl, rfl⟩
    unfold handleEgress
    split
    · rename_i heq
      have impossible : (none : Option SandboxRecord) = some record :=
        heq.symm.trans found
      cases impossible
    · rename_i record' heq
      have same : record' = record := Option.some.inj (heq.symm.trans found)
      subst record'
      rw [dif_pos capability]
      split
      · rename_i deniedPolicy
        rw [deniedPolicy] at policyCase
        cases policyCase
      · rename_i endpoints' selectedPolicy
        have sameEndpoints : endpoints' = endpoints := by
          have samePolicy := selectedPolicy.symm.trans policyCase
          injection samePolicy
        subst endpoints'
        rw [dif_pos member]

end SandboxModel.Controller
