import SandboxModel.Lifecycle.Protocol.RenewalBindings
import SandboxModel.Lifecycle.Protocol.RenewalValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.renewal
open SandboxModel.Stage1Protocol.Renewal
namespace SandboxModel.Stage1Protocol.RenewalValues

theorem normal_shape : renewal_valid exactResult = .ok true := by
  simp [shape_refines, legal_exact]

theorem normal_request : renewal_matches exactResult intent = .ok true := by
  exact (matches_accepts_iff exactResult intent).mpr ⟨rfl, legal_exact⟩

theorem normal_confirmation : renewal_confirmed_matches ⟨extended 201#i64⟩ (extended 201#i64) = .ok true := by
  simp [confirmed_refines, ConfirmedMatches, legal_extension]

theorem changed_retention_rejected : renewal_matches exactResult changedRetention = .ok false := by
  simp [matches_refines, Matches, exactResult, changed_request_is_distinct]

theorem changed_deadline_rejected :
    renewal_confirmed_matches ⟨extended 200#i64⟩ (extended 201#i64) = .ok false := by
  simp [confirmed_refines, ConfirmedMatches, changed_deadline_is_distinct]

theorem lower_extension_rejected : renewal_valid (extended 199#i64) = .ok false := by
  simp [shape_refines, lower_extension_is_invalid]

theorem first_version_rejected : renewal_valid firstVersion = .ok false := by
  simp [shape_refines, first_version_is_invalid]

theorem distinct_retention_window_accepted :
    exactResult.request.request.binding.key.window ≠ exactResult.request.retention.window ∧
      renewal_valid exactResult = .ok true := by
  exact ⟨by decide, normal_shape⟩

end SandboxModel.Stage1Protocol.RenewalValues
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.normal_shape
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.normal_request
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.normal_confirmation
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.changed_retention_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.changed_deadline_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.lower_extension_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.first_version_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.distinct_retention_window_accepted
