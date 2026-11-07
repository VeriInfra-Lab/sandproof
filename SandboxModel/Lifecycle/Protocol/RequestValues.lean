import SandproofStage1Request.Funs
import SandboxModel.Lifecycle.Protocol.Model
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1RequestRust
namespace SandboxModel.Stage1Protocol.Request
abbrev Node := sandproof_protocol.internal.NodeIdentity
abbrev Binding := sandproof_protocol.lifecycle.identity.OperationBinding
abbrev WireRequest := sandproof_protocol.lifecycle.request.LifecycleRequest
abbrev ValidationError := sandproof_protocol.lifecycle.request.ValidationError

def node : Node := ⟨"deployment", "node", 1#u64⟩
def binding : Binding := {
  protocol_version := 4#u16, node := node, session := "session",
  key := ⟨"deployment", "owner", 1#u64, 1#u64⟩,
  expected := .Version 1#u64 }
def digest : Array U8 32#usize := Array.from (List.replicate 32 (0#u8)) (by simp)
def resume (source target : U64) : WireRequest := ⟨binding, .Resume source 1#u64 target digest⟩
def runResume (source target : U64) := request_validate (resume source target) node
def unload (capture : U64) : WireRequest := ⟨binding, .Unload 1#u64 capture⟩

end SandboxModel.Stage1Protocol.Request
