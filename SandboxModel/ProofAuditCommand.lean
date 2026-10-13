import Lean.Util.CollectAxioms
import Lean.Elab.Command

/-! Shared dependency audit. This checks listed theorems, not missing goals. -/
open Lean Elab Command

syntax (name := auditControllerProof) "#audit_controller_proof " ident : command

@[command_elab auditControllerProof]
def elabAuditControllerProof : CommandElab := fun stx => do
  let declName := stx[1].getId
  let info ← getConstInfo declName
  match info with
  | .thmInfo _ => pure ()
  | _ => throwErrorAt stx "Expected a theorem, found {declName}"
  let allowed : Array Name := #[``propext, ``Classical.choice, ``Quot.sound]
  let dependencies ← collectAxioms declName
  for dependency in dependencies do
    unless allowed.contains dependency do
      throwErrorAt stx "Proof {declName} depends on unapproved assumption {dependency}"
  logInfo m!"Checked {declName}: {dependencies.size} standard logical dependencies"
