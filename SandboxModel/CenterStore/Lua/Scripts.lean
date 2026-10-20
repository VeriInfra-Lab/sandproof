import SandboxModel.CenterStore.Lua.RawSource
import SandboxModel.CenterStore.Lua.Paths
import SandboxModel.CenterStore.Lua.Calls

namespace Sandproof.CenterStore.Lua

-- The larger recursion limit is local to certificates for these 700/1204-byte
-- constants. It is not a business-proof search or a change to kernel checking.
set_option maxRecDepth 8192 in
theorem read_source_parses : (parseChars readChars).isSome = true := by cbv

set_option maxRecDepth 8192 in
theorem cas_source_parses : (parseChars casChars).isSome = true := by cbv

def readProgram : List Stmt := (parseChars readChars).getD []
def casProgram : List Stmt := (parseChars casChars).getD []

theorem read_source_binding : parseChars readChars = some readProgram := by
  have h := read_source_parses
  cases he : parseChars readChars with
  | none => simp [he] at h
  | some p => simp [readProgram, he]

theorem cas_source_binding : parseChars casChars = some casProgram := by
  have h := cas_source_parses
  cases he : parseChars casChars with
  | none => simp [he] at h
  | some p => simp [casProgram, he]

set_option maxRecDepth 8192 in
theorem read_write_budget : maxWrites readProgram = 0 := by cbv

set_option maxRecDepth 8192 in
theorem cas_write_budget : maxWrites casProgram = 1 := by cbv

theorem read_flow_no_invocation {n} (h : Flow readProgram n) : n = 0 := by
  have hb := flow_bound h
  rw [read_write_budget] at hb
  omega

theorem cas_flow_at_most_one_invocation {n} (h : Flow casProgram n) : n ≤ 1 := by
  have hb := flow_bound h
  rwa [cas_write_budget] at hb

set_option maxRecDepth 8192 in
theorem read_calls_safe : readProgram.all (callsSafe 64) = true := by cbv

set_option maxRecDepth 8192 in
theorem cas_calls_safe : casProgram.all (callsSafe 64) = true := by cbv

def casPathContract (path : Nat × Terminal) : Bool :=
  match path.2 with
  | .code 0 => path.1 == 1
  | .code n => (n == 1 || n == 2 || n == 3 || n == 4) && path.1 == 0
  | _ => false

set_option maxRecDepth 8192 in
theorem cas_paths_contract : (paths casProgram).all casPathContract = true := by cbv

theorem cas_result_contract {n r} (h : ResultFlow casProgram n r) :
    casPathContract (n, r) = true := by
  exact List.all_eq_true.mp cas_paths_contract (n, r) (resultFlow_mem h)

theorem cas_zero_has_one_call {n} (h : ResultFlow casProgram n (.code 0)) : n = 1 := by
  have hc := cas_result_contract h
  simpa [casPathContract] using hc

theorem cas_nonzero_has_no_calls {n code} (hne : code ≠ 0)
    (h : ResultFlow casProgram n (.code code)) : n = 0 := by
  have hc := cas_result_contract h
  simp [casPathContract] at hc
  exact hc.2

end Sandproof.CenterStore.Lua
