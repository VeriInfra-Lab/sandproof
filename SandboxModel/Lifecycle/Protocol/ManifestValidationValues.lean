import SandboxModel.Lifecycle.Protocol.ManifestValidationModel
import SandboxModel.Lifecycle.Manifest.ManifestValues
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation.Values
open SandboxModel.Stage1Protocol.ManifestChecks
open SandboxModel.Stage1Protocol.ManifestProfile
open SandproofFoundationsRust.IdentifierLibrary

theorem represented_profile : Representable profile := by
  intro drive member
  have same : drive = ⟨"rootfs", "r"⟩ := by simpa [profile] using member
  subst drive
  simp [StringRepresentable, stringBytes]
  change (1 : Nat) ≤ Usize.max
  scalar_tac

theorem original_binding_valid : BindingValid original := by
  simp [BindingValid, HasVersion, original, sandproof_protocol.lifecycle.PROTOCOL_VERSION]

end SandboxModel.Stage1Protocol.ManifestValidation.Values

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Values.represented_profile
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Values.original_binding_valid
