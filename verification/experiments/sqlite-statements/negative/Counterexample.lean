import SandboxModel.Boundaries.SqliteStatements.Contract
import SandboxModel.Boundaries.SqliteStatements.Integer
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std
namespace SandboxModel.Boundaries.SqliteStatements.Negative
open SandproofSqliteStatementsRust
open SandboxModel.Boundaries.SqliteStatements.Bindings
open SandboxModel.Boundaries.SqliteStatements.Contract

/-- This is the unedited output of the original translator for the actual
`changed > 1` Rust mutation, not a separately invented Lean wrong program. -/
theorem actual_wrong_update_accepts_zero {C : Type}
    (transaction : rusqlite.transaction.Transaction) (write : journal.AcceptedWrite C)
    (next : I64) (expected : U64) (expectedSql : I64) (sandbox owner : Str)
    (sandboxView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.sandbox = .ok sandbox)
    (ownerView : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref write.owner = .ok owner)
    (converted : journal.as_sql_integer expected = .ok (.Ok expectedSql))
    (prepared : transaction.connection.prepare originalSql = .ok (.Ok 5))
    (called : transaction.connection.run originalSql
      (completeArguments next expectedSql write.sandbox_state sandbox owner) = .ok (.Ok 0#usize)) :
    journal.statement_core.update_existing transaction write next expected = .ok (.Ok ()) := by
  have primitive : rusqlite.Connection.execute originalParams transaction.connection originalSql
      (next, write.sandbox_state, sandbox, owner, expectedSql) = .ok (.Ok 0#usize) := by
    rw [execute_receives_exact_five_values transaction.connection originalSql
      next expectedSql write.sandbox_state sandbox owner prepared]
    exact called
  dsimp only [originalParams, originalSql] at primitive
  simp [journal.statement_core.update_existing,
    rusqlite.transaction.Transaction.Insts.CoreOpsDerefDerefConnection.deref,
    sandproof_protocol.SandboxId.as_str, sandproof_protocol.TenantId.as_str,
    sandboxView, ownerView, converted, primitive,
    core.result.Result.Insts.CoreOpsTry.branch]

def zeroTransaction : rusqlite.transaction.Transaction :=
  ⟨17, ⟨23, fun _ => .ok (.Ok 5), fun _ _ => ⟨41⟩, fun _ _ => .ok (.Ok 0#usize)⟩⟩

def zeroWrite : journal.AcceptedWrite Unit :=
  ⟨"tenant-a", "sandbox-a", "operation-a", some 1#u64, alloc.vec.Vec.new U8,
    alloc.vec.Vec.new U8, alloc.vec.Vec.new _⟩

theorem sandbox_identifier_is_representable :
    SandproofFoundationsRust.IdentifierLibrary.StringRepresentable "sandbox-a" := by
  have length : (SandproofFoundationsRust.IdentifierLibrary.stringBytes "sandbox-a").length = 9 := by decide
  unfold SandproofFoundationsRust.IdentifierLibrary.StringRepresentable
  rw [length]
  scalar_tac

theorem owner_identifier_is_representable :
    SandproofFoundationsRust.IdentifierLibrary.StringRepresentable "tenant-a" := by
  have length : (SandproofFoundationsRust.IdentifierLibrary.stringBytes "tenant-a").length = 8 := by decide
  unfold SandproofFoundationsRust.IdentifierLibrary.StringRepresentable
  rw [length]
  scalar_tac

def sandboxObservation : Str := .from
  (SandproofFoundationsRust.IdentifierLibrary.stringBytes "sandbox-a") sandbox_identifier_is_representable

def ownerObservation : Str := .from
  (SandproofFoundationsRust.IdentifierLibrary.stringBytes "tenant-a") owner_identifier_is_representable

theorem sandbox_identifier_has_an_actual_view :
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "sandbox-a" = .ok sandboxObservation := by
  simp [alloc.string.String.Insts.CoreOpsDerefDerefStr.deref,
    sandbox_identifier_is_representable, sandboxObservation]

theorem owner_identifier_has_an_actual_view :
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref "tenant-a" = .ok ownerObservation := by
  simp [alloc.string.String.Insts.CoreOpsDerefDerefStr.deref,
    owner_identifier_is_representable, ownerObservation]

theorem expected_revision_has_an_actual_conversion :
    journal.as_sql_integer 1#u64 = .ok (.Ok 1#i64) := by
  have bounded : (1 : Int) ≤ I64.max := by scalar_tac
  have cast : UScalar.hcast .I64 1#u64 = 1#i64 := by decide
  simp [SandboxModel.Boundaries.SqliteStatements.Integer.sql_integer_normal_form, bounded, cast]

theorem actual_wrong_update_rejects_the_same_contract :
    ¬ NormalReplyContract @journal.statement_core.update_existing := by
  intro claimed
  have observed := claimed Unit zeroTransaction zeroWrite 2#i64 1#u64 1#i64
    sandboxObservation ownerObservation (.Ok 0#usize)
    sandbox_identifier_has_an_actual_view owner_identifier_has_an_actual_view expected_revision_has_an_actual_conversion (by rfl) (by rfl)
  have actual := actual_wrong_update_accepts_zero zeroTransaction zeroWrite 2#i64 1#u64 1#i64
    sandboxObservation ownerObservation
    sandbox_identifier_has_an_actual_view owner_identifier_has_an_actual_view expected_revision_has_an_actual_conversion (by rfl) (by rfl)
  rw [actual] at observed
  simp [interpretReply] at observed

end SandboxModel.Boundaries.SqliteStatements.Negative

#audit_controller_proof SandboxModel.Boundaries.SqliteStatements.Negative.actual_wrong_update_accepts_zero
#audit_controller_proof SandboxModel.Boundaries.SqliteStatements.Negative.sandbox_identifier_has_an_actual_view
#audit_controller_proof SandboxModel.Boundaries.SqliteStatements.Negative.actual_wrong_update_rejects_the_same_contract

#audit_controller_proof SandboxModel.Boundaries.SqliteStatements.Negative.expected_revision_has_an_actual_conversion

#audit_controller_proof SandboxModel.Boundaries.SqliteStatements.Negative.owner_identifier_has_an_actual_view

#audit_controller_proof SandboxModel.Boundaries.SqliteStatements.Negative.sandbox_identifier_is_representable
#audit_controller_proof SandboxModel.Boundaries.SqliteStatements.Negative.owner_identifier_is_representable
