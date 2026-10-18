import SandboxModel.Access.Guards
import SandboxModel.Access.Cloning
import SandboxModel.Access.Lookup

namespace AccessProofs
open Aeneas Aeneas.Std SandproofNetworkRust
open SandboxModel.Access

deriving instance DecidableEq for sandbox_controller_core.model.Phase
deriving instance DecidableEq for sandbox_controller_core.model.WorkspaceApiAccess
local instance : Nonempty sandbox_controller_core.model.WorkspaceApiAccess := ⟨.ManagedReadWrite⟩

abbrev World := sandbox_controller_core.model.World
abbrev Record := sandbox_controller_core.model.SandboxRecord
abbrev Actor := sandbox_controller_core.model.Actor
abbrev Sid := sandbox_controller_core.ids.SandboxId
abbrev Vm := sandbox_controller_core.ids.VmId
abbrev Decision := sandbox_controller_core.auxiliary.AuxiliaryDecision
abbrev Reject := sandbox_controller_core.auxiliary.AuxiliaryReject
abbrev Outcome := core.result.Result Decision Reject

/-- Exact effective execution binding. Phase is checked by the outer lifetime
    boundary, not invented here as an additional `handle_exec` condition. -/
def execAllowed (record : Record) (sandbox : Sid) (vm : Vm) : Bool :=
  match record.effective with
  | .Running sid generation machine _ _ _ _ =>
      decide (sid = sandbox ∧ generation = record.generation ∧ machine = vm)
  | .Closed | .Paused _ _ => false

/-- This includes the real workspace/access fields, even though the frozen
    WorkspaceApiAccess enum currently contains only ManagedReadWrite. -/
def fileAllowed (record : Record) (sandbox : Sid) (generation : U64) : Bool :=
  match record.effective with
  | .Running sid effectiveGeneration _ workspace access _ _ =>
      decide (sid = sandbox ∧ effectiveGeneration = generation ∧
        workspace = record.workspace ∧ access = record.workspace_access)
  | .Closed | .Paused _ _ => false

def execExpected (world : World) (actor : Actor) (sandbox : Sid) (vm : Vm) : Outcome :=
  match SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => .Err .SandboxNotFound
  | some record =>
      if !authorized (actorView actor) record.owner.val then .Err .Unauthorized
      else if !execAllowed record sandbox vm then .Err .CapabilityDenied
      else .Ok ⟨.Exec actor sandbox, world⟩

def fileExpected (write : Bool) (world : World) (actor : Actor)
    (sandbox : Sid) (generation : U64) : Outcome :=
  match SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => .Err .SandboxNotFound
  | some record =>
      if record.phase ≠ .Running then .Err .WrongPhase
      else if generation ≠ record.generation then .Err .WrongGeneration
      else if !authorized (actorView actor) record.owner.val then .Err .Unauthorized
      else if !fileAllowed record sandbox generation then .Err .CapabilityDenied
      else .Ok ⟨if write then .FileWrite actor sandbox generation
        else .FileRead actor sandbox generation, world⟩

theorem exec_guard_refines (record : Record) (sandbox : Sid) (vm : Vm) :
    sandbox_controller_core.auxiliary.allows_exec record sandbox vm =
      .ok (execAllowed record sandbox vm) := by
  cases authority : record.effective with
  | Closed => simp [sandbox_controller_core.auxiliary.allows_exec, authority, execAllowed]
  | Paused _ _ => simp [sandbox_controller_core.auxiliary.allows_exec, authority, execAllowed]
  | Running sid generation machine workspace access slot policy =>
      by_cases a : sid = sandbox <;> by_cases b : generation = record.generation <;>
        by_cases c : machine = vm <;>
          simp [sandbox_controller_core.auxiliary.allows_exec, authority, execAllowed,
            sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
            sandbox_controller_core.ids.VmId.Insts.CoreCmpPartialEqVmId.eq, a, b, c]

theorem file_guard_refines (record : Record) (sandbox : Sid) (generation : U64) :
    sandbox_controller_core.auxiliary.allows_file record sandbox generation =
      .ok (fileAllowed record sandbox generation) := by
  cases authority : record.effective with
  | Closed => simp [sandbox_controller_core.auxiliary.allows_file, authority, fileAllowed]
  | Paused _ _ => simp [sandbox_controller_core.auxiliary.allows_file, authority, fileAllowed]
  | Running sid effectiveGeneration machine workspace access slot policy =>
      have accessSame : access = record.workspace_access := by
        cases access; cases record.workspace_access; rfl
      by_cases a : sid = sandbox <;> by_cases b : effectiveGeneration = generation <;>
        by_cases c : workspace = record.workspace <;>
          simp [sandbox_controller_core.auxiliary.allows_file, authority, fileAllowed,
            sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
            sandbox_controller_core.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
            sandbox_controller_core.model.WorkspaceApiAccess.Insts.CoreCmpPartialEqWorkspaceApiAccess.eq,
            a, b, c, accessSame]

theorem phase_ne_running (phase : sandbox_controller_core.model.Phase) :
    core.cmp.PartialEq.ne.trait_default
      sandbox_controller_core.model.Phase.Insts.CoreCmpPartialEqPhase phase .Running =
      .ok (decide (phase ≠ .Running)) := by
  cases phase <;>
    simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      sandbox_controller_core.model.Phase.Insts.CoreCmpPartialEqPhase.eq,
      sandbox_controller_core.model.Phase.read_discriminant]

theorem exec_refines (world : World) (actor : Actor) (sandbox : Sid) (vm : Vm) :
    sandbox_controller_core.auxiliary.handle_exec world actor sandbox vm =
      .ok (execExpected world actor sandbox vm) := by
  unfold sandbox_controller_core.auxiliary.handle_exec execExpected
  rw [store_get_refines _ sandbox_equality_lawful]
  cases found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => simp
  | some record =>
      simp only [bind_tc_ok]
      rw [authorized_refines]
      by_cases owner : authorized (actorView actor) record.owner.val = true
      · rw [exec_guard_refines]
        cases execAllowed record sandbox vm <;> simp [owner]
      · simp [owner]

theorem file_read_refines (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) :
    sandbox_controller_core.auxiliary.handle_file_read world actor sandbox generation =
      .ok (fileExpected false world actor sandbox generation) := by
  unfold sandbox_controller_core.auxiliary.handle_file_read fileExpected
  rw [store_get_refines _ sandbox_equality_lawful]
  cases found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => simp
  | some record =>
      simp only [bind_tc_ok]
      rw [phase_ne_running]
      by_cases phase : record.phase = .Running
      · by_cases gen : generation = record.generation
        · simp only [phase, ne_eq, not_true_eq_false, decide_false, bind_tc_ok,
            Bool.false_eq_true, ↓reduceIte, gen, bne_self_eq_false]
          rw [authorized_refines]
          by_cases owner : authorized (actorView actor) record.owner.val = true
          · rw [file_guard_refines]
            cases fileAllowed record sandbox record.generation <;> simp [owner]
          · simp [owner]
        · simp [phase, gen]
      · simp [phase]

theorem file_write_refines (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) :
    sandbox_controller_core.auxiliary.handle_file_write world actor sandbox generation =
      .ok (fileExpected true world actor sandbox generation) := by
  unfold sandbox_controller_core.auxiliary.handle_file_write fileExpected
  rw [store_get_refines _ sandbox_equality_lawful]
  cases found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => simp
  | some record =>
      simp only [bind_tc_ok]
      rw [phase_ne_running]
      by_cases phase : record.phase = .Running
      · by_cases gen : generation = record.generation
        · simp only [phase, ne_eq, not_true_eq_false, decide_false, bind_tc_ok,
            Bool.false_eq_true, ↓reduceIte, gen, bne_self_eq_false]
          rw [authorized_refines]
          by_cases owner : authorized (actorView actor) record.owner.val = true
          · rw [file_guard_refines]
            cases fileAllowed record sandbox record.generation <;> simp [owner]
          · simp [owner]
        · simp [phase, gen]
      · simp [phase]

theorem exec_root_refines (world : World) (actor : Actor) (sandbox : Sid) (vm : Vm) :
    SandproofNetworkRust.access_exec world actor sandbox vm =
      .ok (execExpected world actor sandbox vm) := exec_refines world actor sandbox vm

theorem file_read_root_refines (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) :
    SandproofNetworkRust.access_file_read world actor sandbox generation =
      .ok (fileExpected false world actor sandbox generation) :=
  file_read_refines world actor sandbox generation

theorem file_write_root_refines (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) :
    SandproofNetworkRust.access_file_write world actor sandbox generation =
      .ok (fileExpected true world actor sandbox generation) :=
  file_write_refines world actor sandbox generation

theorem access_exec_refines (world : World) (actor : Actor) (sandbox : Sid) (vm : Vm) :
    sandbox_controller_core.auxiliary.handle_access world (.Exec actor sandbox vm) =
      .ok (execExpected world actor sandbox vm) := exec_refines world actor sandbox vm

theorem access_file_read_refines (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) :
    sandbox_controller_core.auxiliary.handle_access world (.FileRead actor sandbox generation) =
      .ok (fileExpected false world actor sandbox generation) :=
  file_read_refines world actor sandbox generation

theorem access_file_write_refines (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) :
    sandbox_controller_core.auxiliary.handle_access world (.FileWrite actor sandbox generation) =
      .ok (fileExpected true world actor sandbox generation) :=
  file_write_refines world actor sandbox generation

def ExactExecBinding (record : Record) (sandbox : Sid) (vm : Vm) : Prop :=
  ∃ sid generation machine workspace access slot policy,
    record.effective = .Running sid generation machine workspace access slot policy ∧
    sid = sandbox ∧ generation = record.generation ∧ machine = vm

def ExactFileBinding (record : Record) (sandbox : Sid) (generation : U64) : Prop :=
  ∃ sid effectiveGeneration machine workspace access slot policy,
    record.effective = .Running sid effectiveGeneration machine workspace access slot policy ∧
    sid = sandbox ∧ effectiveGeneration = generation ∧
      workspace = record.workspace ∧ access = record.workspace_access

def ExecRequirements (world : World) (actor : Actor) (sandbox : Sid) (vm : Vm) : Prop :=
  ∃ record,
    SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val = some record ∧
    (actor = .Tenant record.owner ∨ actor = .SystemController) ∧
    ExactExecBinding record sandbox vm

def FileRequirements (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) : Prop :=
  ∃ record,
    SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val = some record ∧
    record.phase = .Running ∧ generation = record.generation ∧
    (actor = .Tenant record.owner ∨ actor = .SystemController) ∧
    ExactFileBinding record sandbox generation

theorem actor_authorized_iff (actor : Actor) (owner : sandbox_controller_core.ids.TenantId) :
    authorized (actorView actor) owner.val = true ↔
      (actor = .Tenant owner ∨ actor = .SystemController) := by
  cases actor with
  | Tenant tenant =>
      simp [authorized, actorView, ← UScalar.eq_equiv]
  | SystemController => simp [authorized, actorView]

theorem exec_allowed_iff (record : Record) (sandbox : Sid) (vm : Vm) :
    execAllowed record sandbox vm = true ↔ ExactExecBinding record sandbox vm := by
  cases authority : record.effective <;> simp [execAllowed, ExactExecBinding, authority]

theorem file_allowed_iff (record : Record) (sandbox : Sid) (generation : U64) :
    fileAllowed record sandbox generation = true ↔ ExactFileBinding record sandbox generation := by
  cases authority : record.effective <;> simp [fileAllowed, ExactFileBinding, authority]

theorem exec_expected_success_iff (world : World) (actor : Actor) (sandbox : Sid) (vm : Vm) :
    execExpected world actor sandbox vm = .Ok ⟨.Exec actor sandbox, world⟩ ↔
      ExecRequirements world actor sandbox vm := by
  unfold execExpected ExecRequirements
  cases found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => simp
  | some record =>
      by_cases owner : authorized (actorView actor) record.owner.val = true
      · cases allowed : execAllowed record sandbox vm <;>
          simp [owner, ← actor_authorized_iff, ← exec_allowed_iff, allowed]
      · simp [owner, ← actor_authorized_iff]

theorem file_expected_success_iff (write : Bool) (world : World) (actor : Actor)
    (sandbox : Sid) (generation : U64) :
    fileExpected write world actor sandbox generation =
      .Ok ⟨if write then .FileWrite actor sandbox generation
        else .FileRead actor sandbox generation, world⟩ ↔
      FileRequirements world actor sandbox generation := by
  unfold fileExpected FileRequirements
  cases found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => simp
  | some record =>
      by_cases phase : record.phase = .Running
      · by_cases gen : generation = record.generation
        · by_cases owner : authorized (actorView actor) record.owner.val = true
          · cases allowed : fileAllowed record sandbox record.generation <;>
              simp [phase, gen, owner, ← actor_authorized_iff, ← file_allowed_iff, allowed]
          · simp [phase, gen, owner, ← actor_authorized_iff]
        · simp [phase, gen]
      · simp [phase]

/-- A universal implementation theorem, including exact identity and unchanged
    whole World. No authorization/capability conclusion is supplied as a premise. -/
theorem exec_success_iff (world : World) (actor : Actor) (sandbox : Sid) (vm : Vm) :
    sandbox_controller_core.auxiliary.handle_exec world actor sandbox vm =
      .ok (.Ok ⟨.Exec actor sandbox, world⟩) ↔ ExecRequirements world actor sandbox vm := by
  rw [exec_refines]
  simpa using exec_expected_success_iff world actor sandbox vm

theorem file_read_success_iff (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) :
    sandbox_controller_core.auxiliary.handle_file_read world actor sandbox generation =
      .ok (.Ok ⟨.FileRead actor sandbox generation, world⟩) ↔
      FileRequirements world actor sandbox generation := by
  rw [file_read_refines]
  simpa using file_expected_success_iff false world actor sandbox generation

theorem file_write_success_iff (world : World) (actor : Actor) (sandbox : Sid) (generation : U64) :
    sandbox_controller_core.auxiliary.handle_file_write world actor sandbox generation =
      .ok (.Ok ⟨.FileWrite actor sandbox generation, world⟩) ↔
      FileRequirements world actor sandbox generation := by
  rw [file_write_refines]
  simpa using file_expected_success_iff true world actor sandbox generation

end AccessProofs
