"""Single source of truth for the safety properties of the Lean model.

Each property records:
  - `lean`: the Lean definition/theorem that establishes it;
  - `dimension`: the invariant family it belongs to;
  - `monitorability`: whether a trace can witness a violation, and how;
  - `rule`: the checker rule that enforces it, if one is implemented.

`tools/coverage.py` renders this into the coverage report. Keep this file in
sync with `SandboxModel/Safety.lean`; it is intentionally data, not logic.
"""
from dataclasses import dataclass, field
from typing import Optional

# Monitorability classes.
MONITORED = "monitored"                      # rule implemented and enforced
MONITORABLE = "monitorable"                   # observable, rule not yet wired
NEEDS_INSTRUMENTATION = "needs_instrumentation"  # needs store/node events
ADAPTER = "adapter"                          # outside the model boundary
LIVENESS = "liveness"                        # not a safety property
STATIC = "static"                            # holds by construction/type
MODEL_GAP = "model_gap"                      # detected, not yet formalized/proved


@dataclass(frozen=True)
class Property:
    name: str
    lean: str
    dimension: str
    monitorability: str
    description: str
    rule: Optional[str] = None


PROPERTIES: list[Property] = [
    # --- state invariants (Safe) ---
    Property("ResourceOwnership", "ResourceOwnership", "resources", NEEDS_INSTRUMENTATION,
             "every VM/fs/snapshot/slot is owned by the sandbox that references it"),
    Property("WorkspaceBindingValid", "WorkspaceBindingValid", "workspace", NEEDS_INSTRUMENTATION,
             "a sandbox's workspace points back to the same sandbox and owner"),
    Property("WorkspaceApiAccessFixed", "WorkspaceApiAccessFixed", "workspace", STATIC,
             "the managed API access mode is the single constructor managedReadWrite"),
    Property("CurrentResourcesValid", "CurrentResourcesValid", "resources", NEEDS_INSTRUMENTATION,
             "currentVm/netSlot references cannot point at another sandbox's resource"),
    Property("UniqueExecution", "UniqueExecution", "execution", MONITORED,
             "at most one executing VM per sandbox", rule="unique_execution"),
    Property("UniqueActiveSlot", "UniqueActiveSlot", "network", MONITORED,
             "at most one active network slot per sandbox", rule="unique_active_slot"),
    Property("ActiveSlotValid", "ActiveSlotValid", "network", MONITORED,
             "an active route is configured and named by the sandbox record", rule="active_slot_valid"),
    Property("DurableSnapshotsImmutable", "DurableSnapshotsImmutable", "snapshot", MONITORED,
             "a durable snapshot is immutable", rule="durable_snapshot_immutable"),
    Property("AuthorityShape", "AuthorityShape", "authority", NEEDS_INSTRUMENTATION,
             "desired/effective authority match the lifecycle phase"),
    Property("FenceWellFormed", "FenceWellFormed", "fence", MONITORED,
             "a worker fence never exceeds the record epoch and never drops terminal", rule="fence_well_formed"),
    Property("FenceClosesAuthority", "FenceClosesAuthority", "fence", NEEDS_INSTRUMENTATION,
             "an installed revocation fence closes effective authority"),
    Property("EffectiveRuntimeValid", "EffectiveRuntimeValid", "authority", NEEDS_INSTRUMENTATION,
             "non-closed, non-paused authority implies an executing VM and active slot"),
    Property("StableStatesQuiescent", "StableStatesQuiescent", "lifecycle", MONITORED,
             "paused/destroyed states have no executing VM and no active slot", rule="stable_states_quiescent"),
    Property("TerminalConsistent", "TerminalConsistent", "lifecycle", MONITORED,
             "terminal holds exactly in revoking/destroyed", rule="kill_irreversible"),
    Property("PausedSnapshotValid", "PausedSnapshotValid", "snapshot", MONITORED,
             "a paused sandbox has a durable, immutable snapshot bound to it", rule="pause_requires_snapshot"),
    Property("RestoreSourceValid", "RestoreSourceValid", "snapshot", MONITORED,
             "a resumed starting state remembers one valid durable source", rule="resume_requires_snapshot"),
    Property("CurrentOperationValid", "CurrentOperationValid", "operation-accounting", NEEDS_INSTRUMENTATION,
             "the current operation's epoch/kind/status agree with the phase"),

    # --- single-state conclusions (ProtocolSecurity) ---
    Property("AppliedFilePolicyValid", "AppliedFilePolicyValid", "file-policy", NEEDS_INSTRUMENTATION,
             "a worker policy receipt names the same sandbox/workspace/policy and no future generation"),
    Property("RunningRequiresAppliedFilePolicy", "RunningRequiresAppliedFilePolicy", "file-policy", MONITORED,
             "Running requires an applied policy receipt for the current generation",
             rule="running_requires_applied_policy"),
    Property("OperationLedgerValid", "OperationLedgerValid", "operation-accounting", NEEDS_INSTRUMENTATION,
             "ledger keys agree with the stored sandbox/operation id"),
    Property("CurrentOperationInLedger", "CurrentOperationInLedger", "operation-accounting", NEEDS_INSTRUMENTATION,
             "the current operation has a persisted ledger record"),
    Property("EffectIdentityValid", "EffectIdentityValid", "effect-accounting", STATIC,
             "an effect id is exactly the deterministic id of its command"),
    Property("ReclaimedResourcesNotCurrent", "ReclaimedResourcesNotCurrent", "cleanup", MONITORED,
             "a reclaimed VM/slot is never the current resource of its sandbox",
             rule="reclaimed_not_current"),

    # --- step properties (ProtocolStepSecurity) ---
    Property("eventAuthorized", "ProtocolStepSecurity.eventAuthorized", "admission", MONITORED,
             "each user-facing event is authorized for the sandbox owner", rule="event_authorized"),
    Property("egressEnforced", "ProtocolStepSecurity.egressEnforced", "admission", MONITORED,
             "egress is only allowed to an endpoint in the active allowlist", rule="egress_enforced"),
    Property("fileWorkspaceBound", "ProtocolStepSecurity.fileWorkspaceBound", "workspace", MONITORED,
             "file events target the sandbox's bound workspace", rule="file_workspace_bound"),
    Property("effectAdmission", "ProtocolStepSecurity.effectAdmission", "effect-accounting", MONITORED,
             "dispatch requires committed, ack requires dispatched and an exact result",
             rule="effect_admission"),
    Property("effectResultStable", "ProtocolStepSecurity.effectResultStable", "effect-accounting", MONITORED,
             "an acknowledged effect keeps its result", rule="effect_admission"),
    Property("operationResultStable", "ProtocolStepSecurity.operationResultStable", "operation-accounting", MONITORED,
             "a final operation result is never rewritten", rule="operation_stable"),
    Property("operationIdentityStable", "ProtocolStepSecurity.operationIdentityStable", "operation-accounting", MONITORED,
             "kind/epoch/input under an operation key are never rewritten", rule="operation_stable"),
    Property("terminalMonotone", "ProtocolStepSecurity.terminalMonotone", "lifecycle", MONITORED,
             "terminal never goes from true back to false", rule="terminal_monotone"),
    Property("workspaceStable", "ProtocolStepSecurity.workspaceStable", "workspace", MONITORED,
             "the workspace identity of a sandbox never changes", rule="workspace_stable"),
    Property("workspaceAccessStable", "ProtocolStepSecurity.workspaceAccessStable", "workspace", STATIC,
             "the managed workspace access mode never changes"),
    Property("filePolicyStable", "ProtocolStepSecurity.filePolicyStable", "file-policy", MONITORED,
             "the file policy identity of a sandbox never changes", rule="file_policy_stable"),
    Property("snapshotManifestStable", "ProtocolStepSecurity.snapshotManifestStable", "snapshot", MONITORED,
             "a snapshot's manifest identity never changes", rule="snapshot_manifest_stable"),
    Property("epochMonotone", "ProtocolStepSecurity.epochMonotone", "fence", MONITORED,
             "the sandbox epoch never decreases", rule="epoch_monotone"),
    Property("generationMonotone", "ProtocolStepSecurity.generationMonotone", "fence", MONITORED,
             "the sandbox generation never decreases", rule="generation_monotone"),
    Property("fenceMonotone", "ProtocolStepSecurity.fenceMonotone", "fence", NEEDS_INSTRUMENTATION,
             "the worker fence epoch/terminal never regresses"),
    Property("cleanupAdmission", "ProtocolStepSecurity.cleanupAdmission", "cleanup", NEEDS_INSTRUMENTATION,
             "cleanup requires the exact stopped/inactive old resource and not-current"),
    Property("runningRequiresStart", "AuthorityShape + RestoreSourceValid", "lifecycle", MONITORED,
             "a running observation is preceded by a successful create or resume", rule="running_requires_start"),

    Property("NoStaleIndexReferences", "Accounting.Consistent", "accounting", MONITORED,
             "index/ledger entries reference exactly the live sandboxes (formalized; runtime#3566)",
             rule="stale_index"),
    Property("CreateCompensation", "Compensation.abortCreate_reclaims", "lifecycle", MONITORED,
             "an aborted/failed create reclaims its prepared VM/slot (E2B#1498)",
             rule="node_orphan"),
    Property("NoOrphanOnNode", "Compensation.abortCreate_reclaims", "lifecycle", MONITORED,
             "no sandbox is held by the node without a running-store record", rule="node_orphan"),
    Property("LifecycleCompletion", "Liveness.can_reach_destroyed", "lifecycle", LIVENESS,
             "every phase can reach destroyed; runtime traces are checked for completion",
             rule="lifecycle_completion"),

    # --- explicitly outside the safety boundary ---
    Property("adapterFidelity", "adapter contract", "adapter", ADAPTER,
             "Linux applies the policy and storage restores the manifest faithfully"),
    Property("liveness", "not proved", "liveness", LIVENESS,
             "started operations eventually reach a terminal state"),
]


def by_rule() -> dict[str, Property]:
    return {p.rule: p for p in PROPERTIES if p.rule}


def report() -> str:
    lines = ["| property | dimension | monitorability | rule |", "|---|---|---|---|"]
    for p in PROPERTIES:
        lines.append(
            f"| `{p.name}` | {p.dimension} | {p.monitorability} | {p.rule or '—'} |")
    return "\n".join(lines)


if __name__ == "__main__":
    print(report())
