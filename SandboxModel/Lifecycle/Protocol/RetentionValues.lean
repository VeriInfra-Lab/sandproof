import SandboxModel.Lifecycle.Protocol.RequestValues
import SandboxModel.Lifecycle.Protocol.RetentionModel

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.identity
open sandproof_protocol.lifecycle.request sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.renewal
namespace SandboxModel.Stage1Protocol.Retention

def checkpoint : RenewalConfirmation := {
  request := {
    request := { Request.renew 1#u64 200#i64 false with
      binding := { Request.binding with key := { Request.binding.key with window := 2#u64 } } },
    accepted_version := 2#u64, center := ⟨"deployment", "center"⟩, retention := ⟨1#u16, 1#u64⟩ },
  effective_end_at_ms := 200#i64 }

def otherCenter : AcceptedLifecycleIntent :=
  { checkpoint.request with center := ⟨"deployment", "another-center"⟩ }

end SandboxModel.Stage1Protocol.Retention
