import SandboxModel.RustRefinement.Values
import SandboxModel.RustRefinement.Clone
import SandboxModel.AuxController
import SandboxModel.Safety

/-!
Refinement bridge for the Rust auxiliary controller entry points.

The access handlers are read-only: acceptance emits an audit event and returns
an exact clone of the persistent world.  The proofs below connect the actual
Aeneas output to the executable model, including guard order, shared
rejections, event translation, and preservation of the complete `WorldRep`.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

noncomputable section

local instance auxiliaryOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance auxiliaryLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance auxiliaryEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq
local instance auxiliaryEndpointDecEq : DecidableEq SandboxControllerRust.model.Endpoint :=
  Classical.decEq _
local instance auxiliaryNetworkPolicyDecEq : DecidableEq SandboxControllerRust.model.NetworkPolicy :=
  Classical.decEq _

abbrev AuxiliaryWorldRep :=
  @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
    effectIdDecidableEq valueRelations

def accessInput : SandboxControllerRust.auxiliary.AccessInput →
    SandboxModel.Controller.AccessInput
  | .Exec actorValue sid vm =>
      .exec (actor actorValue) (sandboxId sid) (vmId vm)
  | .FileRead actorValue sid generation =>
      .fileRead (actor actorValue) (sandboxId sid) (u64ToNat generation)
  | .FileWrite actorValue sid generation =>
      .fileWrite (actor actorValue) (sandboxId sid) (u64ToNat generation)
  | .Ingress actorValue sid vm slot =>
      .ingress (actor actorValue) (sandboxId sid) (vmId vm) (netSlotId slot)
  | .Egress sid vm slot destination =>
      .egress (sandboxId sid) (vmId vm) (netSlotId slot)
        (endpoint destination)

def auxiliaryReject : SandboxControllerRust.auxiliary.AuxiliaryReject →
    SandboxModel.Controller.AuxiliaryReject
  | .SandboxNotFound => .sandboxNotFound
  | .OperationUnavailable => .operationUnavailable
  | .OperationAlreadyFinal => .operationAlreadyFinal
  | .LedgerMismatch => .ledgerMismatch
  | .Unauthorized => .unauthorized
  | .WrongPhase => .wrongPhase
  | .WrongGeneration => .wrongGeneration
  | .CapabilityDenied => .capabilityDenied
  | .DestinationDenied => .destinationDenied

private theorem generated_get_eq [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K) :
    SandboxControllerRust.store.Store.get partialEq store key =
      ok (storeLookup store key) := by
  obtain ⟨value, evaluated, same⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_get_refines partialEq lawful store key)
  subst value
  exact evaluated

private theorem sandboxLookup_cases
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) :
    OptionRel (fun concrete abstract => sandboxRecord concrete = abstract)
      (storeLookup rust.sandboxes sid) (model.sandboxes (sandboxId sid)) :=
  represented.sandboxes.lookupAgrees sid

@[simp] private theorem tenantId_eq_iff {left right} :
    tenantId left = tenantId right ↔ left = right :=
  tenantId_injective.eq_iff

@[simp] private theorem u64_val_eq_iff {left right : Std.U64} :
    left.val = right.val ↔ left = right := by
  constructor
  · exact Std.UScalar.eq_of_val_eq
  · exact fun same => congrArg Std.UScalar.val same

private theorem generated_authorized_eq
    (actorValue : SandboxControllerRust.model.Actor)
    (owner : SandboxControllerRust.ids.TenantId) :
    SandboxControllerRust.transition.authorized actorValue owner =
      ok (decide (SandboxModel.authorized (actor actorValue) (tenantId owner))) := by
  cases actorValue with
  | Tenant tenant =>
      by_cases same : tenant = owner <;>
        simp [SandboxControllerRust.transition.authorized,
          SandboxControllerRust.model.Actor.Insts.CoreCmpPartialEqActor.eq,
          SandboxControllerRust.model.Actor.read_discriminant,
          SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
          SandboxModel.authorized, actor, same]
  | SystemController =>
      simp [SandboxControllerRust.transition.authorized,
        SandboxControllerRust.model.Actor.Insts.CoreCmpPartialEqActor.eq,
        SandboxControllerRust.model.Actor.read_discriminant,
        SandboxModel.authorized, actor]

private theorem generated_allowsExec_eq
    (record : SandboxControllerRust.model.SandboxRecord)
    (sid : SandboxControllerRust.ids.SandboxId)
    (vm : SandboxControllerRust.ids.VmId) :
    SandboxControllerRust.auxiliary.allows_exec record sid vm =
      ok (decide ((sandboxRecord record).effective.allows
        (.exec (sandboxId sid) (sandboxRecord record).generation (vmId vm)))) := by
  cases effectiveCase : record.effective with
  | Closed =>
      simp [SandboxControllerRust.auxiliary.allows_exec, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Paused pausedSid source =>
      simp [SandboxControllerRust.auxiliary.allows_exec, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Running authoritySid generation authorityVm workspace access slot policy =>
      simp only [SandboxControllerRust.auxiliary.allows_exec, effectiveCase,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
      by_cases sameSid : authoritySid = sid <;>
      by_cases sameGeneration : generation = record.generation <;>
      by_cases sameVm : authorityVm = vm <;>
        simp_all [sameSid, sameGeneration, sameVm, sandboxRecord, effectiveCase,
          authority, Authority.allows, u64ToNat,
          SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq]

private theorem generated_allowsFileRead_eq
    (record : SandboxControllerRust.model.SandboxRecord)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : Std.U64) :
    SandboxControllerRust.auxiliary.allows_file record sid generation =
      ok (decide ((sandboxRecord record).effective.allows
        (.fileRead (sandboxId sid) (u64ToNat generation)
          (sandboxRecord record).workspace
          (sandboxRecord record).workspaceAccess))) := by
  cases effectiveCase : record.effective with
  | Closed =>
      simp [SandboxControllerRust.auxiliary.allows_file, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Paused pausedSid source =>
      simp [SandboxControllerRust.auxiliary.allows_file, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Running authoritySid authorityGeneration vm workspace access slot policy =>
      simp only [SandboxControllerRust.auxiliary.allows_file, effectiveCase,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
      by_cases sameSid : authoritySid = sid <;>
      by_cases sameGeneration : authorityGeneration = generation <;>
      by_cases sameWorkspace : workspace = record.workspace <;>
        simp_all [sameSid, sameGeneration, sameWorkspace, sandboxRecord,
          effectiveCase, authority, Authority.allows,
          u64ToNat,
          SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
          SandboxControllerRust.model.WorkspaceApiAccess.Insts.CoreCmpPartialEqWorkspaceApiAccess.eq]

private theorem generated_allowsFileWrite_eq
    (record : SandboxControllerRust.model.SandboxRecord)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : Std.U64) :
    SandboxControllerRust.auxiliary.allows_file record sid generation =
      ok (decide ((sandboxRecord record).effective.allows
        (.fileWrite (sandboxId sid) (u64ToNat generation)
          (sandboxRecord record).workspace
          (sandboxRecord record).workspaceAccess))) := by
  cases effectiveCase : record.effective with
  | Closed =>
      simp [SandboxControllerRust.auxiliary.allows_file, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Paused pausedSid source =>
      simp [SandboxControllerRust.auxiliary.allows_file, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Running authoritySid authorityGeneration vm workspace access slot policy =>
      simp only [SandboxControllerRust.auxiliary.allows_file, effectiveCase,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
      by_cases sameSid : authoritySid = sid <;>
      by_cases sameGeneration : authorityGeneration = generation <;>
      by_cases sameWorkspace : workspace = record.workspace <;>
        simp_all [sameSid, sameGeneration, sameWorkspace, sandboxRecord,
          effectiveCase, authority, Authority.allows,
          u64ToNat,
          SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
          SandboxControllerRust.model.WorkspaceApiAccess.Insts.CoreCmpPartialEqWorkspaceApiAccess.eq]

private theorem generated_endpointEq_eq
    (left right : SandboxControllerRust.model.Endpoint) :
    SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint.eq
        left right = ok (decide (left = right)) := by
  cases left with
  | mk leftAddress leftPrefix leftProtocol leftPort =>
      cases right with
      | mk rightAddress rightPrefix rightProtocol rightPort =>
          simp only [
            SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint.eq]
          by_cases samePrefix : leftPrefix = rightPrefix
          · subst rightPrefix
            by_cases samePort : leftPort = rightPort
            · subst rightPort
              by_cases sameAddress : leftAddress = rightAddress
              · subst rightAddress
                cases leftProtocol <;> cases rightProtocol <;>
                  simp [
                    SandboxControllerRust.model.Protocol.Insts.CoreCmpPartialEqProtocol.eq,
                    SandboxControllerRust.model.Protocol.read_discriminant,
                    alloc.string.String.Insts.CoreCmpPartialEqString.eq]
              · simp [sameAddress,
                  alloc.string.String.Insts.CoreCmpPartialEqString.eq]
            · simp [samePort]
          · simp [samePrefix]

private theorem endpoint_allM_eq : ∀ left right :
    List SandboxControllerRust.model.Endpoint, left.length = right.length →
    List.allM
        (fun pair =>
          SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint.eq
            pair.1 pair.2)
        (List.zip left right) = ok (decide (left = right)) := by
  intro left
  induction left with
  | nil =>
      intro right lengthEq
      cases right with
      | nil => rfl
      | cons _ _ => simp at lengthEq
  | cons head tail induction =>
      intro right lengthEq
      cases right with
      | nil => simp at lengthEq
      | cons other rest =>
          have tailLength : tail.length = rest.length := by
            simpa using lengthEq
          simp only [List.zip_cons_cons, List.allM_cons]
          rw [generated_endpointEq_eq head other]
          by_cases sameHead : head = other
          · subst other
            simpa [Aeneas.Std.instMonadResult] using
              induction rest tailLength
          · simp [sameHead]
            rfl

private theorem generated_endpointVecEq_eq
    (left right : alloc.vec.Vec SandboxControllerRust.model.Endpoint) :
    alloc.vec.partial_eq.PartialEqVec.eq
        SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint
        left right = ok (decide (left = right)) := by
  unfold alloc.vec.partial_eq.PartialEqVec.eq
  by_cases sameLength : left.length = right.length
  · simp only [sameLength, if_pos]
    rw [endpoint_allM_eq left.val right.val (by simpa using sameLength)]
    congr 2
    apply propext
    constructor
    · exact fun same => alloc.vec.Vec.ext left right same
    · exact fun same => congrArg alloc.vec.Vec.val same
  · have different : left ≠ right := by
      intro same
      exact sameLength (congrArg alloc.vec.Vec.length same)
    simp [sameLength, different]

private theorem generated_networkPolicyEq_eq
    (left right : SandboxControllerRust.model.NetworkPolicy) :
    SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq
        left right = ok (decide (left = right)) := by
  cases left with
  | DenyAll =>
      cases right <;>
        simp [
          SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
          SandboxControllerRust.model.NetworkPolicy.read_discriminant]
  | StaticAllowlist leftEndpoints =>
      cases right with
      | DenyAll =>
          simp [
            SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
            SandboxControllerRust.model.NetworkPolicy.read_discriminant]
      | StaticAllowlist rightEndpoints =>
          simp only [
            SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
            SandboxControllerRust.model.NetworkPolicy.read_discriminant,
            ↓reduceIte]
          rw [generated_endpointVecEq_eq leftEndpoints rightEndpoints]
          congr 2
          apply propext
          simp

private def EffectivePolicyExact
    (record : SandboxControllerRust.model.SandboxRecord) : Prop :=
  match record.effective with
  | .Running _ _ _ _ _ _ policy => policy = record.policy
  | .Closed | .Paused _ _ => True

set_option maxHeartbeats 1000000 in
private theorem effectivePolicyExact_of_authorityShape
    (model : World) (shape : AuthorityShape model)
    (record : SandboxControllerRust.model.SandboxRecord)
    (sid : SandboxControllerRust.ids.SandboxId)
    (found : model.sandboxes (sandboxId sid) = some (sandboxRecord record)) :
    EffectivePolicyExact record := by
  have shaped := shape (sandboxId sid) (sandboxRecord record) found
  cases phaseCase : record.phase <;>
  cases effectiveCase : record.effective <;>
    simp_all [EffectivePolicyExact, sandboxRecord, phaseCase, effectiveCase,
      phase, authority, networkPolicy_eq_iff]

private theorem generated_allowsIngress_eq
    (record : SandboxControllerRust.model.SandboxRecord)
    (sid : SandboxControllerRust.ids.SandboxId)
    (vm : SandboxControllerRust.ids.VmId)
    (slot : SandboxControllerRust.ids.NetSlotId)
    (policyExact : EffectivePolicyExact record) :
    SandboxControllerRust.auxiliary.allows_network record sid vm slot =
      ok (decide ((sandboxRecord record).effective.allows
        (.ingress (sandboxId sid) (sandboxRecord record).generation
          (vmId vm) (netSlotId slot)))) := by
  cases effectiveCase : record.effective with
  | Closed =>
      simp [SandboxControllerRust.auxiliary.allows_network, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Paused pausedSid source =>
      simp [SandboxControllerRust.auxiliary.allows_network, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Running authoritySid generation authorityVm workspace access authoritySlot policy =>
      have policySame : policy = record.policy := by
        simpa [EffectivePolicyExact, effectiveCase] using policyExact
      simp only [SandboxControllerRust.auxiliary.allows_network, effectiveCase,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
      by_cases sameSid : authoritySid = sid <;>
      by_cases sameGeneration : generation = record.generation <;>
      by_cases sameVm : authorityVm = vm <;>
      by_cases sameSlot : authoritySlot = slot <;>
      simp_all [sameSid, sameGeneration, sameVm, sameSlot, policySame,
          sandboxRecord, effectiveCase, authority, Authority.allows,
          u64ToNat,
          SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
          SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId.eq,
          generated_networkPolicyEq_eq]

private theorem generated_allowsEgress_eq
    (record : SandboxControllerRust.model.SandboxRecord)
    (sid : SandboxControllerRust.ids.SandboxId)
    (vm : SandboxControllerRust.ids.VmId)
    (slot : SandboxControllerRust.ids.NetSlotId) :
    SandboxControllerRust.auxiliary.allows_network record sid vm slot =
      ok (decide ((sandboxRecord record).effective.allows
        (.egress (sandboxId sid) (sandboxRecord record).generation
          (vmId vm) (netSlotId slot) (sandboxRecord record).policy))) := by
  cases effectiveCase : record.effective with
  | Closed =>
      simp [SandboxControllerRust.auxiliary.allows_network, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Paused pausedSid source =>
      simp [SandboxControllerRust.auxiliary.allows_network, sandboxRecord,
        effectiveCase, authority, Authority.allows]
  | Running authoritySid generation authorityVm workspace access authoritySlot policy =>
      simp only [SandboxControllerRust.auxiliary.allows_network, effectiveCase,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
      by_cases sameSid : authoritySid = sid <;>
      by_cases sameGeneration : generation = record.generation <;>
      by_cases sameVm : authorityVm = vm <;>
      by_cases sameSlot : authoritySlot = slot <;>
      by_cases samePolicy : policy = record.policy
      all_goals
        have reversePolicy : record.policy = policy ↔ policy = record.policy := by
          constructor <;> intro same <;> exact same.symm
        simp_all [sameSid, sameGeneration, sameVm, sameSlot, samePolicy,
          reversePolicy,
          sandboxRecord, effectiveCase, authority, Authority.allows,
          u64ToNat,
          SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
          SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId.eq,
          generated_networkPolicyEq_eq]

private theorem generated_phase_ne_running
    (value : SandboxControllerRust.model.Phase) :
    core.cmp.PartialEq.ne.trait_default
        SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase value
        .Running = ok (decide (phase value ≠ .running)) := by
  cases value <;>
    simp [core.cmp.PartialEq.ne.trait_default,
      core.cmp.PartialEq.ne.default,
      SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase.eq,
      SandboxControllerRust.model.Phase.read_discriminant, phase]

private theorem phase_injective : Function.Injective phase := by
  intro left right same
  cases left <;> cases right <;> simp_all [phase]

private theorem generated_destinationAllowed_loop_refines
    (destination : SandboxControllerRust.model.Endpoint)
    (endpoints : alloc.vec.Vec SandboxControllerRust.model.Endpoint)
    (start : Std.Usize) (allowed : Bool)
    (startBound : start.val ≤ endpoints.val.length) :
    SandboxControllerRust.auxiliary.destination_allowed_loop destination
        endpoints start allowed
      ⦃ result =>
        result =
          (allowed || decide (destination ∈ endpoints.val.drop start.val)) ⦄ := by
  classical
  unfold SandboxControllerRust.auxiliary.destination_allowed_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : Std.Usize × Bool => endpoints.val.length - state.1.val)
    (fun state : Std.Usize × Bool =>
      state.1.val ≤ endpoints.val.length ∧
      (state.2 || decide (destination ∈ endpoints.val.drop state.1.val)) =
        (allowed || decide (destination ∈ endpoints.val.drop start.val)))
    (fun result : Bool =>
      result =
        (allowed || decide (destination ∈ endpoints.val.drop start.val)))
  · rintro ⟨cursor, found⟩ invariant
    unfold SandboxControllerRust.auxiliary.destination_allowed_loop.body
    dsimp only
    split
    · rename_i inBounds
      by_cases alreadyFound : found = true
      · simp [alreadyFound]
        simpa [alreadyFound] using invariant.2.symm
      · have foundFalse : found = false := by
          cases found <;> simp_all
        simp [foundFalse]
        step as ⟨current, currentPost⟩
        have indexBound : cursor.val < endpoints.val.length := by
          simpa using inBounds
        have dropEq : endpoints.val.drop cursor.val =
            current :: endpoints.val.drop (cursor.val + 1) := by
          rw [List.drop_eq_getElem_cons indexBound]
          rw [← currentPost]
        rw [generated_endpointEq_eq current destination]
        by_cases same : current = destination
        · simp [same]
          step as ⟨nextCursor, nextCursorPost⟩
          have targetTrue :
              (allowed || decide
                (destination ∈ endpoints.val.drop start.val)) = true := by
            have old := invariant.2
            rw [dropEq] at old
            simpa [foundFalse, same] using old.symm
          have nextCursorValue : nextCursor.val = cursor.val + 1 :=
            nextCursorPost
          constructor
          · omega
          · constructor
            · simp [targetTrue]
            · omega
        · simp [same]
          step as ⟨nextCursor, nextCursorPost⟩
          have reverseDifferent : destination ≠ current := Ne.symm same
          have nextCursorValue : nextCursor.val = cursor.val + 1 :=
            nextCursorPost
          constructor
          · omega
          · constructor
            · rw [nextCursorValue]
              rcases invariant with ⟨_, invariant⟩
              rw [dropEq] at invariant
              simpa [foundFalse, same, reverseDifferent] using invariant
            · omega
    · rename_i outOfBounds
      simp only [WP.spec_ok]
      rcases invariant with ⟨upper, invariant⟩
      have lower : endpoints.val.length ≤ cursor.val := by
        simpa using outOfBounds
      have dropEmpty : endpoints.val.drop cursor.val = [] :=
        List.drop_eq_nil_of_le lower
      rw [dropEmpty] at invariant
      simpa using invariant
  · exact ⟨startBound, rfl⟩

private theorem generated_destinationAllowed_deny
    (record : SandboxControllerRust.model.SandboxRecord)
    (destination : SandboxControllerRust.model.Endpoint)
    (policyCase : record.policy = .DenyAll) :
    SandboxControllerRust.auxiliary.destination_allowed record destination =
      ok false := by
  simp [SandboxControllerRust.auxiliary.destination_allowed, policyCase]

private theorem generated_destinationAllowed_static
    (record : SandboxControllerRust.model.SandboxRecord)
    (destination : SandboxControllerRust.model.Endpoint)
    (endpoints : alloc.vec.Vec SandboxControllerRust.model.Endpoint)
    (policyCase : record.policy = .StaticAllowlist endpoints) :
    SandboxControllerRust.auxiliary.destination_allowed record destination =
      ok (decide (destination ∈ endpoints.val)) := by
  unfold SandboxControllerRust.auxiliary.destination_allowed
  rw [policyCase]
  change SandboxControllerRust.auxiliary.destination_allowed_loop destination
      endpoints 0#usize false = ok (decide (destination ∈ endpoints.val))
  obtain ⟨result, evaluated, resultSpec⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_destinationAllowed_loop_refines destination endpoints
      0#usize false (by simp))
  rw [evaluated]
  simpa using resultSpec

def AccessResultRep (model : World)
    (rustResult : core.result.Result
      SandboxControllerRust.auxiliary.AuxiliaryDecision
      SandboxControllerRust.auxiliary.AuxiliaryReject)
    (modelResult : Except SandboxModel.Controller.AuxiliaryReject
      (SandboxModel.Controller.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err rustReject, .error modelReject =>
      auxiliaryReject rustReject = modelReject
  | .Ok rustDecision, .ok modelDecision =>
      auxiliaryEvent rustDecision.event = modelDecision.event ∧
        AuxiliaryWorldRep rustDecision.next modelDecision.next
  | _, _ => False

private theorem handleExec_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (actorValue : SandboxControllerRust.model.Actor)
    (sid : SandboxControllerRust.ids.SandboxId)
    (vm : SandboxControllerRust.ids.VmId) :
    ∃ rustResult,
      SandboxControllerRust.auxiliary.handle_exec rust actorValue sid vm =
        ok rustResult ∧
      AccessResultRep model rustResult
        (SandboxModel.Controller.handleExec model (actor actorValue)
          (sandboxId sid) (vmId vm)) := by
  have getEq := generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  cases concreteLookup : storeLookup rust.sandboxes sid with
  | none =>
      have modelLookup : model.sandboxes (sandboxId sid) = none := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) <;>
          simp_all [OptionRel]
      refine ⟨.Err .SandboxNotFound, ?_, ?_⟩
      · simp [SandboxControllerRust.auxiliary.handle_exec, getEq,
          concreteLookup]
      · have modelEvaluates :
            SandboxModel.Controller.handleExec model (actor actorValue)
                (sandboxId sid) (vmId vm) = .error .sandboxNotFound := by
            unfold SandboxModel.Controller.handleExec
            split
            · rfl
            · rename_i foundRecord heq
              have impossible : (none : Option SandboxRecord) =
                  some foundRecord := modelLookup.symm.trans heq
              cases impossible
        rw [modelEvaluates]
        rfl
  | some record =>
      have modelLookup : model.sandboxes (sandboxId sid) =
          some (sandboxRecord record) := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) with
        | none => simp [OptionRel, abstractLookup] at agrees
        | some abstractRecord =>
            rw [abstractLookup] at agrees
            simp only [OptionRel] at agrees
            exact congrArg some agrees.symm
      by_cases auth : SandboxModel.authorized (actor actorValue)
          (sandboxRecord record).owner
      · by_cases capability : (sandboxRecord record).effective.allows
            (.exec (sandboxId sid) (sandboxRecord record).generation (vmId vm))
        · let rustDecision : SandboxControllerRust.auxiliary.AuxiliaryDecision :=
            { event := .Exec actorValue sid, next := rust }
          have concreteAuth : SandboxModel.authorized (actor actorValue)
              (tenantId record.owner) := by
            simpa [sandboxRecord] using auth
          refine ⟨.Ok rustDecision, ?_, ?_⟩
          · simp [SandboxControllerRust.auxiliary.handle_exec, getEq,
              concreteLookup, generated_authorized_eq, concreteAuth,
              generated_allowsExec_eq, capability, world_clone_eq,
              rustDecision]
          · have modelEvaluates :
                SandboxModel.Controller.handleExec model (actor actorValue)
                    (sandboxId sid) (vmId vm) =
                  .ok ⟨.exec (actor actorValue) (sandboxId sid), model,
                    Step.exec modelLookup auth capability⟩ := by
                unfold SandboxModel.Controller.handleExec
                split
                · rename_i heq
                  have impossible : (none : Option SandboxRecord) =
                      some (sandboxRecord record) := heq.symm.trans modelLookup
                  cases impossible
                · rename_i modelRecord heq
                  have same : modelRecord = sandboxRecord record :=
                    Option.some.inj (heq.symm.trans modelLookup)
                  subst modelRecord
                  rw [dif_pos auth, dif_pos capability]
            rw [modelEvaluates]
            exact ⟨rfl, represented⟩
        · have concreteAuth : SandboxModel.authorized (actor actorValue)
              (tenantId record.owner) := by
            simpa [sandboxRecord] using auth
          refine ⟨.Err .CapabilityDenied, ?_, ?_⟩
          · simp [SandboxControllerRust.auxiliary.handle_exec, getEq,
              concreteLookup, generated_authorized_eq, concreteAuth,
              generated_allowsExec_eq, capability]
          · have modelEvaluates :
                SandboxModel.Controller.handleExec model (actor actorValue)
                    (sandboxId sid) (vmId vm) = .error .capabilityDenied := by
                unfold SandboxModel.Controller.handleExec
                split
                · rename_i heq
                  have impossible : (none : Option SandboxRecord) =
                      some (sandboxRecord record) := heq.symm.trans modelLookup
                  cases impossible
                · rename_i modelRecord heq
                  have same : modelRecord = sandboxRecord record :=
                    Option.some.inj (heq.symm.trans modelLookup)
                  subst modelRecord
                  rw [dif_pos auth, dif_neg capability]
            rw [modelEvaluates]
            rfl
      · have concreteAuth : ¬ SandboxModel.authorized (actor actorValue)
            (tenantId record.owner) := by
          simpa [sandboxRecord] using auth
        refine ⟨.Err .Unauthorized, ?_, ?_⟩
        · simp [SandboxControllerRust.auxiliary.handle_exec, getEq,
            concreteLookup, generated_authorized_eq, concreteAuth]
        · have modelEvaluates :
              SandboxModel.Controller.handleExec model (actor actorValue)
                  (sandboxId sid) (vmId vm) = .error .unauthorized := by
              unfold SandboxModel.Controller.handleExec
              split
              · rename_i heq
                have impossible : (none : Option SandboxRecord) =
                    some (sandboxRecord record) := heq.symm.trans modelLookup
                cases impossible
              · rename_i modelRecord heq
                have same : modelRecord = sandboxRecord record :=
                  Option.some.inj (heq.symm.trans modelLookup)
                subst modelRecord
                rw [dif_neg auth]
          rw [modelEvaluates]
          rfl

private theorem handleFileRead_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (actorValue : SandboxControllerRust.model.Actor)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : Std.U64) :
    ∃ rustResult,
      SandboxControllerRust.auxiliary.handle_file_read rust actorValue sid
          generation = ok rustResult ∧
      AccessResultRep model rustResult
        (SandboxModel.Controller.handleFileRead model (actor actorValue)
          (sandboxId sid) (u64ToNat generation)) := by
  have getEq := generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  cases concreteLookup : storeLookup rust.sandboxes sid with
  | none =>
      have modelLookup : model.sandboxes (sandboxId sid) = none := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) <;>
          simp_all [OptionRel]
      refine ⟨.Err .SandboxNotFound, ?_, ?_⟩
      · simp [SandboxControllerRust.auxiliary.handle_file_read, getEq,
          concreteLookup]
      · have modelEvaluates :
            SandboxModel.Controller.handleFileRead model (actor actorValue)
                (sandboxId sid) (u64ToNat generation) =
              .error .sandboxNotFound := by
            unfold SandboxModel.Controller.handleFileRead
            split
            · rfl
            · rename_i foundRecord heq
              have impossible : (none : Option SandboxRecord) =
                  some foundRecord := modelLookup.symm.trans heq
              cases impossible
        rw [modelEvaluates]
        rfl
  | some record =>
      have modelLookup : model.sandboxes (sandboxId sid) =
          some (sandboxRecord record) := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) with
        | none => simp [OptionRel, abstractLookup] at agrees
        | some abstractRecord =>
            rw [abstractLookup] at agrees
            simp only [OptionRel] at agrees
            exact congrArg some agrees.symm
      by_cases running : record.phase = .Running
      · have modelRunning : (sandboxRecord record).phase = .running := by
          simp [sandboxRecord, running, phase]
        by_cases sameGeneration : generation = record.generation
        · have modelGeneration : u64ToNat generation =
              (sandboxRecord record).generation := by
            simp [sandboxRecord, sameGeneration]
          by_cases auth : SandboxModel.authorized (actor actorValue)
              (sandboxRecord record).owner
          · by_cases capability : (sandboxRecord record).effective.allows
                (.fileRead (sandboxId sid) (u64ToNat generation)
                  (sandboxRecord record).workspace
                  (sandboxRecord record).workspaceAccess)
            · let rustDecision :
                  SandboxControllerRust.auxiliary.AuxiliaryDecision :=
                { event := .FileRead actorValue sid generation, next := rust }
              have concreteAuth : SandboxModel.authorized (actor actorValue)
                  (tenantId record.owner) := by
                simpa [sandboxRecord] using auth
              have concreteCapability : (sandboxRecord record).effective.allows
                  (.fileRead (sandboxId sid) (u64ToNat record.generation)
                    (sandboxRecord record).workspace
                    (sandboxRecord record).workspaceAccess) := by
                simpa [sameGeneration] using capability
              refine ⟨.Ok rustDecision, ?_, ?_⟩
              · simp [SandboxControllerRust.auxiliary.handle_file_read,
                  getEq, concreteLookup, generated_phase_ne_running, running,
                  phase, sameGeneration, generated_authorized_eq, concreteAuth,
                  generated_allowsFileRead_eq, concreteCapability,
                  world_clone_eq,
                  rustDecision]
              · let transition : Step model
                    (.fileRead (actor actorValue) (sandboxId sid)
                      (u64ToNat generation)) model :=
                    Step.fileRead modelLookup modelRunning modelGeneration auth
                      capability
                rcases SandboxModel.Controller.handleFileRead_complete transition with
                  ⟨modelDecision, modelEvaluates, eventSame, nextSame⟩
                rw [modelEvaluates]
                refine ⟨?_, ?_⟩
                · simpa [rustDecision, auxiliaryEvent] using eventSame.symm
                · simpa [rustDecision, nextSame] using represented
            · have concreteAuth : SandboxModel.authorized (actor actorValue)
                  (tenantId record.owner) := by
                simpa [sandboxRecord] using auth
              have concreteCapability : ¬
                  (sandboxRecord record).effective.allows
                    (.fileRead (sandboxId sid) (u64ToNat record.generation)
                      (sandboxRecord record).workspace
                      (sandboxRecord record).workspaceAccess) := by
                simpa [sameGeneration] using capability
              refine ⟨.Err .CapabilityDenied, ?_, ?_⟩
              · simp [SandboxControllerRust.auxiliary.handle_file_read,
                  getEq, concreteLookup, generated_phase_ne_running, running,
                  phase, sameGeneration, generated_authorized_eq, concreteAuth,
                  generated_allowsFileRead_eq, concreteCapability]
              · have modelEvaluates :
                    SandboxModel.Controller.handleFileRead model
                        (actor actorValue) (sandboxId sid)
                        (u64ToNat generation) = .error .capabilityDenied := by
                    unfold SandboxModel.Controller.handleFileRead
                    split
                    · rename_i heq
                      have impossible : (none : Option SandboxRecord) =
                          some (sandboxRecord record) := heq.symm.trans modelLookup
                      cases impossible
                    · rename_i modelRecord heq
                      have same : modelRecord = sandboxRecord record :=
                        Option.some.inj (heq.symm.trans modelLookup)
                      subst modelRecord
                      rw [dif_pos modelRunning, dif_pos modelGeneration,
                        dif_pos auth, dif_neg capability]
                rw [modelEvaluates]
                rfl
          · have concreteAuth : ¬ SandboxModel.authorized (actor actorValue)
                (tenantId record.owner) := by
              simpa [sandboxRecord] using auth
            refine ⟨.Err .Unauthorized, ?_, ?_⟩
            · simp [SandboxControllerRust.auxiliary.handle_file_read,
                getEq, concreteLookup, generated_phase_ne_running, running,
                phase, sameGeneration, generated_authorized_eq, concreteAuth]
            · have modelEvaluates :
                  SandboxModel.Controller.handleFileRead model
                      (actor actorValue) (sandboxId sid)
                      (u64ToNat generation) = .error .unauthorized := by
                  unfold SandboxModel.Controller.handleFileRead
                  split
                  · rename_i heq
                    have impossible : (none : Option SandboxRecord) =
                        some (sandboxRecord record) := heq.symm.trans modelLookup
                    cases impossible
                  · rename_i modelRecord heq
                    have same : modelRecord = sandboxRecord record :=
                      Option.some.inj (heq.symm.trans modelLookup)
                    subst modelRecord
                    rw [dif_pos modelRunning, dif_pos modelGeneration,
                      dif_neg auth]
              rw [modelEvaluates]
              rfl
        · have modelGeneration : u64ToNat generation ≠
              (sandboxRecord record).generation := by
            simp only [sandboxRecord]
            exact fun same => sameGeneration (u64ToNat_injective same)
          refine ⟨.Err .WrongGeneration, ?_, ?_⟩
          · simp [SandboxControllerRust.auxiliary.handle_file_read, getEq,
              concreteLookup, generated_phase_ne_running, running,
              phase, sameGeneration]
          · have modelEvaluates :
                SandboxModel.Controller.handleFileRead model
                    (actor actorValue) (sandboxId sid) (u64ToNat generation) =
                  .error .wrongGeneration := by
                unfold SandboxModel.Controller.handleFileRead
                split
                · rename_i heq
                  have impossible : (none : Option SandboxRecord) =
                      some (sandboxRecord record) := heq.symm.trans modelLookup
                  cases impossible
                · rename_i modelRecord heq
                  have same : modelRecord = sandboxRecord record :=
                    Option.some.inj (heq.symm.trans modelLookup)
                  subst modelRecord
                  rw [dif_pos modelRunning, dif_neg modelGeneration]
            rw [modelEvaluates]
            rfl
      · have modelRunning : (sandboxRecord record).phase ≠ .running := by
          simp only [sandboxRecord]
          exact fun same => running (phase_injective same)
        have concretePhaseNotRunning : phase record.phase ≠ .running := by
          exact fun same => running (phase_injective same)
        have generatedPhase :
            core.cmp.PartialEq.ne.trait_default
                SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase
                record.phase .Running = ok true := by
          rw [generated_phase_ne_running]
          simp [concretePhaseNotRunning]
        refine ⟨.Err .WrongPhase, ?_, ?_⟩
        · simp [SandboxControllerRust.auxiliary.handle_file_read, getEq,
            concreteLookup, generatedPhase]
        · have modelEvaluates :
              SandboxModel.Controller.handleFileRead model (actor actorValue)
                  (sandboxId sid) (u64ToNat generation) = .error .wrongPhase := by
              unfold SandboxModel.Controller.handleFileRead
              split
              · rename_i heq
                have impossible : (none : Option SandboxRecord) =
                    some (sandboxRecord record) := heq.symm.trans modelLookup
                cases impossible
              · rename_i modelRecord heq
                have same : modelRecord = sandboxRecord record :=
                  Option.some.inj (heq.symm.trans modelLookup)
                subst modelRecord
                rw [dif_neg modelRunning]
          rw [modelEvaluates]
          rfl

private theorem handleFileWrite_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (actorValue : SandboxControllerRust.model.Actor)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : Std.U64) :
    ∃ rustResult,
      SandboxControllerRust.auxiliary.handle_file_write rust actorValue sid
          generation = ok rustResult ∧
      AccessResultRep model rustResult
        (SandboxModel.Controller.handleFileWrite model (actor actorValue)
          (sandboxId sid) (u64ToNat generation)) := by
  have getEq := generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  cases concreteLookup : storeLookup rust.sandboxes sid with
  | none =>
      have modelLookup : model.sandboxes (sandboxId sid) = none := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) <;>
          simp_all [OptionRel]
      refine ⟨.Err .SandboxNotFound, ?_, ?_⟩
      · simp [SandboxControllerRust.auxiliary.handle_file_write, getEq,
          concreteLookup]
      · have modelEvaluates :
            SandboxModel.Controller.handleFileWrite model (actor actorValue)
                (sandboxId sid) (u64ToNat generation) =
              .error .sandboxNotFound := by
            unfold SandboxModel.Controller.handleFileWrite
            split
            · rfl
            · rename_i foundRecord heq
              have impossible : (none : Option SandboxRecord) =
                  some foundRecord := modelLookup.symm.trans heq
              cases impossible
        rw [modelEvaluates]
        rfl
  | some record =>
      have modelLookup : model.sandboxes (sandboxId sid) =
          some (sandboxRecord record) := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) with
        | none => simp [OptionRel, abstractLookup] at agrees
        | some abstractRecord =>
            rw [abstractLookup] at agrees
            simp only [OptionRel] at agrees
            exact congrArg some agrees.symm
      by_cases running : record.phase = .Running
      · have modelRunning : (sandboxRecord record).phase = .running := by
          simp [sandboxRecord, running, phase]
        by_cases sameGeneration : generation = record.generation
        · have modelGeneration : u64ToNat generation =
              (sandboxRecord record).generation := by
            simp [sandboxRecord, sameGeneration]
          by_cases auth : SandboxModel.authorized (actor actorValue)
              (sandboxRecord record).owner
          · by_cases capability : (sandboxRecord record).effective.allows
                (.fileWrite (sandboxId sid) (u64ToNat generation)
                  (sandboxRecord record).workspace
                  (sandboxRecord record).workspaceAccess)
            · let rustDecision :
                  SandboxControllerRust.auxiliary.AuxiliaryDecision :=
                { event := .FileWrite actorValue sid generation, next := rust }
              have concreteAuth : SandboxModel.authorized (actor actorValue)
                  (tenantId record.owner) := by
                simpa [sandboxRecord] using auth
              have concreteCapability : (sandboxRecord record).effective.allows
                  (.fileWrite (sandboxId sid) (u64ToNat record.generation)
                    (sandboxRecord record).workspace
                    (sandboxRecord record).workspaceAccess) := by
                simpa [sameGeneration] using capability
              refine ⟨.Ok rustDecision, ?_, ?_⟩
              · simp [SandboxControllerRust.auxiliary.handle_file_write,
                  getEq, concreteLookup, generated_phase_ne_running, running,
                  phase, sameGeneration, generated_authorized_eq, concreteAuth,
                  generated_allowsFileWrite_eq, concreteCapability,
                  world_clone_eq,
                  rustDecision]
              · let transition : Step model
                    (.fileWrite (actor actorValue) (sandboxId sid)
                      (u64ToNat generation)) model :=
                    Step.fileWrite modelLookup modelRunning modelGeneration auth
                      capability
                rcases SandboxModel.Controller.handleFileWrite_complete transition with
                  ⟨modelDecision, modelEvaluates, eventSame, nextSame⟩
                rw [modelEvaluates]
                refine ⟨?_, ?_⟩
                · simpa [rustDecision, auxiliaryEvent] using eventSame.symm
                · simpa [rustDecision, nextSame] using represented
            · have concreteAuth : SandboxModel.authorized (actor actorValue)
                  (tenantId record.owner) := by
                simpa [sandboxRecord] using auth
              have concreteCapability : ¬
                  (sandboxRecord record).effective.allows
                    (.fileWrite (sandboxId sid) (u64ToNat record.generation)
                      (sandboxRecord record).workspace
                      (sandboxRecord record).workspaceAccess) := by
                simpa [sameGeneration] using capability
              refine ⟨.Err .CapabilityDenied, ?_, ?_⟩
              · simp [SandboxControllerRust.auxiliary.handle_file_write,
                  getEq, concreteLookup, generated_phase_ne_running, running,
                  phase, sameGeneration, generated_authorized_eq, concreteAuth,
                  generated_allowsFileWrite_eq, concreteCapability]
              · have modelEvaluates :
                    SandboxModel.Controller.handleFileWrite model
                        (actor actorValue) (sandboxId sid)
                        (u64ToNat generation) = .error .capabilityDenied := by
                    unfold SandboxModel.Controller.handleFileWrite
                    split
                    · rename_i heq
                      have impossible : (none : Option SandboxRecord) =
                          some (sandboxRecord record) := heq.symm.trans modelLookup
                      cases impossible
                    · rename_i modelRecord heq
                      have same : modelRecord = sandboxRecord record :=
                        Option.some.inj (heq.symm.trans modelLookup)
                      subst modelRecord
                      rw [dif_pos modelRunning, dif_pos modelGeneration,
                        dif_pos auth, dif_neg capability]
                rw [modelEvaluates]
                rfl
          · have concreteAuth : ¬ SandboxModel.authorized (actor actorValue)
                (tenantId record.owner) := by
              simpa [sandboxRecord] using auth
            refine ⟨.Err .Unauthorized, ?_, ?_⟩
            · simp [SandboxControllerRust.auxiliary.handle_file_write,
                getEq, concreteLookup, generated_phase_ne_running, running,
                phase, sameGeneration, generated_authorized_eq, concreteAuth]
            · have modelEvaluates :
                  SandboxModel.Controller.handleFileWrite model
                      (actor actorValue) (sandboxId sid)
                      (u64ToNat generation) = .error .unauthorized := by
                  unfold SandboxModel.Controller.handleFileWrite
                  split
                  · rename_i heq
                    have impossible : (none : Option SandboxRecord) =
                        some (sandboxRecord record) := heq.symm.trans modelLookup
                    cases impossible
                  · rename_i modelRecord heq
                    have same : modelRecord = sandboxRecord record :=
                      Option.some.inj (heq.symm.trans modelLookup)
                    subst modelRecord
                    rw [dif_pos modelRunning, dif_pos modelGeneration,
                      dif_neg auth]
              rw [modelEvaluates]
              rfl
        · have modelGeneration : u64ToNat generation ≠
              (sandboxRecord record).generation := by
            simp only [sandboxRecord]
            exact fun same => sameGeneration (u64ToNat_injective same)
          refine ⟨.Err .WrongGeneration, ?_, ?_⟩
          · simp [SandboxControllerRust.auxiliary.handle_file_write, getEq,
              concreteLookup, generated_phase_ne_running, running,
              phase, sameGeneration]
          · have modelEvaluates :
                SandboxModel.Controller.handleFileWrite model
                    (actor actorValue) (sandboxId sid) (u64ToNat generation) =
                  .error .wrongGeneration := by
                unfold SandboxModel.Controller.handleFileWrite
                split
                · rename_i heq
                  have impossible : (none : Option SandboxRecord) =
                      some (sandboxRecord record) := heq.symm.trans modelLookup
                  cases impossible
                · rename_i modelRecord heq
                  have same : modelRecord = sandboxRecord record :=
                    Option.some.inj (heq.symm.trans modelLookup)
                  subst modelRecord
                  rw [dif_pos modelRunning, dif_neg modelGeneration]
            rw [modelEvaluates]
            rfl
      · have modelRunning : (sandboxRecord record).phase ≠ .running := by
          simp only [sandboxRecord]
          exact fun same => running (phase_injective same)
        have concretePhaseNotRunning : phase record.phase ≠ .running := by
          exact fun same => running (phase_injective same)
        have generatedPhase :
            core.cmp.PartialEq.ne.trait_default
                SandboxControllerRust.model.Phase.Insts.CoreCmpPartialEqPhase
                record.phase .Running = ok true := by
          rw [generated_phase_ne_running]
          simp [concretePhaseNotRunning]
        refine ⟨.Err .WrongPhase, ?_, ?_⟩
        · simp [SandboxControllerRust.auxiliary.handle_file_write, getEq,
            concreteLookup, generatedPhase]
        · have modelEvaluates :
              SandboxModel.Controller.handleFileWrite model (actor actorValue)
                  (sandboxId sid) (u64ToNat generation) = .error .wrongPhase := by
              unfold SandboxModel.Controller.handleFileWrite
              split
              · rename_i heq
                have impossible : (none : Option SandboxRecord) =
                    some (sandboxRecord record) := heq.symm.trans modelLookup
                cases impossible
              · rename_i modelRecord heq
                have same : modelRecord = sandboxRecord record :=
                  Option.some.inj (heq.symm.trans modelLookup)
                subst modelRecord
                rw [dif_neg modelRunning]
          rw [modelEvaluates]
          rfl

private theorem handleIngress_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (authorityShape : AuthorityShape model)
    (actorValue : SandboxControllerRust.model.Actor)
    (sid : SandboxControllerRust.ids.SandboxId)
    (vm : SandboxControllerRust.ids.VmId)
    (slot : SandboxControllerRust.ids.NetSlotId) :
    ∃ rustResult,
      SandboxControllerRust.auxiliary.handle_ingress rust actorValue sid vm
          slot = ok rustResult ∧
      AccessResultRep model rustResult
        (SandboxModel.Controller.handleIngress model (actor actorValue)
          (sandboxId sid) (vmId vm) (netSlotId slot)) := by
  have getEq := generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  cases concreteLookup : storeLookup rust.sandboxes sid with
  | none =>
      have modelLookup : model.sandboxes (sandboxId sid) = none := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) <;>
          simp_all [OptionRel]
      refine ⟨.Err .SandboxNotFound, ?_, ?_⟩
      · simp [SandboxControllerRust.auxiliary.handle_ingress, getEq,
          concreteLookup]
      · have modelEvaluates :
            SandboxModel.Controller.handleIngress model (actor actorValue)
                (sandboxId sid) (vmId vm) (netSlotId slot) =
              .error .sandboxNotFound := by
            unfold SandboxModel.Controller.handleIngress
            split
            · rfl
            · rename_i foundRecord heq
              have impossible : (none : Option SandboxRecord) =
                  some foundRecord := modelLookup.symm.trans heq
              cases impossible
        rw [modelEvaluates]
        rfl
  | some record =>
      have modelLookup : model.sandboxes (sandboxId sid) =
          some (sandboxRecord record) := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) with
        | none => simp [OptionRel, abstractLookup] at agrees
        | some abstractRecord =>
            rw [abstractLookup] at agrees
            simp only [OptionRel] at agrees
            exact congrArg some agrees.symm
      by_cases auth : SandboxModel.authorized (actor actorValue)
          (sandboxRecord record).owner
      · by_cases capability : (sandboxRecord record).effective.allows
            (.ingress (sandboxId sid) (sandboxRecord record).generation
              (vmId vm) (netSlotId slot))
        · let rustDecision : SandboxControllerRust.auxiliary.AuxiliaryDecision :=
            { event := .Ingress actorValue sid, next := rust }
          have policyExact := effectivePolicyExact_of_authorityShape model
            authorityShape record sid modelLookup
          have concreteAuth : SandboxModel.authorized (actor actorValue)
              (tenantId record.owner) := by
            simpa [sandboxRecord] using auth
          refine ⟨.Ok rustDecision, ?_, ?_⟩
          · simp [SandboxControllerRust.auxiliary.handle_ingress, getEq,
              concreteLookup, generated_authorized_eq, concreteAuth,
              generated_allowsIngress_eq record sid vm slot policyExact,
              capability, world_clone_eq,
              rustDecision]
          · let modelDecision : SandboxModel.Controller.Decision model :=
                ⟨.ingress (actor actorValue) (sandboxId sid), model,
                  Step.ingress modelLookup auth capability⟩
            have modelEvaluates :
                SandboxModel.Controller.handleIngress model (actor actorValue)
                    (sandboxId sid) (vmId vm) (netSlotId slot) =
                  .ok modelDecision := by
              unfold SandboxModel.Controller.handleIngress
              split
              · rename_i heq
                have impossible : (none : Option SandboxRecord) =
                    some (sandboxRecord record) := heq.symm.trans modelLookup
                cases impossible
              · rename_i modelRecord heq
                have same : modelRecord = sandboxRecord record :=
                  Option.some.inj (heq.symm.trans modelLookup)
                subst modelRecord
                rw [dif_pos auth, dif_pos capability]
            rw [modelEvaluates]
            exact ⟨rfl, represented⟩
        · have policyExact := effectivePolicyExact_of_authorityShape model
            authorityShape record sid modelLookup
          have concreteAuth : SandboxModel.authorized (actor actorValue)
              (tenantId record.owner) := by
            simpa [sandboxRecord] using auth
          refine ⟨.Err .CapabilityDenied, ?_, ?_⟩
          · simp [SandboxControllerRust.auxiliary.handle_ingress, getEq,
              concreteLookup, generated_authorized_eq, concreteAuth,
              generated_allowsIngress_eq record sid vm slot policyExact,
              capability]
          · have modelEvaluates :
                SandboxModel.Controller.handleIngress model (actor actorValue)
                    (sandboxId sid) (vmId vm) (netSlotId slot) =
                  .error .capabilityDenied := by
                unfold SandboxModel.Controller.handleIngress
                split
                · rename_i heq
                  have impossible : (none : Option SandboxRecord) =
                      some (sandboxRecord record) := heq.symm.trans modelLookup
                  cases impossible
                · rename_i modelRecord heq
                  have same : modelRecord = sandboxRecord record :=
                    Option.some.inj (heq.symm.trans modelLookup)
                  subst modelRecord
                  rw [dif_pos auth, dif_neg capability]
            rw [modelEvaluates]
            rfl
      · have concreteAuth : ¬ SandboxModel.authorized (actor actorValue)
            (tenantId record.owner) := by
          simpa [sandboxRecord] using auth
        refine ⟨.Err .Unauthorized, ?_, ?_⟩
        · simp [SandboxControllerRust.auxiliary.handle_ingress, getEq,
            concreteLookup, generated_authorized_eq, concreteAuth]
        · have modelEvaluates :
              SandboxModel.Controller.handleIngress model (actor actorValue)
                  (sandboxId sid) (vmId vm) (netSlotId slot) =
                .error .unauthorized := by
              unfold SandboxModel.Controller.handleIngress
              split
              · rename_i heq
                have impossible : (none : Option SandboxRecord) =
                    some (sandboxRecord record) := heq.symm.trans modelLookup
                cases impossible
              · rename_i modelRecord heq
                have same : modelRecord = sandboxRecord record :=
                  Option.some.inj (heq.symm.trans modelLookup)
                subst modelRecord
                rw [dif_neg auth]
          rw [modelEvaluates]
          rfl

private theorem model_egress_missing {vm : VmId}
    (missing : model.sandboxes sid = none) :
    SandboxModel.Controller.handleEgress model sid vm slot destination =
      .error .sandboxNotFound := by
  unfold SandboxModel.Controller.handleEgress
  split <;> simp_all

private theorem model_egress_capabilityDenied {vm : VmId}
    (found : model.sandboxes sid = some record)
    (denied : ¬ record.effective.allows
      (.egress sid record.generation vm slot record.policy)) :
    SandboxModel.Controller.handleEgress model sid vm slot destination =
      .error .capabilityDenied := by
  unfold SandboxModel.Controller.handleEgress
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    rw [dif_neg denied]

private theorem model_egress_destinationDenied {vm : VmId}
    (found : model.sandboxes sid = some record)
    (allowed : record.effective.allows
      (.egress sid record.generation vm slot record.policy))
    (denied : ¬ ∃ endpoints, record.policy = .staticAllowlist endpoints ∧
      destination ∈ endpoints) :
    SandboxModel.Controller.handleEgress model sid vm slot destination =
      .error .destinationDenied := by
  unfold SandboxModel.Controller.handleEgress
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    rw [dif_pos allowed]
    split
    · rfl
    · rename_i endpoints policyCase
      have absent : destination ∉ endpoints := fun member =>
        denied ⟨endpoints, policyCase, member⟩
      rw [dif_neg absent]

private theorem model_egress_allowed {vm : VmId}
    (found : model.sandboxes sid = some record)
    (allowed : record.effective.allows
      (.egress sid record.generation vm slot record.policy))
    (policyCase : record.policy = .staticAllowlist endpoints)
    (member : destination ∈ endpoints) :
    ∃ decision, SandboxModel.Controller.handleEgress model sid vm slot destination =
      .ok decision ∧ decision.event = .egress sid destination ∧ decision.next = model := by
  let decision : SandboxModel.Controller.Decision model :=
    ⟨.egress sid destination, model, Step.egress found allowed ⟨policyCase, member⟩⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  unfold SandboxModel.Controller.handleEgress
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    rw [dif_pos allowed]
    split
    · simp_all
    · rename_i selectedEndpoints selectedPolicy
      have same := NetworkPolicy.staticAllowlist.inj (selectedPolicy.symm.trans policyCase)
      subst selectedEndpoints
      rw [dif_pos member]

theorem handleEgress_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId)
    (vm : SandboxControllerRust.ids.VmId)
    (slot : SandboxControllerRust.ids.NetSlotId)
    (destination : SandboxControllerRust.model.Endpoint) :
    ∃ rustResult,
      SandboxControllerRust.auxiliary.handle_egress rust sid vm slot destination = ok rustResult ∧
      AccessResultRep model rustResult
        (SandboxModel.Controller.handleEgress model (sandboxId sid)
          (vmId vm) (netSlotId slot) (endpoint destination)) := by
  have getEq := generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  cases concreteLookup : storeLookup rust.sandboxes sid with
  | none =>
      have modelLookup : model.sandboxes (sandboxId sid) = none := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) <;> simp_all [OptionRel]
      refine ⟨.Err .SandboxNotFound, ?_, ?_⟩
      · simp [SandboxControllerRust.auxiliary.handle_egress, getEq, concreteLookup]
      · rw [model_egress_missing modelLookup]
        rfl
  | some record =>
      have modelLookup : model.sandboxes (sandboxId sid) = some (sandboxRecord record) := by
        have agrees := sandboxLookup_cases rust model represented sid
        rw [concreteLookup] at agrees
        cases abstractLookup : model.sandboxes (sandboxId sid) with
        | none => simp [OptionRel, abstractLookup] at agrees
        | some abstractRecord =>
            rw [abstractLookup] at agrees
            exact congrArg some agrees.symm
      have capabilityEq := generated_allowsEgress_eq record sid vm slot
      by_cases capability : (sandboxRecord record).effective.allows
          (.egress (sandboxId sid) (sandboxRecord record).generation
            (vmId vm) (netSlotId slot) (sandboxRecord record).policy)
      · cases policyCase : record.policy with
        | DenyAll =>
            refine ⟨.Err .DestinationDenied, ?_, ?_⟩
            · simp [SandboxControllerRust.auxiliary.handle_egress, getEq, concreteLookup,
                capabilityEq, capability, generated_destinationAllowed_deny record destination policyCase]
            · rw [model_egress_destinationDenied modelLookup capability
                (by simp [sandboxRecord, policyCase, networkPolicy])]
              rfl
        | StaticAllowlist endpoints =>
            have destinationEq := generated_destinationAllowed_static record destination endpoints policyCase
            have mappedPolicy : (sandboxRecord record).policy =
                .staticAllowlist (endpoints.val.map endpoint) := by
              simp [sandboxRecord, policyCase, networkPolicy]
            have memberIff : endpoint destination ∈ endpoints.val.map endpoint ↔
                destination ∈ endpoints.val := by
              simp only [List.mem_map]
              constructor
              · rintro ⟨value, member, same⟩
                exact endpoint_injective same ▸ member
              · intro member
                exact ⟨destination, member, rfl⟩
            by_cases member : destination ∈ endpoints.val
            · let decision : SandboxControllerRust.auxiliary.AuxiliaryDecision :=
                { event := .Egress sid destination, next := rust }
              refine ⟨.Ok decision, ?_, ?_⟩
              · simp [SandboxControllerRust.auxiliary.handle_egress, getEq, concreteLookup,
                  capabilityEq, capability, destinationEq, member, world_clone_eq, decision]
              · obtain ⟨abstractDecision, evaluated, eventSame, worldSame⟩ :=
                  model_egress_allowed modelLookup capability mappedPolicy (memberIff.mpr member)
                rw [evaluated]
                change auxiliaryEvent decision.event = abstractDecision.event ∧
                  AuxiliaryWorldRep decision.next abstractDecision.next
                rw [worldSame, eventSame]
                exact ⟨rfl, represented⟩
            · refine ⟨.Err .DestinationDenied, ?_, ?_⟩
              · simp [SandboxControllerRust.auxiliary.handle_egress, getEq, concreteLookup,
                  capabilityEq, capability, destinationEq, member]
              · have denied : ¬ ∃ targets,
                    (sandboxRecord record).policy = .staticAllowlist targets ∧
                    endpoint destination ∈ targets := by
                  rintro ⟨targets, policySame, present⟩
                  have same := NetworkPolicy.staticAllowlist.inj (mappedPolicy.symm.trans policySame)
                  subst targets
                  exact member (memberIff.mp present)
                rw [model_egress_destinationDenied modelLookup capability denied]
                rfl
      · refine ⟨.Err .CapabilityDenied, ?_, ?_⟩
        · simp [SandboxControllerRust.auxiliary.handle_egress, getEq, concreteLookup,
            capabilityEq, capability]
        · rw [model_egress_capabilityDenied modelLookup capability]
          rfl

/-- All five access entry points, including every rejection and the exact
    unchanged successor state.  Authority shape is a proved safety invariant. -/
theorem handleAccess_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (authorityShape : AuthorityShape model)
    (input : SandboxControllerRust.auxiliary.AccessInput) :
    ∃ rustResult,
      SandboxControllerRust.auxiliary.handle_access rust input = ok rustResult ∧
      AccessResultRep model rustResult
        (SandboxModel.Controller.handleAccess model (accessInput input)) := by
  cases input with
  | Exec actorValue sid vm =>
      exact handleExec_refines rust model represented actorValue sid vm
  | FileRead actorValue sid generation =>
      exact handleFileRead_refines rust model represented actorValue sid generation
  | FileWrite actorValue sid generation =>
      exact handleFileWrite_refines rust model represented actorValue sid generation
  | Ingress actorValue sid vm slot =>
      exact handleIngress_refines rust model represented authorityShape actorValue sid vm slot
  | Egress sid vm slot destination =>
      exact handleEgress_refines rust model represented sid vm slot destination

end

end SandboxModel.RustRefinement
