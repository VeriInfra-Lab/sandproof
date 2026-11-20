import SandboxModel.Lifecycle.PauseReceipt.Conditions
import SandboxModel.Lifecycle.PauseDispatch.ReceiptPair
import SandboxModel.Lifecycle.PausePublication.Guards

namespace SandboxModel.Lifecycle.PauseReceipt
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

deriving instance DecidableEq for model.VmStatus
deriving instance DecidableEq for model.EffectOutcome

inductive PauseCommand : model.Command → Prop
  | stop (sandbox : ids.SandboxId) (epoch generation : U64) (vm : ids.VmId) :
      PauseCommand (.StopVm sandbox epoch generation vm)
  | commit (sandbox : ids.SandboxId) (epoch generation : U64)
      (snapshot : ids.SnapshotId) (manifest : model.SnapshotManifestRef) :
      PauseCommand (.CommitSnapshot sandbox epoch generation snapshot manifest)

def stopObserved (world : model.World) (sandbox : ids.SandboxId)
    (generation : U64) (vm : ids.VmId) : Bool :=
  match tableLookup vm world.vms.entries.val with
  | none => false
  | some row => decide (row.sandbox = sandbox ∧ row.generation = generation ∧
      (row.status = .Stopped ∨ row.status = .Reclaimed))

def commitObserved (world : model.World) (sandbox : ids.SandboxId)
    (generation : U64) (snapshot : ids.SnapshotId) (manifest : model.SnapshotManifestRef) : Bool :=
  match tableLookup snapshot world.snapshots.entries.val with
  | none => false
  | some row => decide (row.sandbox = sandbox ∧ row.source_generation = generation ∧
      row.manifest = manifest ∧ row.stage = .Durable ∧ row.immutable = true)

def observed (world : model.World) (command : model.Command) : Bool :=
  match command with
  | .StopVm sandbox _ generation vm => stopObserved world sandbox generation vm
  | .CommitSnapshot sandbox _ generation snapshot manifest =>
      commitObserved world sandbox generation snapshot manifest
  | _ => false

/-- This observes the original fields only. It does not authenticate a physical
exit, durable bytes, or the payload of a reported result digest. -/
def ResultObserved (world : model.World) (command : model.Command) : Prop :=
  match command with
  | .StopVm sandbox _ generation vm =>
    ∃ row, tableLookup vm world.vms.entries.val = some row ∧
      row.sandbox = sandbox ∧ row.generation = generation ∧
      (row.status = .Stopped ∨ row.status = .Reclaimed)
  | .CommitSnapshot sandbox _ generation snapshot manifest =>
    ∃ row, tableLookup snapshot world.snapshots.entries.val = some row ∧
      row.sandbox = sandbox ∧ row.source_generation = generation ∧
      row.manifest = manifest ∧ row.stage = .Durable ∧ row.immutable = true
  | _ => False

def outcomeMatches (world : model.World) (command : model.Command)
    (outcome : model.EffectOutcome) : Bool :=
  match outcome with
  | .Succeeded _ => observed world command
  | .Failed _ => !observed world command

noncomputable section
local instance receiptModelEffectId : DecidableEq model.EffectId := Classical.decEq _

def FirstUnresolved (world : model.World) (id : model.EffectId) : Prop :=
  tableLookup id world.effects.entries.val = some .Dispatched ∨
  tableLookup id world.effects.entries.val = some .OutcomeUnknown

def FirstPauseReceipt (world : model.World) (id : model.EffectId) : Prop :=
  PauseCommand id.logical.command ∧ FirstUnresolved world id

def actualAccepts (world : model.World) (id : model.EffectId)
    (outcome : model.EffectOutcome) (decision : effect.EffectDecision) : Prop :=
  effect.receipt_pause world id outcome = .ok (some (.Ok decision))

def expectedOutcome (world : model.World) (id : model.EffectId)
    (outcome : model.EffectOutcome) : Prop :=
  outcomeMatches world id.logical.command outcome = true

end
end SandboxModel.Lifecycle.PauseReceipt
