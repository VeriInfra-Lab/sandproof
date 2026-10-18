import AccessProofs
import CredentialProofs
import LifetimeFiniteProofs
import ByteProofs
import TokenFormatProofs
import RawRowProofs

/-! Composition of actual extracted synchronous results. This does not claim
that the unextracted async callback itself has been translated. The caller must
provide the explicit snapshot/argument projections below under its lock. -/
namespace RedispatchComposition
open Aeneas Aeneas.Std

abbrev World := AccessProofs.World
abbrev Record := AccessProofs.Record
abbrev Actor := AccessProofs.Actor
abbrev Sid := AccessProofs.Sid
abbrev Vm := AccessProofs.Vm
abbrev Digest := CredentialProofs.Digest

inductive Kind where
  | exec | fileRead | fileWrite

def actorProjection : Actor → Option U64
  | .Tenant owner => some owner
  | .SystemController => none

def phaseProjection : SandproofNetworkRust.sandbox_controller_core.model.Phase →
    Authority.lifetime_core.LifetimePhase
  | .Starting => .Starting
  | .Running => .Running
  | .Pausing => .Pausing
  | .Paused => .Paused
  | .Revoking => .Revoking
  | .Destroyed => .Destroyed

theorem actor_projection_iff (actor : Actor) (owner : U64) :
    actorProjection actor = some owner ↔ actor = .Tenant owner := by
  cases actor <;> simp [actorProjection]

theorem phase_projection_running_iff
    (phase : SandproofNetworkRust.sandbox_controller_core.model.Phase) :
    phaseProjection phase = .Running ↔ phase = .Running := by
  cases phase <;> simp [phaseProjection]

theorem token_success_iff (token : Slice U8) :
    Authority.credential_core.byte_core.token_format token = .ok true ↔
      TokenFormatProofs.Format token := by
  rw [TokenFormatProofs.token_format_exact]
  simp

theorem digest_success_iff (actual registered : Digest) :
    Authority.credential_core.digest_matches actual registered = .ok true ↔
      actual.val = registered.val := by
  rw [ByteProofs.digest_matches_exact]
  simp

theorem row_success_iff (row : Authority.lifetime_core.RawLifetimeRow) (expected : Slice U8)
    (lifetime : Authority.lifetime_core.Lifetime) :
    Authority.lifetime_core.decode_row row expected = .ok (.Ok lifetime) ↔
      RawRowProofs.RowValid row expected ∧ RawRowProofs.decoded row = lifetime := by
  rw [RawRowProofs.decode_row_exact]
  by_cases valid : RawRowProofs.RowValid row expected <;> simp [valid]

structure Inputs where
  kind : Kind
  world : World
  actor : Actor
  sandbox : Sid
  vm : Vm
  token : Slice U8
  stored : Option (U64 × Digest)
  inverse : Option (U64 × U64)
  actualDigest : Digest
  registeredDigest : Digest
  rawLifetime : Authority.lifetime_core.RawLifetimeRow
  expectedOwnerBytes : Slice U8
  lifetime : Authority.lifetime_core.Lifetime
  now : I64
  actorArgument : Option U64
  recordArgument : Option (U64 × Authority.lifetime_core.LifetimePhase)

def accessObservation (input : Inputs) (generation : U64) : Result AccessProofs.Outcome :=
  match input.kind with
  | .exec => SandproofNetworkRust.sandbox_controller_core.auxiliary.handle_exec
      input.world input.actor input.sandbox input.vm
  | .fileRead => SandproofNetworkRust.sandbox_controller_core.auxiliary.handle_file_read
      input.world input.actor input.sandbox generation
  | .fileWrite => SandproofNetworkRust.sandbox_controller_core.auxiliary.handle_file_write
      input.world input.actor input.sandbox generation

def expectedEvent (input : Inputs) (generation : U64) :
    SandproofNetworkRust.sandbox_controller_core.auxiliary.AuxiliaryEvent :=
  match input.kind with
  | .exec => .Exec input.actor input.sandbox
  | .fileRead => .FileRead input.actor input.sandbox generation
  | .fileWrite => .FileWrite input.actor input.sandbox generation

def AccessBinding (input : Inputs) (record : Record) : Prop :=
  match input.kind with
  | .exec => AccessProofs.ExactExecBinding record input.sandbox input.vm
  | .fileRead | .fileWrite => AccessProofs.ExactFileBinding record input.sandbox record.generation

theorem access_success_iff (input : Inputs) (record : Record)
    (lookup : SandboxModel.Foundations.tableLookup input.sandbox
      input.world.sandboxes.entries.val = some record)
    (actor : input.actor = .Tenant record.owner) (phase : record.phase = .Running) :
    accessObservation input record.generation =
      .ok (.Ok ⟨expectedEvent input record.generation, input.world⟩) ↔ AccessBinding input record := by
  cases kind : input.kind with
  | exec =>
      simp only [accessObservation, expectedEvent, AccessBinding, kind]
      rw [AccessProofs.exec_success_iff]
      simp [AccessProofs.ExecRequirements, lookup, actor]
  | fileRead =>
      simp only [accessObservation, expectedEvent, AccessBinding, kind]
      rw [AccessProofs.file_read_success_iff]
      simp [AccessProofs.FileRequirements, lookup, actor, phase]
  | fileWrite =>
      simp only [accessObservation, expectedEvent, AccessBinding, kind]
      rw [AccessProofs.file_write_success_iff]
      simp [AccessProofs.FileRequirements, lookup, actor, phase]

/-- Raw observations and real callee results; no credentialValid, lifetimeLive
    or accessAllowed boolean is supplied by the caller. Hashing/storage and
    argument movement remain explicit boundary observations. -/
def Checks (input : Inputs) (record : Record) : Prop :=
  Authority.credential_core.byte_core.token_format input.token = .ok true ∧
  Authority.credential_core.registered_digest input.stored record.generation =
    .ok (.Ok input.registeredDigest) ∧
  Authority.credential_core.inverse_binding input.sandbox record.generation input.inverse =
    .ok (.Ok ()) ∧
  Authority.credential_core.digest_matches input.actualDigest input.registeredDigest = .ok true ∧
  Authority.lifetime_core.decode_row input.rawLifetime input.expectedOwnerBytes =
    .ok (.Ok input.lifetime) ∧
  Authority.lifetime_core.check_clock input.now = .ok (.Ok ()) ∧
  Authority.lifetime_core.check_record input.actorArgument input.recordArgument =
    .ok (.Ok record.owner) ∧
  Authority.lifetime_core.check_live input.now (some input.lifetime) =
    .ok (.Ok input.lifetime) ∧
  accessObservation input record.generation =
    .ok (.Ok ⟨expectedEvent input record.generation, input.world⟩)

/-- Every successful check is exposed as a mathematical property of original
    bytes/row fields/identity/state, including exact decoded output fields. -/
def Requirements (input : Inputs) (record : Record) : Prop :=
  TokenFormatProofs.Format input.token ∧
  input.stored = some (record.generation, input.registeredDigest) ∧
  input.inverse = some (input.sandbox, record.generation) ∧
  input.actualDigest.val = input.registeredDigest.val ∧
  (RawRowProofs.RowValid input.rawLifetime input.expectedOwnerBytes ∧
    RawRowProofs.decoded input.rawLifetime = input.lifetime) ∧
  0 ≤ input.now.val ∧
  input.actor = .Tenant record.owner ∧
  record.phase = .Running ∧
  input.now.val < input.lifetime.deadline_ms.val ∧
  AccessBinding input record

/-- All three families use the same observed generation, sandbox, owner and
    World. The caller-provided equations state its exact argument projection;
    they do not assume the desired authorization decision. -/
theorem checks_iff_requirements (input : Inputs) (record : Record)
    (lookup : SandboxModel.Foundations.tableLookup input.sandbox
      input.world.sandboxes.entries.val = some record)
    (actorMapping : input.actorArgument = actorProjection input.actor)
    (recordMapping : input.recordArgument = some (record.owner, phaseProjection record.phase)) :
    Checks input record ↔ Requirements input record := by
  constructor
  · rintro ⟨token, registered, inverse, digest, row, clock, liveRecord, live, access⟩
    have recordFacts := (LifetimeFiniteProofs.record_success_iff _ _ record.owner).mp liveRecord
    have actorFacts : actorProjection input.actor = some record.owner := by
      simpa [actorMapping] using recordFacts.2
    have phaseFacts : phaseProjection record.phase = .Running := by
      simpa [recordMapping] using recordFacts.1
    have actualActor := (actor_projection_iff _ _).mp actorFacts
    have actualPhase := (phase_projection_running_iff _).mp phaseFacts
    exact ⟨(token_success_iff _).mp token,
      (CredentialProofs.registered_success_iff _ _ _).mp registered,
      (CredentialProofs.inverse_success_iff _ _ _).mp inverse,
      (digest_success_iff _ _).mp digest, (row_success_iff _ _ _).mp row,
      (LifetimeFiniteProofs.clock_success_iff _).mp clock,
      actualActor, actualPhase,
      ((LifetimeFiniteProofs.live_success_iff _ _ _).mp live).2,
      (access_success_iff input record lookup actualActor actualPhase).mp access⟩
  · rintro ⟨token, registered, inverse, digest, row, clock, actor, phase, live, binding⟩
    refine ⟨(token_success_iff _).mpr token,
      (CredentialProofs.registered_success_iff _ _ _).mpr registered,
      (CredentialProofs.inverse_success_iff _ _ _).mpr inverse,
      (digest_success_iff _ _).mpr digest, (row_success_iff _ _ _).mpr row,
      (LifetimeFiniteProofs.clock_success_iff _).mpr clock, ?_,
      (LifetimeFiniteProofs.live_success_iff _ _ _).mpr ⟨rfl, live⟩, ?_⟩
    · apply (LifetimeFiniteProofs.record_success_iff _ _ record.owner).mpr
      constructor
      · simp [recordMapping, phase, phaseProjection]
      · simp [actorMapping, actor, actorProjection]
    · exact (access_success_iff input record lookup actor phase).mpr binding

theorem closed_authority_cannot_satisfy_checks (input : Inputs) (record : Record)
    (lookup : SandboxModel.Foundations.tableLookup input.sandbox
      input.world.sandboxes.entries.val = some record)
    (actorMapping : input.actorArgument = actorProjection input.actor)
    (recordMapping : input.recordArgument = some (record.owner, phaseProjection record.phase))
    (closed : record.effective = .Closed) : ¬ Checks input record := by
  intro checks
  have facts := (checks_iff_requirements input record lookup actorMapping recordMapping).mp checks
  have binding := facts.2.2.2.2.2.2.2.2.2
  cases kind : input.kind <;> simp only [AccessBinding, kind] at binding
  all_goals
    rcases binding with ⟨sid, generation, machine, workspace, access, slot, policy, effective, _⟩
    rw [closed] at effective
    cases effective

theorem decoded_disk_is_original_positive_field (input : Inputs) (record : Record)
    (lookup : SandboxModel.Foundations.tableLookup input.sandbox
      input.world.sandboxes.entries.val = some record)
    (actorMapping : input.actorArgument = actorProjection input.actor)
    (recordMapping : input.recordArgument = some (record.owner, phaseProjection record.phase))
    (checks : Checks input record) :
    input.lifetime.disk_bytes.val = input.rawLifetime.bytes.val.toNat := by
  have facts := (checks_iff_requirements input record lookup actorMapping recordMapping).mp checks
  have rowFacts := facts.2.2.2.2.1
  have positive := rowFacts.1.2.2.2.1
  rw [← rowFacts.2]
  exact RawRowProofs.decoded_disk_exact input.rawLifetime positive

end RedispatchComposition
