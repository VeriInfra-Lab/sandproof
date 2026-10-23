import SandboxModel.Boundaries.ProtectedDirectory.Elaboration

run_cmd Lean.Elab.Command.liftTermElabM do
  Lean.Meta.withLocalDeclD `p (Lean.mkSort Lean.Level.zero) fun p => do
    let found ← Lean.Meta.synthInstance? (Lean.mkApp (Lean.mkConst ``Decidable) p)
    if found.isSome then throwError "unexpected arbitrary Prop instance before namespaces"

namespace SandproofProtectedDirectoryRust
run_cmd Lean.Elab.Command.liftTermElabM do
  Lean.Meta.withLocalDeclD `p (Lean.mkSort Lean.Level.zero) fun p => do
    let found ← Lean.Meta.synthInstance? (Lean.mkApp (Lean.mkConst ``Decidable) p)
    unless found.isSome do throwError "correct namespace did not activate its scope"
end SandproofProtectedDirectoryRust

namespace SandproofProtectedDirectoryBadRust
run_cmd Lean.Elab.Command.liftTermElabM do
  Lean.Meta.withLocalDeclD `p (Lean.mkSort Lean.Level.zero) fun p => do
    let found ← Lean.Meta.synthInstance? (Lean.mkApp (Lean.mkConst ``Decidable) p)
    unless found.isSome do throwError "mutant namespace did not activate its scope"
end SandproofProtectedDirectoryBadRust

namespace OutsideGeneratedNamespaces
run_cmd Lean.Elab.Command.liftTermElabM do
  Lean.Meta.withLocalDeclD `p (Lean.mkSort Lean.Level.zero) fun p => do
    let found ← Lean.Meta.synthInstance? (Lean.mkApp (Lean.mkConst ``Decidable) p)
    if found.isSome then throwError "generated scoped instance leaked outside its namespace"
end OutsideGeneratedNamespaces
