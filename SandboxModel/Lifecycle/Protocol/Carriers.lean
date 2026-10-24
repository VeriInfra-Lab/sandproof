import SandproofStage1Carriers.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1CarriersRust
namespace SandboxModel.Stage1Protocol
theorem capture_value_exact (value : U64) : capture_value value = .ok value := by rfl
theorem capture_constructor_exact (value : U64) :
    capture_from_u64 value = .ok (if value = 0#u64 then
      .Err sandproof_protocol.internal.InternalError.InvalidCounter else .Ok value) := by
  simp only [capture_from_u64,
    sandproof_protocol.lifecycle.identity.CaptureId.Insts.CoreConvertTryFromU64InternalError.try_from,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreConvertTryFromU64InternalError.try_from]
  split <;> simp
theorem capture_zero_refused : capture_from_u64 0#u64 =
    .ok (.Err sandproof_protocol.internal.InternalError.InvalidCounter) := by
  simpa using capture_constructor_exact 0#u64
theorem capture_one_accepted : capture_from_u64 1#u64 = .ok (.Ok 1#u64) := by
  simpa using capture_constructor_exact 1#u64
theorem digest_all_bytes_exact (bytes : Array U8 32#usize) :
    digest_roundtrip bytes = .ok bytes := by
  simp [digest_roundtrip, sandproof_protocol.lifecycle.digest.ContentDigest.from_sha256,
    sandproof_protocol.lifecycle.digest.ContentDigest.into_bytes]
theorem capture_max_exhausted : capture_next 18446744073709551615#u64 =
    .ok (.Err sandproof_protocol.internal.InternalError.InvalidCounter) := by
  have exhausted : U64.checked_add 18446744073709551615#u64 1#u64 = none := by decide
  simp [capture_next, sandproof_protocol.lifecycle.identity.CaptureId.checked_next,
    sandproof_protocol.internal.PositiveCounter.checked_next,
    sandproof_protocol.internal.PositiveCounter.get, Aeneas.Std.lift, exhausted]
end SandboxModel.Stage1Protocol
#audit_controller_proof SandboxModel.Stage1Protocol.capture_value_exact
#audit_controller_proof SandboxModel.Stage1Protocol.capture_constructor_exact
#audit_controller_proof SandboxModel.Stage1Protocol.capture_zero_refused
#audit_controller_proof SandboxModel.Stage1Protocol.capture_one_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.digest_all_bytes_exact
#audit_controller_proof SandboxModel.Stage1Protocol.capture_max_exhausted
