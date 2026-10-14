import SandboxModel.Local.Slots.Model
import SandproofNetwork.Funs

namespace SandboxModel.Local.Slots
open Aeneas Aeneas.Std SandproofNetworkRust

abbrev RustBinding := sandbox_controller_core.maintenance.slots.HostBinding
abbrev RustRow := sandbox_controller_core.maintenance.slots.HostSlotRecord
abbrev RustInput := sandbox_controller_core.maintenance.slots.SlotInput
abbrev RustReject := sandbox_controller_core.maintenance.slots.SlotReject
abbrev RustPhase := sandbox_controller_core.maintenance.slots.HostSlotPhase

def bindingView (value : RustBinding) : Binding :=
  ⟨value.owner.val, value.sandbox.val, value.generation.val, value.vm.val, value.network.val⟩
def phaseView : RustPhase → Phase
  | .Occupied => .occupied | .Releasing => .releasing | .Free => .free
def rowView (row : RustRow) : Row := ⟨bindingView row.binding, phaseView row.phase⟩
def inputView : RustInput → Input
  | .Bind position expected binding => .bind position.val (expected.map UScalar.val) (bindingView binding)
  | .BeginRelease position binding => .beginRelease position.val (bindingView binding)
  | .ConfirmRelease position binding => .confirmRelease position.val (bindingView binding)
  | .CheckOccupied position binding => .checkOccupied position.val (bindingView binding)
def rejectView : RustReject → Reject
  | .OutOfRange => .outOfRange | .BindingMismatch => .bindingMismatch
  | .NotFree => .notFree | .NotOccupied => .notOccupied | .NotReleasing => .notReleasing
  | .StaleIdentity => .staleIdentity | .IdentityAlreadyBound => .identityAlreadyBound
def resultView : core.result.Result RustRow RustReject → Except Reject Row
  | .Ok row => .ok (rowView row) | .Err error => .error (rejectView error)

end SandboxModel.Local.Slots
