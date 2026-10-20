import SandboxModel.CenterStore.Lua.Scripts

namespace Sandproof.CenterStore.Lua

set_option maxRecDepth 8192 in
theorem skip_source_parses : (parseChars skipReaffirmChars).isSome = true := by cbv

set_option maxRecDepth 8192 in
theorem early_source_parses : (parseChars earlyWriteChars).isSome = true := by cbv

def skipProgram : List Stmt := (parseChars skipReaffirmChars).getD []
def earlyProgram : List Stmt := (parseChars earlyWriteChars).getD []

set_option maxRecDepth 8192 in
theorem skip_rejected_by_same_contract :
    (paths skipProgram).all casPathContract = false := by cbv

set_option maxRecDepth 8192 in
theorem early_rejected_by_same_contract :
    (paths earlyProgram).all casPathContract = false := by cbv

set_option maxRecDepth 8192 in
theorem early_exceeds_original_write_budget : maxWrites earlyProgram = 2 := by cbv

theorem early_violates_original_write_budget : ¬maxWrites earlyProgram ≤ 1 := by
  rw [early_exceeds_original_write_budget]
  decide

end Sandproof.CenterStore.Lua
