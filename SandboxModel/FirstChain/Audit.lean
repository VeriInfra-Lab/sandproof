import SandboxModel.FirstChain.Scenarios
import Lean.Util.CollectAxioms
import Lean.Elab.Command

/-! Enumerate this batch's actual proofs; not an implementation-coverage claim. -/
open Lean Elab Command

run_cmd do
  let names : Array Name := #[
    ``SandboxModel.FirstChain.empty_safe,
    ``SandboxModel.FirstChain.advance_safe,
    ``SandboxModel.FirstChain.advance_fact_mono,
    ``SandboxModel.FirstChain.advance_terminal_mono,
    ``SandboxModel.FirstChain.potential_has_responsibility,
    ``SandboxModel.FirstChain.release_requires_quiescence,
    ``SandboxModel.FirstChain.terminal_blocks_receive,
    ``SandboxModel.FirstChain.terminal_blocks_acquire,
    ``SandboxModel.FirstChain.fence_blocks_effect,
    ``SandboxModel.FirstChain.no_second_claim,
    ``SandboxModel.FirstChain.initial_invariant,
    ``SandboxModel.FirstChain.afterAction_invariant,
    ``SandboxModel.FirstChain.step_invariant,
    ``SandboxModel.FirstChain.reachable_invariant,
    ``SandboxModel.FirstChain.reachable_responsibility,
    ``SandboxModel.FirstChain.reachable_release,
    ``SandboxModel.FirstChain.other_binding_unchanged,
    ``SandboxModel.FirstChain.restart_not_dispatch_ready,
    ``SandboxModel.FirstChain.step_terminal_mono,
    ``SandboxModel.FirstChain.crash_center_preserves_records,
    ``SandboxModel.FirstChain.crash_node_preserves_resources,
    ``SandboxModel.FirstChain.startup_example,
    ``SandboxModel.FirstChain.pending_is_not_failure,
    ``SandboxModel.FirstChain.live_lease_blocks_release,
    ``SandboxModel.FirstChain.completed_cleanup_example,
    ``SandboxModel.FirstChain.terminal_first_rejects_late_create,
    ``SandboxModel.FirstChain.stopped_does_not_mean_cleaned,
    ``SandboxModel.FirstChain.lost_reply_and_crash_reachable,
    ``SandboxModel.FirstChain.confirmed_write_reply_may_be_lost,
    ``SandboxModel.FirstChain.premature_release_counterexample,
    ``SandboxModel.FirstChain.invented_ready_counterexample,
    ``SandboxModel.FirstChain.original_ready_recoverable,
    ``SandboxModel.FirstChain.receipt_is_not_admission,
    ``SandboxModel.FirstChain.retained_reference_blocks_release]
  let allowed : Array Name := #[``propext, ``Classical.choice, ``Quot.sound]
  for name in names do
    match ← getConstInfo name with
    | .thmInfo _ => pure ()
    | _ => throwError "Expected theorem: {name}"
    let dependencies ← collectAxioms name
    for dependency in dependencies do
      unless allowed.contains dependency do
        throwError "Unapproved dependency {dependency} in {name}"
  logInfo m!"Checked {names.size} first-chain model theorems; no Rust refinement claimed."
