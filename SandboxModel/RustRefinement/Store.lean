import SandboxControllerCore
import SandboxModel.State

/-!
This module is the first representation bridge between the Rust controller
translated by Aeneas and the mathematical state-machine model.  The Rust
implementation uses bounded `u64` identifiers and finite vectors of key/value
pairs; the specification uses unbounded natural-number identifiers and total
lookup functions.  Keeping the conversion explicit makes the bounded-machine
assumption visible in every later refinement theorem.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result ControlFlow Error

/-- Interpret a concrete Rust `u64` as the natural number used by the model. -/
def u64ToNat (value : Std.U64) : Nat := value.val

theorem u64ToNat_injective : Function.Injective u64ToNat := by
  intro left right same
  exact Std.UScalar.eq_of_val_eq same

@[simp] theorem u64ToNat_ofNat (value : Nat) (h : value < 2 ^ 64) :
    u64ToNat (Std.U64.ofNatCore value (by simpa using h)) = value := by
  simp [u64ToNat]

def sandboxId (id : SandboxControllerRust.ids.SandboxId) : SandboxId := ⟨u64ToNat id⟩
def tenantId (id : SandboxControllerRust.ids.TenantId) : TenantId := ⟨u64ToNat id⟩
def vmId (id : SandboxControllerRust.ids.VmId) : VmId := ⟨u64ToNat id⟩
def fsId (id : SandboxControllerRust.ids.FsId) : FsId := ⟨u64ToNat id⟩
def snapshotId (id : SandboxControllerRust.ids.SnapshotId) : SnapshotId := ⟨u64ToNat id⟩
def netSlotId (id : SandboxControllerRust.ids.NetSlotId) : NetSlotId := ⟨u64ToNat id⟩
def operationId (id : SandboxControllerRust.ids.OperationId) : OperationId := ⟨u64ToNat id⟩
def policyDigest (id : SandboxControllerRust.ids.PolicyDigest) : PolicyDigest := ⟨u64ToNat id⟩
def manifestDigest (id : SandboxControllerRust.ids.ManifestDigest) : ManifestDigest := ⟨u64ToNat id⟩
def resultDigest (id : SandboxControllerRust.ids.ResultDigest) : ResultDigest := ⟨u64ToNat id⟩

@[simp] theorem sandboxId_value (id : SandboxControllerRust.ids.SandboxId) :
    (sandboxId id).value = id.val := rfl

@[simp] theorem tenantId_value (id : SandboxControllerRust.ids.TenantId) :
    (tenantId id).value = id.val := rfl

theorem sandboxId_injective : Function.Injective sandboxId := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg SandboxId.value same

theorem fsId_injective : Function.Injective fsId := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg FsId.value same

theorem operationId_injective : Function.Injective operationId := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg OperationId.value same

/-- Basic enum translations are total and constructor preserving. -/
def actor : SandboxControllerRust.model.Actor → Actor
  | .Tenant id => .tenant (tenantId id)
  | .SystemController => .systemController

def phase : SandboxControllerRust.model.Phase → Phase
  | .Starting => .starting
  | .Running => .running
  | .Pausing => .pausing
  | .Paused => .paused
  | .Revoking => .revoking
  | .Destroyed => .destroyed

def vmStatus : SandboxControllerRust.model.VmStatus → VmStatus
  | .Prepared => .prepared
  | .Executing => .executing
  | .Frozen => .frozen
  | .Stopped => .stopped
  | .Reclaimed => .reclaimed

def snapshotStage : SandboxControllerRust.model.SnapshotStage → SnapshotStage
  | .LocalSealed => .localSealed
  | .Durable => .durable

def workspaceApiAccess : SandboxControllerRust.model.WorkspaceApiAccess → WorkspaceApiAccess
  | .ManagedReadWrite => .managedReadWrite

@[simp] theorem actor_tenant (id : SandboxControllerRust.ids.TenantId) :
    actor (.Tenant id) = .tenant (tenantId id) := rfl

@[simp] theorem actor_systemController :
    actor .SystemController = .systemController := rfl

/-- Pure first-match lookup used to denote the Rust association list.  It is
    written explicitly with `DecidableEq` so later proofs do not silently
    depend on a separately inferred `BEq` instance. -/
def associationLookup [DecidableEq K] (key : K) : List (K × V) → Option V
  | [] => none
  | (storedKey, value) :: rest =>
      if key = storedKey then some value else associationLookup key rest

/-- The finite Rust store represented as a first-match association-list lookup. -/
def storeLookup [DecidableEq K]
    (store : SandboxControllerRust.store.Store K V) (key : K) : Option V :=
  associationLookup key store.entries.val

/-- A successful first-match lookup identifies an actual stored entry. -/
theorem associationLookup_some_mem [DecidableEq K]
    {entries : List (K × V)} (found : associationLookup key entries = some value) :
    (key, value) ∈ entries := by
  induction entries with
  | nil => simp [associationLookup] at found
  | cons entry rest induction =>
      rcases entry with ⟨stored, existing⟩
      by_cases same : key = stored
      · simp [associationLookup, same] at found
        simp [same, found]
      · simp [associationLookup, same] at found
        exact List.mem_cons_of_mem _ (induction found)

theorem storeLookup_some_mem [DecidableEq K]
    {store : SandboxControllerRust.store.Store K V}
    (found : storeLookup store key = some value) :
    (key, value) ∈ store.entries.val := associationLookup_some_mem found

/-- Pointwise relation lifted to optional lookup results. -/
def OptionRel (valueRel : RustValue → ModelValue → Prop) :
    Option RustValue → Option ModelValue → Prop
  | none, none => True
  | some rust, some model => valueRel rust model
  | _, _ => False

/-- Representation invariant for Rust's finite association-list store.

`keyUnique` is needed because Rust updates the first matching entry whereas the
model's functional update has only one value per key.  `modelSupport` states
the bounded-machine boundary: every populated abstract key has a concrete Rust
representative.  `capacity` is the vector validity bound; mutating operations
add the strictly stronger `StoreHeadroom` premise when they may append. -/
structure StoreRep [DecidableEq RustKey]
    (keyMap : RustKey → ModelKey)
    (valueRel : RustValue → ModelValue → Prop)
    (rust : SandboxControllerRust.store.Store RustKey RustValue)
    (model : ModelKey → Option ModelValue) : Prop where
  keyUnique : (rust.entries.val.map Prod.fst).Nodup
  lookupAgrees : ∀ key,
    OptionRel valueRel (storeLookup rust key) (model (keyMap key))
  modelSupport : ∀ key value, model key = some value →
    ∃ rustKey, keyMap rustKey = key
  capacity : rust.entries.val.length ≤ Std.Usize.max

/-- A represented finite store and its abstract lookup agree on key presence,
    independently of the relation chosen for values. -/
theorem StoreRep.lookup_isSome_eq [DecidableEq RustKey]
    {keyMap : RustKey → ModelKey}
    {valueRel : RustValue → ModelValue → Prop}
    {rust : SandboxControllerRust.store.Store RustKey RustValue}
    {model : ModelKey → Option ModelValue}
    (represented : StoreRep keyMap valueRel rust model) (key : RustKey) :
    (storeLookup rust key).isSome = (model (keyMap key)).isSome := by
  have agrees := represented.lookupAgrees key
  generalize concrete : storeLookup rust key = concreteValue at agrees ⊢
  generalize abstract : model (keyMap key) = abstractValue at agrees ⊢
  cases concreteValue <;> cases abstractValue <;>
    simp_all [OptionRel]

/-- A counter can be incremented without leaving Rust's representable domain. -/
def U64Headroom (value : Std.U64) : Prop := value.val < Std.U64.max

/-- A store can append one more association without overflowing a Rust vector. -/
def StoreHeadroom (store : SandboxControllerRust.store.Store K V) : Prop :=
  store.entries.val.length < Std.Usize.max

/-- Semantic contract required of the Rust `PartialEq` implementation used by
    `Store`.  It rules out arbitrary generated trait dictionaries and ties the
    executable comparison to Lean equality. -/
def LawfulPartialEq [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) : Prop :=
  ∀ left right, partialEq.eq left right = ok (decide (left = right))

theorem sandboxId_partialEq_lawful :
    LawfulPartialEq
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId := by
  intro left right
  simp [SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]

theorem vmId_partialEq_lawful :
    LawfulPartialEq
      SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId := by
  intro left right
  simp [SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq]

theorem fsId_partialEq_lawful :
    LawfulPartialEq
      SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId := by
  intro left right
  simp [SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq]

theorem snapshotId_partialEq_lawful :
    LawfulPartialEq
      SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId := by
  intro left right
  simp [SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq]

theorem netSlotId_partialEq_lawful :
    LawfulPartialEq
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId := by
  intro left right
  simp [SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId.eq]

/-- The generated loop implements association-list lookup from any in-range
    starting index.  The decreasing measure is the number of entries that
    remain; consequently the theorem also proves termination of this path. -/
theorem generated_get_loop_refines [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K)
    (start : Std.Usize) (startBound : start.val ≤ store.entries.val.length) :
    SandboxControllerRust.store.Store.get_loop partialEq store key start
      ⦃ value =>
        value = associationLookup key (store.entries.val.drop start.val) ⦄ := by
  unfold SandboxControllerRust.store.Store.get_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index => store.entries.val.length - index.val)
    (fun index =>
      index.val ≤ store.entries.val.length ∧
      associationLookup key (store.entries.val.drop index.val) =
        associationLookup key (store.entries.val.drop start.val))
    (fun value =>
      value = associationLookup key (store.entries.val.drop start.val))
  · intro index invariant
    unfold SandboxControllerRust.store.Store.get_loop.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step
      have indexBound : index.val < store.entries.val.length := by
        simpa using inBounds
      have dropEq : store.entries.val.drop index.val =
          (t, t1) :: store.entries.val.drop (index.val + 1) := by
        rw [List.drop_eq_getElem_cons indexBound]
        rw [← t_post]
      simp only [core.cmp.impls.PartialEqShared.eq]
      rw [lawful t key]
      by_cases same : t = key
      · simp [same]
        rcases invariant with ⟨_, lookupEq⟩
        rw [dropEq] at lookupEq
        simpa [associationLookup, same] using lookupEq
      · simp [same]
        step as ⟨ index1, index1_post ⟩
        rcases invariant with ⟨upper, lookupEq⟩
        have different : key ≠ t := by
          intro equal
          exact same equal.symm
        constructor
        · scalar_tac
        · constructor
          · rw [index1_post]
            rw [dropEq] at lookupEq
            simpa [associationLookup, different] using lookupEq
          · omega
    · rename_i outOfBounds
      simp only [WP.spec_ok]
      rcases invariant with ⟨upper, lookupEq⟩
      have lower : store.entries.val.length ≤ index.val := by
        simpa using outOfBounds
      have dropEmpty : store.entries.val.drop index.val = [] :=
        List.drop_eq_nil_of_le lower
      rw [dropEmpty] at lookupEq
      simpa [associationLookup] using lookupEq
  · exact ⟨startBound, rfl⟩

/-- Arbitrary-store specification for the actual Aeneas-generated `get`. -/
theorem generated_get_refines [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K) :
    SandboxControllerRust.store.Store.get partialEq store key
      ⦃ value => value = storeLookup store key ⦄ := by
  unfold SandboxControllerRust.store.Store.get storeLookup
  simpa using generated_get_loop_refines partialEq lawful store key 0#usize
    (by simp)

/-- The generated `get_copy` loop has the same pure finite-store semantics as
    `get`; Rust's `Copy` marker carries no runtime operation here. -/
theorem generated_get_copy_loop_refines [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K)
    (start : Std.Usize) (startBound : start.val ≤ store.entries.val.length) :
    SandboxControllerRust.store.Store.get_copy_loop partialEq store key start
      ⦃ value =>
        value = associationLookup key (store.entries.val.drop start.val) ⦄ := by
  unfold SandboxControllerRust.store.Store.get_copy_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index => store.entries.val.length - index.val)
    (fun index =>
      index.val ≤ store.entries.val.length ∧
      associationLookup key (store.entries.val.drop index.val) =
        associationLookup key (store.entries.val.drop start.val))
    (fun value =>
      value = associationLookup key (store.entries.val.drop start.val))
  · intro index invariant
    unfold SandboxControllerRust.store.Store.get_copy_loop.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step
      have indexBound : index.val < store.entries.val.length := by
        simpa using inBounds
      have dropEq : store.entries.val.drop index.val =
          (t, t1) :: store.entries.val.drop (index.val + 1) := by
        rw [List.drop_eq_getElem_cons indexBound]
        rw [← t_post]
      simp only [core.cmp.impls.PartialEqShared.eq]
      rw [lawful t key]
      by_cases same : t = key
      · simp [same]
        rcases invariant with ⟨_, lookupEq⟩
        rw [dropEq] at lookupEq
        simpa [associationLookup, same] using lookupEq
      · simp [same]
        step as ⟨ index1, index1_post ⟩
        rcases invariant with ⟨upper, lookupEq⟩
        have different : key ≠ t := by
          intro equal
          exact same equal.symm
        constructor
        · scalar_tac
        · constructor
          · rw [index1_post]
            rw [dropEq] at lookupEq
            simpa [associationLookup, different] using lookupEq
          · omega
    · rename_i outOfBounds
      simp only [WP.spec_ok]
      rcases invariant with ⟨upper, lookupEq⟩
      have lower : store.entries.val.length ≤ index.val := by
        simpa using outOfBounds
      have dropEmpty : store.entries.val.drop index.val = [] :=
        List.drop_eq_nil_of_le lower
      rw [dropEmpty] at lookupEq
      simpa [associationLookup] using lookupEq
  · exact ⟨startBound, rfl⟩

/-- Arbitrary-store specification for the actual Aeneas-generated
    `Store::get_copy`. -/
theorem generated_get_copy_refines [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (copy : core.marker.Copy V)
    (store : SandboxControllerRust.store.Store K V) (key : K) :
    SandboxControllerRust.store.Store.get_copy partialEq copy store key
      ⦃ value => value = storeLookup store key ⦄ := by
  unfold SandboxControllerRust.store.Store.get_copy storeLookup
  simpa using generated_get_copy_loop_refines partialEq lawful store key
    0#usize (by simp)

/-- Arbitrary-store specification for the generated membership check. -/
theorem generated_contains_key_refines [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K) :
    SandboxControllerRust.store.Store.contains_key partialEq store key
      ⦃ found => found = (storeLookup store key).isSome ⦄ := by
  unfold SandboxControllerRust.store.Store.contains_key
  apply Aeneas.Std.WP.spec_bind
    (generated_get_refines partialEq lawful store key)
  intro value same
  subst value
  simp

/-- Equality form of `generated_contains_key_refines`, convenient when a
    handler branches directly on the generated membership result. -/
theorem generated_contains_key_eq [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K) :
    SandboxControllerRust.store.Store.contains_key partialEq store key =
      ok (storeLookup store key).isSome := by
  obtain ⟨found, evaluated, same⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_contains_key_refines partialEq lawful store key)
  subst found
  exact evaluated

/-- First-match update of the finite association list.  If the key is absent,
    the new pair is appended, matching the Rust implementation's order. -/
def associationPut [DecidableEq K] (key : K) (value : V) :
    List (K × V) → List (K × V)
  | [] => [(key, value)]
  | (storedKey, storedValue) :: rest =>
      if storedKey = key then (storedKey, value) :: rest
      else (storedKey, storedValue) :: associationPut key value rest

@[simp] theorem lookup_associationPut [DecidableEq K]
    (query key : K) (value : V) (entries : List (K × V)) :
    associationLookup query (associationPut key value entries) =
      if query = key then some value else associationLookup query entries := by
  induction entries with
  | nil => simp [associationPut, associationLookup]
  | cons entry rest induction =>
      rcases entry with ⟨storedKey, storedValue⟩
      by_cases stored : storedKey = key
      · subst storedKey
        by_cases queryMatches : query = key <;>
          simp [associationPut, associationLookup, queryMatches]
      · by_cases queryMatchesStored : query = storedKey
        · simp [associationPut, associationLookup, stored,
            queryMatchesStored]
        · simp [associationPut, associationLookup, stored,
            queryMatchesStored, induction]

theorem keys_associationPut [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V)) :
    (associationPut key value entries).map Prod.fst =
      if key ∈ entries.map Prod.fst then entries.map Prod.fst
      else entries.map Prod.fst ++ [key] := by
  induction entries with
  | nil => simp [associationPut]
  | cons entry rest induction =>
      rcases entry with ⟨storedKey, storedValue⟩
      by_cases same : storedKey = key
      · subst storedKey
        simp [associationPut]
      · by_cases member : key ∈ rest.map Prod.fst
        · simp [associationPut, same, induction, member]
        · have reverse : key ≠ storedKey := by
            intro equal
            exact same equal.symm
          simp [associationPut, same, induction, member, reverse]

theorem keys_associationPut_nodup [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V))
    (unique : (entries.map Prod.fst).Nodup) :
    ((associationPut key value entries).map Prod.fst).Nodup := by
  rw [keys_associationPut]
  by_cases member : key ∈ entries.map Prod.fst
  · simp [member, unique]
  · simp only [member, ↓reduceIte]
    rw [List.nodup_append]
    refine ⟨unique, by simp, ?_⟩
    intro existing existingMem appended appendedMem
    simp only [List.mem_singleton] at appendedMem
    subst appended
    intro equal
    apply member
    simpa [equal] using existingMem

theorem length_associationPut_le [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V)) :
    (associationPut key value entries).length ≤ entries.length + 1 := by
  induction entries with
  | nil => simp [associationPut]
  | cons entry rest induction =>
      rcases entry with ⟨storedKey, storedValue⟩
      by_cases same : storedKey = key
      · simp [associationPut, same]
      · simp [associationPut, same]
        omega

theorem associationPut_length_of_present [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V)) (old : V)
    (present : associationLookup key entries = some old) :
    (associationPut key value entries).length = entries.length := by
  induction entries with
  | nil => simp [associationLookup] at present
  | cons entry rest induction =>
      rcases entry with ⟨storedKey, storedValue⟩
      by_cases same : storedKey = key
      · simp [associationPut, same]
      · have reverse : key ≠ storedKey := by
          intro equal
          exact same equal.symm
        have tailPresent : associationLookup key rest = some old := by
          simpa [associationLookup, reverse] using present
        simp [associationPut, same, induction tailPresent]

theorem associationPut_length_of_absent [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V))
    (absent : associationLookup key entries = none) :
    (associationPut key value entries).length = entries.length + 1 := by
  induction entries with
  | nil => simp [associationPut]
  | cons entry rest induction =>
      rcases entry with ⟨storedKey, storedValue⟩
      by_cases same : storedKey = key
      · subst storedKey
        simp [associationLookup] at absent
      · have reverse : key ≠ storedKey := by
          intro equal
          exact same equal.symm
        have tailAbsent : associationLookup key rest = none := by
          simpa [associationLookup, reverse] using absent
        simp [associationPut, same, induction tailAbsent]

/-- The generated write loop implements `associationPut` from every valid
    starting index.  `capacity` is explicit because the absent-key branch calls
    `Vec::push`; the replacement branch does not consume it. -/
theorem generated_put_loop_refines [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K) (value : V)
    (start : Std.Usize) (startBound : start.val ≤ store.entries.val.length)
    (capacity : store.entries.val.length < Std.Usize.max) :
    SandboxControllerRust.store.Store.put_loop partialEq store key value start
      ⦃ entries => entries.val =
        store.entries.val.take start.val ++
          associationPut key value (store.entries.val.drop start.val) ⦄ := by
  unfold SandboxControllerRust.store.Store.put_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun (index : Std.Usize) => store.entries.val.length - index.val)
    (fun (index : Std.Usize) =>
      index.val ≤ store.entries.val.length ∧
      store.entries.val.take index.val ++
          associationPut key value (store.entries.val.drop index.val) =
        store.entries.val.take start.val ++
          associationPut key value (store.entries.val.drop start.val))
    (fun (entries : alloc.vec.Vec (K × V)) => entries.val =
      store.entries.val.take start.val ++
        associationPut key value (store.entries.val.drop start.val))
  · intro index invariant
    unfold SandboxControllerRust.store.Store.put_loop.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨ storedKey, storedValue, entryPost ⟩
      have indexBound : index.val < store.entries.val.length := by
        simpa using inBounds
      have dropEq : store.entries.val.drop index.val =
          (storedKey, storedValue) ::
            store.entries.val.drop (index.val + 1) := by
        rw [List.drop_eq_getElem_cons indexBound]
        rw [← entryPost]
      have takeEq : store.entries.val.take (index.val + 1) =
          store.entries.val.take index.val ++ [(storedKey, storedValue)] := by
        rw [List.take_succ_eq_append_getElem indexBound]
        rw [← entryPost]
      rw [lawful storedKey key]
      by_cases same : storedKey = key
      · simp [same]
        step
        have sameKey : t1 = storedKey :=
          congrArg Prod.fst (t1_post.trans entryPost.symm)
        subst t1
        rw [index_mut_back_post]
        rcases invariant with ⟨_, updateEq⟩
        rw [dropEq] at updateEq
        simpa [alloc.vec.Vec.set, List.set_eq_take_append_cons_drop,
          indexBound, associationPut, same] using updateEq
      · simp [same]
        step as ⟨ index1, index1Post ⟩
        rcases invariant with ⟨upper, updateEq⟩
        have different : storedKey ≠ key := same
        constructor
        · scalar_tac
        · constructor
          · rw [index1Post, takeEq]
            rw [dropEq] at updateEq
            simpa [associationPut, different, List.append_assoc] using updateEq
          · omega
    · rename_i outOfBounds
      step as ⟨ appended, appendedPost ⟩
      rcases invariant with ⟨upper, updateEq⟩
      have lower : store.entries.val.length ≤ index.val := by
        simpa using outOfBounds
      have atEnd : index.val = store.entries.val.length := by omega
      rw [appendedPost]
      calc
        store.entries.val ++ [(key, value)] =
            store.entries.val.take index.val ++
              associationPut key value (store.entries.val.drop index.val) := by
                rw [atEnd]
                simp [associationPut]
        _ = store.entries.val.take start.val ++
              associationPut key value (store.entries.val.drop start.val) := updateEq
  · exact ⟨startBound, rfl⟩

/-- Arbitrary-store specification for the actual Aeneas-generated `put`. -/
theorem generated_put_refines [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K) (value : V)
    (capacity : StoreHeadroom store) :
    SandboxControllerRust.store.Store.put partialEq store key value
      ⦃ next =>
        next.entries.val = associationPut key value store.entries.val ⦄ := by
  unfold SandboxControllerRust.store.Store.put
  apply Aeneas.Std.WP.spec_bind
    (generated_put_loop_refines partialEq lawful store key value 0#usize
      (by simp) capacity)
  intro entries entriesPost
  simpa using entriesPost

/-- The list-level update preserves the complete finite-store representation
    invariant.  Injectivity of the identifier embedding is what lets an
    abstract key comparison determine the unique concrete key. -/
theorem storeRep_of_associationPut
    [DecidableEq RustKey] [DecidableEq ModelKey]
    (keyMap : RustKey → ModelKey) (keyMapInjective : Function.Injective keyMap)
    (valueRel : RustValue → ModelValue → Prop)
    (rust : SandboxControllerRust.store.Store RustKey RustValue)
    (model : ModelKey → Option ModelValue)
    (represented : StoreRep keyMap valueRel rust model)
    (key : RustKey) (rustValue : RustValue) (modelValue : ModelValue)
    (valueRelated : valueRel rustValue modelValue)
    (next : SandboxControllerRust.store.Store RustKey RustValue)
    (nextEntries : next.entries.val =
      associationPut key rustValue rust.entries.val)
    (headroom : StoreHeadroom rust) :
    StoreRep keyMap valueRel next
      (SandboxModel.put model (keyMap key) modelValue) := by
  constructor
  · rw [nextEntries]
    exact keys_associationPut_nodup key rustValue rust.entries.val
      represented.keyUnique
  · intro candidate
    unfold storeLookup
    rw [nextEntries, lookup_associationPut]
    by_cases same : candidate = key
    · subst candidate
      simp [SandboxModel.put, OptionRel, valueRelated]
    · have mappedDifferent : keyMap candidate ≠ keyMap key := by
        intro equal
        exact same (keyMapInjective equal)
      simp only [same, ↓reduceIte]
      simpa [storeLookup, SandboxModel.put, mappedDifferent] using
        represented.lookupAgrees candidate
  · intro candidate value found
    by_cases same : candidate = keyMap key
    · exact ⟨key, same.symm⟩
    · have oldFound : model candidate = some value := by
        simpa [SandboxModel.put, same] using found
      exact represented.modelSupport candidate value oldFound
  · rw [nextEntries]
    calc
      (associationPut key rustValue rust.entries.val).length ≤
          rust.entries.val.length + 1 :=
        length_associationPut_le key rustValue rust.entries.val
      _ ≤ Std.Usize.max := by
        unfold StoreHeadroom at headroom
        omega

/-- Main write-refinement theorem.  It covers replacement and append with one
    postcondition and makes the append-capacity premise explicit. -/
theorem generated_put_preserves_storeRep
    [DecidableEq RustKey] [DecidableEq ModelKey]
    (partialEq : core.cmp.PartialEq RustKey RustKey)
    (lawful : LawfulPartialEq partialEq)
    (keyMap : RustKey → ModelKey) (keyMapInjective : Function.Injective keyMap)
    (valueRel : RustValue → ModelValue → Prop)
    (rust : SandboxControllerRust.store.Store RustKey RustValue)
    (model : ModelKey → Option ModelValue)
    (represented : StoreRep keyMap valueRel rust model)
    (key : RustKey) (rustValue : RustValue) (modelValue : ModelValue)
    (valueRelated : valueRel rustValue modelValue)
    (headroom : StoreHeadroom rust) :
    SandboxControllerRust.store.Store.put partialEq rust key rustValue
      ⦃ next =>
        StoreRep keyMap valueRel next
          (SandboxModel.put model (keyMap key) modelValue) ⦄ := by
  apply Aeneas.Std.WP.spec_mono
    (generated_put_refines partialEq lawful rust key rustValue headroom)
  intro next nextEntries
  exact storeRep_of_associationPut keyMap keyMapInjective valueRel rust model
    represented key rustValue modelValue valueRelated next nextEntries headroom

/-- Existing-key branch: the generated write preserves the representation and
    does not change the vector length. -/
theorem generated_put_existing_preserves_storeRep
    [DecidableEq RustKey] [DecidableEq ModelKey]
    (partialEq : core.cmp.PartialEq RustKey RustKey)
    (lawful : LawfulPartialEq partialEq)
    (keyMap : RustKey → ModelKey) (keyMapInjective : Function.Injective keyMap)
    (valueRel : RustValue → ModelValue → Prop)
    (rust : SandboxControllerRust.store.Store RustKey RustValue)
    (model : ModelKey → Option ModelValue)
    (represented : StoreRep keyMap valueRel rust model)
    (key : RustKey) (oldValue rustValue : RustValue)
    (modelValue : ModelValue) (valueRelated : valueRel rustValue modelValue)
    (present : storeLookup rust key = some oldValue)
    (headroom : StoreHeadroom rust) :
    SandboxControllerRust.store.Store.put partialEq rust key rustValue
      ⦃ next =>
        StoreRep keyMap valueRel next
            (SandboxModel.put model (keyMap key) modelValue) ∧
          next.entries.val.length = rust.entries.val.length ⦄ := by
  apply Aeneas.Std.WP.spec_mono
    (generated_put_refines partialEq lawful rust key rustValue headroom)
  intro next nextEntries
  constructor
  · exact storeRep_of_associationPut keyMap keyMapInjective valueRel rust model
      represented key rustValue modelValue valueRelated next nextEntries headroom
  · rw [nextEntries]
    apply associationPut_length_of_present key rustValue rust.entries.val oldValue
    simpa [storeLookup] using present

/-- Fresh-key branch: the generated write preserves the representation and
    appends exactly one entry. -/
theorem generated_put_fresh_preserves_storeRep
    [DecidableEq RustKey] [DecidableEq ModelKey]
    (partialEq : core.cmp.PartialEq RustKey RustKey)
    (lawful : LawfulPartialEq partialEq)
    (keyMap : RustKey → ModelKey) (keyMapInjective : Function.Injective keyMap)
    (valueRel : RustValue → ModelValue → Prop)
    (rust : SandboxControllerRust.store.Store RustKey RustValue)
    (model : ModelKey → Option ModelValue)
    (represented : StoreRep keyMap valueRel rust model)
    (key : RustKey) (rustValue : RustValue) (modelValue : ModelValue)
    (valueRelated : valueRel rustValue modelValue)
    (absent : storeLookup rust key = none)
    (headroom : StoreHeadroom rust) :
    SandboxControllerRust.store.Store.put partialEq rust key rustValue
      ⦃ next =>
        StoreRep keyMap valueRel next
            (SandboxModel.put model (keyMap key) modelValue) ∧
          next.entries.val.length = rust.entries.val.length + 1 ⦄ := by
  apply Aeneas.Std.WP.spec_mono
    (generated_put_refines partialEq lawful rust key rustValue headroom)
  intro next nextEntries
  constructor
  · exact storeRep_of_associationPut keyMap keyMapInjective valueRel rust model
      represented key rustValue modelValue valueRelated next nextEntries headroom
  · rw [nextEntries]
    apply associationPut_length_of_absent key rustValue rust.entries.val
    simpa [storeLookup] using absent

/-- A concrete empty Rust store, used by the executable empty-world bridge. -/
def emptyStore (K V : Type) : SandboxControllerRust.store.Store K V :=
  { entries := alloc.vec.Vec.new (K × V) }

def singletonStore (key : K) (value : V) :
    SandboxControllerRust.store.Store K V :=
  { entries := alloc.vec.Vec.from [(key, value)] (by scalar_tac) }

@[simp] theorem emptyStore_entries (K V : Type) :
    (emptyStore K V).entries.val = [] := by
  simp [emptyStore]

@[simp] theorem storeLookup_empty [DecidableEq K] (key : K) :
    storeLookup (emptyStore K V) key = none := by
  simp [storeLookup, associationLookup]

theorem emptyStore_rep [DecidableEq RustKey]
    (keyMap : RustKey → ModelKey)
    (valueRel : RustValue → ModelValue → Prop) :
    StoreRep keyMap valueRel (emptyStore RustKey RustValue)
      (fun _ => none) := by
  constructor
  · simp [emptyStore]
  · intro key
    simp [OptionRel, storeLookup, associationLookup]
  · intro key value found
    simp at found
  · simp [emptyStore]

@[simp] theorem singletonStore_entries (key : K) (value : V) :
    (singletonStore key value).entries.val = [(key, value)] := by
  simp [singletonStore]

@[simp] theorem storeLookup_singleton_self [DecidableEq K] (key : K) (value : V) :
    storeLookup (singletonStore key value) key = some value := by
  simp [storeLookup, associationLookup]

/-- Direct machine check of the generated Rust `Store::get` on an empty store. -/
theorem generated_get_empty
    (partialEq : core.cmp.PartialEq K K) (key : K) :
    SandboxControllerRust.store.Store.get partialEq (emptyStore K V) key = ok none := by
  unfold SandboxControllerRust.store.Store.get
  rw [SandboxControllerRust.store.Store.get_loop.eq_1]
  simp [SandboxControllerRust.store.Store.get_loop.body, emptyStore]
  rw [Aeneas.Std.loop.eq_1]
  simp

/-- The generated `contains_key` agrees that an empty store has no key. -/
theorem generated_contains_key_empty
    (partialEq : core.cmp.PartialEq K K) (key : K) :
    SandboxControllerRust.store.Store.contains_key partialEq (emptyStore K V) key = ok false := by
  simp [SandboxControllerRust.store.Store.contains_key, generated_get_empty]

/-- The actual generated loop returns the stored value from a one-entry store,
    provided the Rust equality implementation reports reflexive equality. -/
theorem generated_get_singleton_self
    (partialEq : core.cmp.PartialEq K K) (key : K) (value : V)
    (equality : partialEq.eq key key = ok true) :
    SandboxControllerRust.store.Store.get partialEq
        (singletonStore key value) key = ok (some value) := by
  unfold SandboxControllerRust.store.Store.get
  rw [SandboxControllerRust.store.Store.get_loop.eq_1]
  rw [Aeneas.Std.loop.eq_1]
  simp [SandboxControllerRust.store.Store.get_loop.body, singletonStore,
    alloc.vec.Vec.index_usize, equality]

/-- Instantiation of the singleton theorem with the exact equality code generated
    for the Rust `SandboxId` newtype. -/
theorem generated_get_sandbox_singleton
    (key : SandboxControllerRust.ids.SandboxId) (value : V) :
    SandboxControllerRust.store.Store.get
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
        (singletonStore key value) key = ok (some value) := by
  apply generated_get_singleton_self
  simp [SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]

/-- On the empty store, the actual generated `Store::put` creates exactly one
    association.  The postcondition is phrased with Aeneas' weakest-precondition
    judgment, so it also proves that this Rust path cannot fail. -/
theorem generated_put_empty
    (partialEq : core.cmp.PartialEq K K) (key : K) (value : V) :
    SandboxControllerRust.store.Store.put partialEq (emptyStore K V) key value
      ⦃ next => next.entries.val = [(key, value)] ⦄ := by
  unfold SandboxControllerRust.store.Store.put
  rw [SandboxControllerRust.store.Store.put_loop.eq_1]
  rw [Aeneas.Std.loop.eq_1]
  have capacity : 1 ≤ Std.U32.max ∨ 1 ≤ Std.Usize.max := by
    left
    scalar_tac
  simp [SandboxControllerRust.store.Store.put_loop.body, emptyStore,
    alloc.vec.Vec.push, capacity]

/-- A one-entry Rust store represents the corresponding functional model
    update whenever the key embedding is injective. -/
theorem singletonStore_rep
    [DecidableEq RustKey] [DecidableEq ModelKey]
    (keyMap : RustKey → ModelKey) (keyMapInjective : Function.Injective keyMap)
    (valueMap : RustValue → ModelValue) (key : RustKey) (value : RustValue) :
    StoreRep keyMap (fun rust model => valueMap rust = model)
      (singletonStore key value)
      (SandboxModel.put (fun _ => none) (keyMap key) (valueMap value)) := by
  constructor
  · simp [singletonStore]
  · intro candidate
    by_cases same : candidate = key
    · subst candidate
      simp [OptionRel, SandboxModel.put]
    · have mappedDifferent : keyMap candidate ≠ keyMap key := by
        intro equal
        exact same (keyMapInjective equal)
      simp [OptionRel, storeLookup, associationLookup, singletonStore,
        SandboxModel.put, same, mappedDifferent]
  · intro candidate mappedValue found
    by_cases same : candidate = keyMap key
    · exact ⟨key, same.symm⟩
    · simp [SandboxModel.put, same] at found
  · simp [singletonStore]
    scalar_tac

/-- The concrete empty-store `put` establishes the full one-entry store
    representation invariant, not merely the shape of the returned vector. -/
theorem generated_put_empty_refines
    [DecidableEq RustKey] [DecidableEq ModelKey]
    (partialEq : core.cmp.PartialEq RustKey RustKey)
    (keyMap : RustKey → ModelKey) (keyMapInjective : Function.Injective keyMap)
    (valueMap : RustValue → ModelValue) (key : RustKey) (value : RustValue) :
    SandboxControllerRust.store.Store.put partialEq
        (emptyStore RustKey RustValue) key value
      ⦃ next =>
        StoreRep keyMap (fun rust model => valueMap rust = model) next
          (SandboxModel.put (fun _ => none) (keyMap key) (valueMap value)) ⦄ := by
  apply Aeneas.Std.WP.spec_mono (generated_put_empty partialEq key value)
  intro next entries
  have nextEq : next = singletonStore key value := by
    cases next with
    | mk nextEntries =>
        apply congrArg SandboxControllerRust.store.Store.mk
        apply alloc.vec.Vec.ext
        simpa [singletonStore] using entries
  subst next
  exact singletonStore_rep keyMap keyMapInjective valueMap key value

end SandboxModel.RustRefinement
