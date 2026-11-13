import SandboxModel.Lifecycle.Protocol.SourceLive.Spec
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofSourceLiveRust
namespace SandboxModel.Stage1Protocol.SourceLive
open sandbox_controller_core

namespace Values

def life : Lifetime := ⟨some 10#i64, 100#i64, 200#i64, 20#i64, 4096#u64⟩
def native : Native :=
  ⟨7#u64, .Running, false, 2#u64, 3#u64, some 5#u64, none, some life, alloc.vec.Vec.new _⟩
def localBinding : Local := ⟨11#u64, 7#u64, 13#u64⟩
def atDeadline : I64 := 100#i64
def afterStartup : Native :=
  { native with lifetime := some { life with started_at_ms := some 21#i64 } }
def futureStart : Native :=
  { native with lifetime := some { life with started_at_ms := some 19#i64 } }
def negativeClock : Native :=
  { native with lifetime := some { life with started_at_ms := some (-2)#i64 } }
def permissiveBoundary : Native :=
  { native with
    owner := 0#u64
    current_vm := some 0#u64
    lifetime := some { life with
      started_at_ms := some (-1)#i64
      startup_deadline_ms := (-1)#i64
      max_deadline_ms := (-2)#i64
      disk_bytes := 0#u64 } }
def boundaryLocal : Local := ⟨0#u64, 0#u64, 0#u64⟩

end Values

end SandboxModel.Stage1Protocol.SourceLive
