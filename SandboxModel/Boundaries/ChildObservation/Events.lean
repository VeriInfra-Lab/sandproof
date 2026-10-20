import Aeneas

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.ChildObservation

/-! Per-invocation observations; `handedOff` is historical, not queue membership.
These events do not freeze the concurrently changing OS, clock or reaper. -/

inductive Event (K L S E T : Type) where
  | poll (child : K) (reply : core.result.Result (Option S) E)
  | kill (child : K) (reply : core.result.Result Unit E)
  | closeStdin (child : K)
  | closeStdout (child : K)
  | closeStderr (child : K)
  | now (instant : T)
  | deadline (start finish : T)
  | pause
  | handedOff (child : K) (slot : L)
  | timeoutError (error : E)

end SandboxModel.Boundaries.ChildObservation
