import SandboxModel.RustRefinement.Create
import SandboxModel.RustRefinement.Clone

/-!
Complete result correspondence for the request admission layer generated from
`controller-core/src/request.rs`.

The read-only replay/query paths are proved below without any capacity
assumption.  Mutating paths use the same result relation; their transition
lemmas make the finite-machine headroom boundary explicit.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

noncomputable section

local instance requestHandlerOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance requestHandlerLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance requestHandlerEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq
local instance requestHandlerEndpointDecEq : DecidableEq SandboxControllerRust.model.Endpoint :=
  Classical.decEq _
local instance requestHandlerNetworkPolicyDecEq : DecidableEq SandboxControllerRust.model.NetworkPolicy :=
  Classical.decEq _
local instance requestHandlerRestoreSourceDecEq : DecidableEq SandboxControllerRust.model.RestoreSource :=
  Classical.decEq _
local instance requestHandlerOperationInputDecEq : DecidableEq SandboxControllerRust.model.OperationInput :=
  Classical.decEq _

abbrev RequestWorldRep :=
  @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
    effectIdDecidableEq valueRelations

def requestInput : SandboxControllerRust.request.Request →
    SandboxModel.Controller.Request
  | .Create input => .create (actor input.actor) (tenantId input.owner)
      (sandboxId input.sandbox) (fsId input.workspace)
      (filePolicyRef input.file_policy) (networkPolicy input.policy)
      (operationId input.operation)
  | .Pause input => .pause (actor input.actor) (sandboxId input.sandbox)
      (operationId input.operation)
  | .Resume input => .resume (actor input.actor) (sandboxId input.sandbox)
      (operationId input.operation)
  | .Kill input => .kill (actor input.actor) (sandboxId input.sandbox)
      (operationId input.operation)
  | .Replay input => .replay (actor input.actor) (sandboxId input.sandbox)
      (operationId input.operation) (operationKind input.kind)
      (operationInput input.input)
  | .Query input => .query (actor input.actor) (sandboxId input.sandbox)
      (operationId input.operation)
  | .Compensate input => .compensate (sandboxId input.sandbox)
      (failureKind input.failure) (input.evidence.map effectId)

def RequestResultRep (model : World)
    (rustResult : core.result.Result SandboxControllerRust.request.Decision
      SandboxControllerRust.request.Reject)
    (modelResult : Except SandboxModel.Controller.Reject
      (SandboxModel.Controller.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err rustReject, .error modelReject => reject? rustReject = some modelReject
  | .Ok rustDecision, .ok modelDecision =>
      modelEvent rustDecision.event = modelDecision.event ∧
        RequestWorldRep rustDecision.next modelDecision.next
  | _, _ => False

private theorem generated_get_eq [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K) :
    SandboxControllerRust.store.Store.get partialEq store key =
      ok (storeLookup store key) := by
  obtain ⟨value, evaluated, same⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_get_refines partialEq lawful store key)
  subst value
  exact evaluated

private theorem sandboxLookup_agrees
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : RequestWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) :
    OptionRel (fun concrete abstract => sandboxRecord concrete = abstract)
      (storeLookup rust.sandboxes sid) (model.sandboxes (sandboxId sid)) :=
  represented.sandboxes.lookupAgrees sid

private theorem operationLookup_agrees
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : RequestWorldRep rust model)
    (key : SandboxControllerRust.model.OperationKey) :
    OptionRel (fun concrete abstract => operationLedger concrete = abstract)
      (storeLookup rust.operations key) (model.operations (operationKey key)) :=
  represented.operations.lookupAgrees key

@[simp] private theorem tenantId_eq_iff {left right} :
    tenantId left = tenantId right ↔ left = right :=
  tenantId_injective.eq_iff

theorem generated_authorized_eq
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

private theorem operationKind_ne_eq
    (left right : SandboxControllerRust.model.OperationKind) :
    core.cmp.PartialEq.ne.trait_default
        SandboxControllerRust.model.OperationKind.Insts.CoreCmpPartialEqOperationKind
        left right =
      ok (decide (operationKind left ≠ operationKind right)) := by
  cases left <;> cases right <;>
    simp [core.cmp.PartialEq.ne.trait_default,
      core.cmp.PartialEq.ne.default,
      SandboxControllerRust.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
      SandboxControllerRust.model.OperationKind.read_discriminant, operationKind]

private theorem operationInput_injective : Function.Injective operationInput := by
  intro left right same
  cases left <;> cases right <;> simp_all [operationInput,
    networkPolicy_eq_iff, restoreSource_eq_iff, filePolicyRef_eq_iff,
    u64ToNat_eq_iff]

private theorem allM_zip_eq_of_lawful [DecidableEq α]
    (partialEq : core.cmp.PartialEq α α)
    (lawful : LawfulPartialEq partialEq) :
    ∀ left right : List α, left.length = right.length →
      List.allM (fun pair : α × α => partialEq.eq pair.1 pair.2)
          (List.zip left right) = ok (decide (left = right)) := by
  intro left
  induction left with
  | nil =>
      intro right sameLength
      cases right with
      | nil => rfl
      | cons _ _ => simp at sameLength
  | cons head tail ih =>
      intro right sameLength
      cases right with
      | nil => simp at sameLength
      | cons other rest =>
          have tailLength : tail.length = rest.length := by simpa using sameLength
          simp only [List.zip_cons_cons, List.allM_cons]
          rw [lawful head other]
          by_cases sameHead : head = other
          · subst other
            simpa [Aeneas.Std.instMonadResult] using ih rest tailLength
          · simp [sameHead]
            rfl

private theorem vecPartialEq_lawful [DecidableEq α]
    (partialEq : core.cmp.PartialEq α α)
    (lawful : LawfulPartialEq partialEq)
    (left right : alloc.vec.Vec α) :
    alloc.vec.partial_eq.PartialEqVec.eq partialEq left right =
      ok (decide (left = right)) := by
  unfold alloc.vec.partial_eq.PartialEqVec.eq
  by_cases sameLength : left.length = right.length
  · simp only [sameLength, if_pos]
    rw [allM_zip_eq_of_lawful partialEq lawful left.val right.val
      (by simpa using sameLength)]
    congr 2
    apply propext
    exact ⟨fun same => alloc.vec.Vec.ext left right same,
      fun same => congrArg alloc.vec.Vec.val same⟩
  · have different : left ≠ right := by
      intro same
      exact sameLength (congrArg alloc.vec.Vec.length same)
    simp [sameLength, different]

private theorem endpointPartialEq_lawful :
    LawfulPartialEq
      SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint := by
  classical
  intro left right
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
                  simp [alloc.string.String.Insts.CoreCmpPartialEqString.eq,
                    SandboxControllerRust.model.Protocol.Insts.CoreCmpPartialEqProtocol.eq,
                    SandboxControllerRust.model.Protocol.read_discriminant]
              · simp [sameAddress,
                  alloc.string.String.Insts.CoreCmpPartialEqString.eq]
            · simp [samePort]
          · simp [samePrefix]

private theorem networkPolicyPartialEq_lawful
    (left right : SandboxControllerRust.model.NetworkPolicy) :
    SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq
        left right = ok (decide (left = right)) := by
  classical
  cases left with
  | DenyAll =>
      cases right <;>
        simp [SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
          SandboxControllerRust.model.NetworkPolicy.read_discriminant]
  | StaticAllowlist leftEndpoints =>
      cases right with
      | DenyAll =>
          simp [SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
            SandboxControllerRust.model.NetworkPolicy.read_discriminant]
      | StaticAllowlist rightEndpoints =>
          simp only [
            SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
            SandboxControllerRust.model.NetworkPolicy.read_discriminant,
            ↓reduceIte]
          rw [vecPartialEq_lawful
            SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint
            endpointPartialEq_lawful leftEndpoints rightEndpoints]
          congr 2
          apply propext
          simp

private theorem restoreSourcePartialEq_lawful
    (left right : SandboxControllerRust.model.RestoreSource) :
    SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq
        left right = ok (decide (left = right)) := by
  classical
  cases left with
  | mk leftSnapshot leftManifest =>
      cases right with
      | mk rightSnapshot rightManifest =>
          cases leftManifest with
          | mk leftDigest =>
              cases rightManifest with
              | mk rightDigest =>
                  simp only [
                    SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq,
                    SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq,
                    SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
                    SandboxControllerRust.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq]
                  by_cases sameSnapshot : leftSnapshot = rightSnapshot <;>
                  by_cases sameDigest : leftDigest = rightDigest <;>
                    simp [sameSnapshot, sameDigest]

private theorem operationInputPartialEq_lawful
    (left right : SandboxControllerRust.model.OperationInput) :
    SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq
        left right = ok (decide (left = right)) := by
  classical
  cases left with
  | Create leftWorkspace leftPolicy leftFilePolicy =>
      cases right with
      | Create rightWorkspace rightPolicy rightFilePolicy =>
          cases leftFilePolicy with
          | mk leftDigest =>
              cases rightFilePolicy with
              | mk rightDigest =>
                  simp only [
                    SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq,
                    SandboxControllerRust.model.OperationInput.read_discriminant,
                    ↓reduceIte,
                    SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq]
                  rw [networkPolicyPartialEq_lawful]
                  by_cases sameWorkspace : leftWorkspace = rightWorkspace <;>
                  by_cases samePolicy : leftPolicy = rightPolicy <;>
                  by_cases sameDigest : leftDigest = rightDigest <;>
                    simp [sameWorkspace, samePolicy, sameDigest,
                      SandboxControllerRust.model.FilePolicyRef.Insts.CoreCmpPartialEqFilePolicyRef.eq,
                      SandboxControllerRust.ids.PolicyDigest.Insts.CoreCmpPartialEqPolicyDigest.eq]
      | Pause _ | Resume _ | Kill _ =>
          simp [SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq,
            SandboxControllerRust.model.OperationInput.read_discriminant]
  | Pause leftGeneration =>
      cases right with
      | Create _ _ _ | Resume _ | Kill _ =>
          simp [SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq,
            SandboxControllerRust.model.OperationInput.read_discriminant]
      | Pause rightGeneration =>
          simp [SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq,
            SandboxControllerRust.model.OperationInput.read_discriminant, Aeneas.Std.lift]
  | Resume leftSource =>
      cases right with
      | Create _ _ _ | Pause _ | Kill _ =>
          simp [SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq,
            SandboxControllerRust.model.OperationInput.read_discriminant]
      | Resume rightSource =>
          simpa [
            SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq,
            SandboxControllerRust.model.OperationInput.read_discriminant] using
              restoreSourcePartialEq_lawful leftSource rightSource
  | Kill leftGeneration =>
      cases right with
      | Create _ _ _ | Pause _ | Resume _ =>
          simp [SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq,
            SandboxControllerRust.model.OperationInput.read_discriminant]
      | Kill rightGeneration =>
          simp [SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput.eq,
            SandboxControllerRust.model.OperationInput.read_discriminant, Aeneas.Std.lift]

private theorem operationInput_ne_eq
    (left right : SandboxControllerRust.model.OperationInput) :
    core.cmp.PartialEq.ne.trait_default
        SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput
        left right =
      ok (decide (operationInput left ≠ operationInput right)) := by
  classical
  have concrete :
      core.cmp.PartialEq.ne.trait_default
          SandboxControllerRust.model.OperationInput.Insts.CoreCmpPartialEqOperationInput
          left right = ok (decide (left ≠ right)) := by
    unfold core.cmp.PartialEq.ne.trait_default core.cmp.PartialEq.ne.default
    simp_rw [operationInputPartialEq_lawful]
    by_cases same : left = right <;> simp [same]
  rw [concrete]
  congr 2
  apply propext
  exact not_congr operationInput_injective.eq_iff.symm

theorem handleReplay_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : RequestWorldRep rust model)
    (input : SandboxControllerRust.request.ReplayRequest) :
    ∃ rustResult,
      SandboxControllerRust.request.handle_replay rust input = ok rustResult ∧
      RequestResultRep model rustResult
        (SandboxModel.Controller.handleReplay model (actor input.actor)
          (sandboxId input.sandbox) (operationId input.operation)
          (operationKind input.kind) (operationInput input.input)) := by
  have sandboxGet := generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes input.sandbox
  cases concreteSandbox : storeLookup rust.sandboxes input.sandbox with
  | none =>
      have agrees := sandboxLookup_agrees represented input.sandbox
      rw [concreteSandbox] at agrees
      cases modelSandbox : model.sandboxes (sandboxId input.sandbox) with
      | none =>
          have rustEval : SandboxControllerRust.request.handle_replay rust input =
              ok (.Err .SandboxNotFound) := by
            unfold SandboxControllerRust.request.handle_replay
            rw [sandboxGet, concreteSandbox]
            simp [Aeneas.Std.instMonadResult]
          have modelEval : SandboxModel.Controller.handleReplay model
              (actor input.actor) (sandboxId input.sandbox)
              (operationId input.operation) (operationKind input.kind)
              (operationInput input.input) = .error .sandboxNotFound := by
            unfold SandboxModel.Controller.handleReplay
            split <;> simp_all
            all_goals (try split) <;> try simp_all
            all_goals (try split) <;> try simp_all
            all_goals (try split) <;> try simp_all
          refine ⟨.Err .SandboxNotFound, rustEval, ?_⟩
          rw [modelEval]
          rfl
      | some value => simp [OptionRel, modelSandbox] at agrees
  | some concreteRecord =>
      have agrees := sandboxLookup_agrees represented input.sandbox
      rw [concreteSandbox] at agrees
      cases modelSandbox : model.sandboxes (sandboxId input.sandbox) with
      | none => simp [OptionRel, modelSandbox] at agrees
      | some modelRecord =>
          have recordSame : sandboxRecord concreteRecord = modelRecord := by
            simpa [OptionRel, modelSandbox] using agrees
          subst modelRecord
          have authEq := generated_authorized_eq input.actor concreteRecord.owner
          by_cases auth : SandboxModel.authorized (actor input.actor)
              (tenantId concreteRecord.owner)
          · have modelAuth : SandboxModel.authorized (actor input.actor)
                (sandboxRecord concreteRecord).owner := by
              simpa [sandboxRecord] using auth
            let key : SandboxControllerRust.model.OperationKey :=
              { sandbox := input.sandbox, operation := input.operation }
            have operationGet := generated_get_eq
              SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
              operationKey_partialEq_lawful rust.operations
                ({ sandbox := input.sandbox, operation := input.operation } :
                  SandboxControllerRust.model.OperationKey)
            cases concreteOperation : storeLookup rust.operations key with
            | none =>
                have operationAgrees := operationLookup_agrees represented key
                rw [concreteOperation] at operationAgrees
                cases modelOperation : model.operations (operationKey key) with
                | none =>
                    have modelOperationMissing :
                        model.operations (SandboxModel.operationKey
                          (sandboxId input.sandbox) (operationId input.operation)) =
                          none := by
                      simpa [key, operationKey, SandboxModel.operationKey] using
                        modelOperation
                    have rustEval : SandboxControllerRust.request.handle_replay rust input =
                        ok (.Err .OperationUnavailable) := by
                      unfold SandboxControllerRust.request.handle_replay
                      rw [sandboxGet, concreteSandbox]
                      simp [Aeneas.Std.instMonadResult]
                      rw [authEq]
                      simp [Aeneas.Std.instMonadResult, auth,
                        SandboxControllerRust.transition.operation_key]
                      rw [operationGet, concreteOperation]
                      simp [Aeneas.Std.instMonadResult]
                    have modelEval : SandboxModel.Controller.handleReplay model
                        (actor input.actor) (sandboxId input.sandbox)
                        (operationId input.operation) (operationKind input.kind)
                        (operationInput input.input) = .error .operationUnavailable := by
                      unfold SandboxModel.Controller.handleReplay
                      split <;> simp_all [key, modelOperationMissing]
                      all_goals (try split) <;>
                        try simp_all [key, modelOperationMissing]
                    refine ⟨.Err .OperationUnavailable, rustEval, ?_⟩
                    rw [modelEval]
                    rfl
                | some value => simp [OptionRel, modelOperation] at operationAgrees
            | some concreteLedger =>
                have operationAgrees := operationLookup_agrees represented key
                rw [concreteOperation] at operationAgrees
                cases modelOperation : model.operations (operationKey key) with
                | none => simp [OptionRel, modelOperation] at operationAgrees
                | some modelLedger =>
                    have ledgerSame : operationLedger concreteLedger = modelLedger := by
                      simpa [OptionRel, modelOperation] using operationAgrees
                    subst modelLedger
                    have modelOperationFound :
                        model.operations (SandboxModel.operationKey
                          (sandboxId input.sandbox) (operationId input.operation)) =
                          some (operationLedger concreteLedger) := by
                      simpa [key, operationKey, SandboxModel.operationKey] using
                        modelOperation
                    have kindEq := operationKind_ne_eq
                      concreteLedger.operation.kind input.kind
                    by_cases sameKind : operationKind concreteLedger.operation.kind =
                        operationKind input.kind
                    · have inputEq := operationInput_ne_eq concreteLedger.input input.input
                      by_cases sameInput : operationInput concreteLedger.input =
                          operationInput input.input
                      ·
                        let decision : SandboxControllerRust.request.Decision := {
                          event := .RetryOperation input.actor input.sandbox
                            input.operation input.kind input.input
                          next := rust }
                        have rustEval : SandboxControllerRust.request.handle_replay rust input =
                            ok (.Ok decision) := by
                          unfold SandboxControllerRust.request.handle_replay
                          rw [sandboxGet, concreteSandbox]
                          simp [Aeneas.Std.instMonadResult]
                          rw [authEq]
                          simp [Aeneas.Std.instMonadResult, auth,
                            SandboxControllerRust.transition.operation_key]
                          rw [operationGet, concreteOperation]
                          simp [Aeneas.Std.instMonadResult]
                          rw [kindEq]
                          simp [Aeneas.Std.instMonadResult, sameKind]
                          rw [inputEq]
                          simp [Aeneas.Std.instMonadResult, sameInput]
                          rw [world_clone_eq]
                          simp [Aeneas.Std.instMonadResult, decision]
                        have modelEval : SandboxModel.Controller.handleReplay model
                            (actor input.actor) (sandboxId input.sandbox)
                            (operationId input.operation) (operationKind input.kind)
                            (operationInput input.input) = .ok
                              { event := .retryOperation (actor input.actor)
                                  (sandboxId input.sandbox) (operationId input.operation)
                                  (operationKind input.kind) (operationInput input.input)
                                next := model
                                legal := by
                                  exact Step.retryOperation modelSandbox modelAuth
                                    modelOperationFound
                                    (by simpa [operationLedger, operation] using sameKind)
                                    (by simpa [operationLedger] using sameInput) } := by
                          unfold SandboxModel.Controller.handleReplay
                          split
                          · rename_i missing
                            simp_all
                          · rename_i record found
                            have same : record = sandboxRecord concreteRecord :=
                              Option.some.inj (found.symm.trans modelSandbox)
                            subst record
                            rw [dif_pos modelAuth]
                            split
                            · rename_i missing
                              simp_all
                            · rename_i ledger found
                              have same : ledger = operationLedger concreteLedger :=
                                Option.some.inj (found.symm.trans modelOperationFound)
                              subst ledger
                              rw [dif_pos (show (operationLedger concreteLedger).operation.kind =
                                operationKind input.kind from sameKind)]
                              rw [dif_pos (show (operationLedger concreteLedger).input =
                                operationInput input.input from sameInput)]
                        refine ⟨.Ok decision, rustEval, ?_⟩
                        rw [modelEval]
                        simp only [RequestResultRep]
                        constructor
                        · simp [decision, modelEvent]
                        · exact represented
                      · have rustEval : SandboxControllerRust.request.handle_replay rust input =
                            ok (.Err .LedgerMismatch) := by
                          unfold SandboxControllerRust.request.handle_replay
                          rw [sandboxGet, concreteSandbox]
                          simp [Aeneas.Std.instMonadResult]
                          rw [authEq]
                          simp [Aeneas.Std.instMonadResult, auth,
                            SandboxControllerRust.transition.operation_key]
                          rw [operationGet, concreteOperation]
                          simp [Aeneas.Std.instMonadResult]
                          rw [kindEq]
                          simp [Aeneas.Std.instMonadResult, sameKind]
                          rw [inputEq]
                          simp [Aeneas.Std.instMonadResult, sameInput]
                        have modelEval : SandboxModel.Controller.handleReplay model
                            (actor input.actor) (sandboxId input.sandbox)
                            (operationId input.operation) (operationKind input.kind)
                            (operationInput input.input) = .error .ledgerMismatch := by
                          unfold SandboxModel.Controller.handleReplay
                          split <;> simp_all [operationLedger, operation, key,
                            modelOperationFound]
                          all_goals (try split) <;>
                            try simp_all [operationLedger, operation, key,
                              modelOperationFound]
                          all_goals (try split) <;>
                            try simp_all [operationLedger, operation, key,
                              modelOperationFound]
                        refine ⟨.Err .LedgerMismatch, rustEval, ?_⟩
                        rw [modelEval]
                        rfl
                    · have rustEval : SandboxControllerRust.request.handle_replay rust input =
                          ok (.Err .LedgerMismatch) := by
                        unfold SandboxControllerRust.request.handle_replay
                        rw [sandboxGet, concreteSandbox]
                        simp [Aeneas.Std.instMonadResult]
                        rw [authEq]
                        simp [Aeneas.Std.instMonadResult, auth,
                          SandboxControllerRust.transition.operation_key]
                        rw [operationGet, concreteOperation]
                        simp [Aeneas.Std.instMonadResult]
                        rw [kindEq]
                        simp [Aeneas.Std.instMonadResult, sameKind]
                      have modelEval : SandboxModel.Controller.handleReplay model
                          (actor input.actor) (sandboxId input.sandbox)
                          (operationId input.operation) (operationKind input.kind)
                          (operationInput input.input) = .error .ledgerMismatch := by
                        unfold SandboxModel.Controller.handleReplay
                        split <;> simp_all [operationLedger, operation, key,
                          modelOperationFound]
                        all_goals (try split) <;>
                          try simp_all [operationLedger, operation, key,
                            modelOperationFound]
                      refine ⟨.Err .LedgerMismatch, rustEval, ?_⟩
                      rw [modelEval]
                      rfl
          · have modelNoAuth : ¬ SandboxModel.authorized (actor input.actor)
                (sandboxRecord concreteRecord).owner := by
              simpa [sandboxRecord] using auth
            have rustEval : SandboxControllerRust.request.handle_replay rust input =
                ok (.Err .Unauthorized) := by
              unfold SandboxControllerRust.request.handle_replay
              rw [sandboxGet, concreteSandbox]
              simp [Aeneas.Std.instMonadResult]
              rw [authEq]
              simp [Aeneas.Std.instMonadResult, auth]
            have modelEval : SandboxModel.Controller.handleReplay model
                (actor input.actor) (sandboxId input.sandbox)
                (operationId input.operation) (operationKind input.kind)
                (operationInput input.input) = .error .unauthorized := by
              unfold SandboxModel.Controller.handleReplay
              split <;> simp_all
              all_goals (try split) <;> try simp_all
            refine ⟨.Err .Unauthorized, rustEval, ?_⟩
            rw [modelEval]
            rfl

theorem handleQuery_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : RequestWorldRep rust model)
    (input : SandboxControllerRust.request.QueryRequest) :
    ∃ rustResult,
      SandboxControllerRust.request.handle_query rust input = ok rustResult ∧
      RequestResultRep model rustResult
        (SandboxModel.Controller.handleQuery model (actor input.actor)
          (sandboxId input.sandbox) (operationId input.operation)) := by
  unfold SandboxControllerRust.request.handle_query
  rw [generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful]
  cases concreteSandbox : storeLookup rust.sandboxes input.sandbox with
  | none =>
      have agrees := sandboxLookup_agrees represented input.sandbox
      rw [concreteSandbox] at agrees
      cases modelSandbox : model.sandboxes (sandboxId input.sandbox) with
      | none =>
          refine ⟨.Err .SandboxNotFound, by simp [concreteSandbox], ?_⟩
          have modelEval : SandboxModel.Controller.handleQuery model
              (actor input.actor) (sandboxId input.sandbox)
              (operationId input.operation) = .error .sandboxNotFound := by
            unfold SandboxModel.Controller.handleQuery
            split <;> simp_all
          rw [modelEval]
          rfl
      | some value => simp [OptionRel, modelSandbox] at agrees
  | some concreteRecord =>
      have agrees := sandboxLookup_agrees represented input.sandbox
      rw [concreteSandbox] at agrees
      cases modelSandbox : model.sandboxes (sandboxId input.sandbox) with
      | none => simp [OptionRel, modelSandbox] at agrees
      | some modelRecord =>
          have recordSame : sandboxRecord concreteRecord = modelRecord := by
            simpa [OptionRel, modelSandbox] using agrees
          simp [Aeneas.Std.instMonadResult]
          rw [generated_authorized_eq]
          by_cases auth : SandboxModel.authorized (actor input.actor)
              (tenantId concreteRecord.owner)
          · have modelAuth : SandboxModel.authorized (actor input.actor)
                modelRecord.owner := by
              simpa [← recordSame, sandboxRecord] using auth
            simp [auth, SandboxControllerRust.transition.operation_key]
            rw [generated_get_eq
              SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
              operationKey_partialEq_lawful]
            let key : SandboxControllerRust.model.OperationKey :=
              { sandbox := input.sandbox, operation := input.operation }
            cases concreteOperation : storeLookup rust.operations key with
            | none =>
                have operationAgrees := operationLookup_agrees represented key
                rw [concreteOperation] at operationAgrees
                cases modelOperation : model.operations (operationKey key) with
                | none =>
                    refine ⟨.Err .OperationUnavailable, by simp [key, concreteOperation], ?_⟩
                    have modelLookup : model.operations (SandboxModel.operationKey
                        (sandboxId input.sandbox) (operationId input.operation)) = none := by
                      simpa [key, operationKey, SandboxModel.operationKey] using modelOperation
                    have modelEval : SandboxModel.Controller.handleQuery model
                        (actor input.actor) (sandboxId input.sandbox)
                        (operationId input.operation) = .error .operationUnavailable := by
                      unfold SandboxModel.Controller.handleQuery
                      split <;> simp_all
                      all_goals (try split) <;> try simp_all
                    rw [modelEval]
                    rfl
                | some value => simp [OptionRel, modelOperation] at operationAgrees
            | some concreteLedger =>
                have operationAgrees := operationLookup_agrees represented key
                rw [concreteOperation] at operationAgrees
                cases modelOperation : model.operations (operationKey key) with
                | none => simp [OptionRel, modelOperation] at operationAgrees
                | some modelLedger =>
                    have ledgerSame : operationLedger concreteLedger = modelLedger := by
                      simpa [OptionRel, modelOperation] using operationAgrees
                    simp [Aeneas.Std.instMonadResult]
                    rw [world_clone_eq]
                    let decision : SandboxControllerRust.request.Decision := {
                      event := .QueryOperation input.actor input.sandbox
                        input.operation concreteLedger.operation.status
                        concreteLedger.result
                      next := rust }
                    refine ⟨.Ok decision, by simp [decision, key, concreteOperation], ?_⟩
                    have modelLookup : model.operations (SandboxModel.operationKey
                        (sandboxId input.sandbox) (operationId input.operation)) = some modelLedger := by
                      simpa [key, operationKey, SandboxModel.operationKey] using modelOperation
                    unfold SandboxModel.Controller.handleQuery
                    split
                    · rename_i missing
                      simp_all
                    · rename_i record found
                      have same : record = modelRecord := Option.some.inj (found.symm.trans modelSandbox)
                      subst record
                      rw [dif_pos modelAuth]
                      split
                      · rename_i missing
                        simp_all
                      · rename_i ledger found
                        have same : ledger = modelLedger := Option.some.inj (found.symm.trans modelLookup)
                        subst ledger
                        constructor
                        · simp [decision, modelEvent, ← ledgerSame, operationLedger, operation]
                        · exact represented
          · have modelNoAuth : ¬ SandboxModel.authorized (actor input.actor)
                modelRecord.owner := by
              simpa [← recordSame, sandboxRecord] using auth
            simp [auth]
            have modelEval : SandboxModel.Controller.handleQuery model
                (actor input.actor) (sandboxId input.sandbox)
                (operationId input.operation) = .error .unauthorized := by
              unfold SandboxModel.Controller.handleQuery
              split <;> simp_all
              all_goals (try split) <;> try simp_all
            rw [modelEval]
            rfl

end

end SandboxModel.RustRefinement
