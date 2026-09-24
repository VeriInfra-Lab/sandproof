import SandboxModel.RustRefinement.PublicationAdmissionChecks

namespace SandboxModel.RustRefinement

-- The pinned Lean compiler shares these dependent matchers across handlers.
-- These lemmas consume a lookup equation directly instead of splitting later guards.
theorem publication_sandbox_some {α : Sort u} {value : Option SandboxRecord}
    {record : SandboxRecord} (found : value = some record)
    (missing : value = none → α) (present : (r : SandboxRecord) → value = some r → α) :
    Controller.handlePause.match_3 (fun _ => α) value missing present = present record found := by
  cases found
  rfl

theorem publication_sandbox_none {α : Sort u} {value : Option SandboxRecord}
    (found : value = none)
    (missing : value = none → α) (present : (r : SandboxRecord) → value = some r → α) :
    Controller.handlePause.match_3 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

theorem publication_operation_some {α : Sort u} {value : Option Operation}
    {op : Operation} (found : value = some op)
    (missing : value = none → α) (present : (r : Operation) → value = some r → α) :
    Controller.handleCompensation.match_1 (fun _ => α) value missing present = present op found := by
  cases found
  rfl

theorem publication_operation_none {α : Sort u} {value : Option Operation}
    (found : value = none)
    (missing : value = none → α) (present : (r : Operation) → value = some r → α) :
    Controller.handleCompensation.match_1 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

theorem publication_ledger_some {α : Sort u} {value : Option OperationLedgerRecord}
    {ledger : OperationLedgerRecord} (found : value = some ledger)
    (missing : value = none → α) (present : (r : OperationLedgerRecord) → value = some r → α) :
    Controller.handleReplay.match_1 (fun _ => α) value missing present = present ledger found := by
  cases found
  rfl

theorem publication_ledger_none {α : Sort u} {value : Option OperationLedgerRecord}
    (found : value = none)
    (missing : value = none → α) (present : (r : OperationLedgerRecord) → value = some r → α) :
    Controller.handleReplay.match_1 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

end SandboxModel.RustRefinement
