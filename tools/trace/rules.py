"""Checker rules. Each rule enforces one (or a small cluster of) safety
property and owns its per-sandbox monitor state.

A rule declares `requires`: the observation keys it needs. The checker calls a
rule only with observations that carry its data, so a rule is logically
complete while live instrumentation catches up. Add a rule here; the checker
discovers it from `REGISTRY` (open/closed).
"""
from typing import Optional

from tools.trace import mapper
from tools.trace.trace_model import Finding, Record


class Rule:
    """Monitor rule interface. `state` is per-rule, per-sandbox."""

    name: str = ""
    properties: list[str] = []
    requires: list[str] = []  # observation keys; empty = always applicable

    def applies(self, record: Record) -> bool:
        if self.requires:
            return any(key in record.obs for key in self.requires)
        return True

    def on_event(self, state: dict, record: Record) -> Optional[Finding]:
        return None

    def on_observe(self, state: dict, record: Record) -> Optional[Finding]:
        return None

    def on_finish(self, state: dict, sandbox: str) -> Optional[Finding]:
        """Optional end-of-trace check, for properties about where a trace
        ends (e.g. completion)."""
        return None

    def _finding(self, record: Record, detail: str, subsequence: list[int],
                 inconclusive: bool = False) -> Finding:
        return Finding(
            rule=self.name, sandbox=record.sandbox or "?",
            seq=record.seq, detail=detail, minimal_subsequence=subsequence,
            properties=list(self.properties), inconclusive=inconclusive,
        )


# --------------------------------------------------------------------------
# Lifecycle
# --------------------------------------------------------------------------

class KillIrreversible(Rule):
    """Once a kill is accepted, no later resume/running observation is legal."""
    name = "kill_irreversible"
    properties = ["TerminalConsistent", "step_never_clears_terminal"]

    def on_event(self, state, record):
        if mapper.is_event(record, "acceptKill"):
            result = mapper.result_of(record)
            if result == "success":
                state["kill_seq"] = record.seq
            elif result == "unknown":
                state["kill_unknown_seq"] = record.seq
        if mapper.is_event(record, "acceptResume") and mapper.result_of(record) == "success":
            if "kill_seq" in state:
                return self._finding(
                    record, "successful resume after a successful kill",
                    [state["kill_seq"], record.seq])
        if mapper.is_event(record, "acceptPause") and mapper.result_of(record) == "success":
            if "kill_seq" in state:
                return self._finding(
                    record, "successful pause after a successful kill",
                    [state["kill_seq"], record.seq])
        return None

    def on_observe(self, state, record):
        if mapper.running(record) and "kill_seq" in state:
            return self._finding(
                record, "sandbox observed running after a successful kill",
                [state["kill_seq"], record.seq])
        if mapper.running(record) and "kill_unknown_seq" in state:
            return self._finding(
                record, "running after a kill with unknown result (cannot decide)",
                [state["kill_unknown_seq"], record.seq], inconclusive=True)
        return None


class TerminalMonotone(Rule):
    """A terminal (kill) observation is never followed by a new successful start."""
    name = "terminal_monotone"
    properties = ["terminalMonotone"]

    def on_event(self, state, record):
        if mapper.is_event(record, "acceptKill") and mapper.result_of(record) == "success":
            state["terminal_seq"] = record.seq
        if (mapper.is_event(record, "acceptCreate") or mapper.is_event(record, "acceptResume")) \
                and mapper.result_of(record) == "success" and "terminal_seq" in state:
            return self._finding(
                record, "a successful start clears an irreversible terminal state",
                [state["terminal_seq"], record.seq])
        return None


class RunningRequiresStart(Rule):
    """A running observation must be preceded by an accepted create or resume."""
    name = "running_requires_start"
    properties = ["runningRequiresStart", "AuthorityShape"]

    def on_event(self, state, record):
        if (mapper.is_event(record, "acceptCreate") or mapper.is_event(record, "acceptResume")) \
                and mapper.result_of(record) == "success":
            state["start_seq"] = record.seq
        return None

    def on_observe(self, state, record):
        if mapper.running(record) and "start_seq" not in state:
            return self._finding(
                record, "observed running without an accepted create/resume", [record.seq])
        return None


class ResumeRequiresSnapshot(Rule):
    """A successful resume requires a usable (present, not deleted) snapshot."""
    name = "resume_requires_snapshot"
    properties = ["RestoreSourceValid", "PausedSnapshotValid"]

    def on_observe(self, state, record):
        if mapper.snapshot_present(record):
            state["snapshot_seen"] = True
        deleted = mapper.snapshot_deleted(record)
        if deleted is not None:
            state["snapshot_deleted"] = deleted
        return None

    def on_event(self, state, record):
        if mapper.is_event(record, "acceptResume") and mapper.result_of(record) == "success":
            if not state.get("snapshot_seen"):
                return self._finding(record, "resume succeeded without an observed snapshot", [record.seq])
            if state.get("snapshot_deleted"):
                return self._finding(record, "resume succeeded after the snapshot was deleted", [record.seq])
        return None


class PauseRequiresSnapshot(Rule):
    """A paused observation must be backed by a present, non-deleted snapshot."""
    name = "pause_requires_snapshot"
    properties = ["PausedSnapshotValid", "DurableSnapshotsImmutable"]

    def on_observe(self, state, record):
        if mapper.phase_hint(record) != "paused":
            return None
        if mapper.snapshot_present(record) is False:
            return self._finding(record, "paused observed without a snapshot", [record.seq])
        if mapper.snapshot_deleted(record) is True:
            return self._finding(record, "paused observed with a deleted snapshot", [record.seq])
        return None


class RunningRequiresAppliedPolicy(Rule):
    """A running sandbox must have the current file policy applied."""
    name = "running_requires_applied_policy"
    properties = ["RunningRequiresAppliedFilePolicy", "AppliedFilePolicyValid"]
    requires = ["applied_policy", "file_policy"]

    def on_observe(self, state, record):
        if not mapper.running(record):
            return None
        applied = record.obs.get("applied_policy")
        policy = record.obs.get("file_policy")
        if applied is not None and policy is not None and applied != policy:
            return self._finding(
                record, "running with a stale/foreign applied file policy", [record.seq])
        return None


# --------------------------------------------------------------------------
# Identity stability
# --------------------------------------------------------------------------

class _ScalarStable(Rule):
    """Base for properties of the form 'once observed, a value never changes'."""

    key: str = ""

    def on_observe(self, state, record):
        if self.key not in record.obs:
            return None
        value = record.obs[self.key]
        previous = state.get("value")
        if previous is None:
            state["value"] = value
            state["seq"] = record.seq
        elif value != previous:
            return self._finding(
                record, f"{self.key} changed from {previous!r} to {value!r}",
                [state["seq"], record.seq])
        return None


class SnapshotManifestStable(_ScalarStable):
    name = "snapshot_manifest_stable"
    properties = ["snapshotManifestStable"]
    requires = ["snapshot_manifest"]
    key = "snapshot_manifest"


class WorkspaceStable(_ScalarStable):
    name = "workspace_stable"
    properties = ["workspaceStable", "WorkspaceBindingValid"]
    requires = ["workspace"]
    key = "workspace"


class FilePolicyStable(_ScalarStable):
    name = "file_policy_stable"
    properties = ["filePolicyStable"]
    requires = ["file_policy"]
    key = "file_policy"


# --------------------------------------------------------------------------
# Monotonicity
# --------------------------------------------------------------------------

class _ScalarMonotone(Rule):
    key: str = ""

    def on_observe(self, state, record):
        if self.key not in record.obs:
            return None
        value = record.obs[self.key]
        previous = state.get("value")
        if previous is None:
            state["value"] = value
            state["seq"] = record.seq
        elif value < previous:
            return self._finding(
                record, f"{self.key} decreased from {previous} to {value}",
                [state["seq"], record.seq])
        else:
            state["value"] = value
        return None


class GenerationMonotone(_ScalarMonotone):
    name = "generation_monotone"
    properties = ["generationMonotone"]
    requires = ["generation"]
    key = "generation"


class EpochMonotone(_ScalarMonotone):
    name = "epoch_monotone"
    properties = ["epochMonotone", "fenceMonotone"]
    requires = ["epoch"]
    key = "epoch"


# --------------------------------------------------------------------------
# Resource uniqueness / cleanup
# --------------------------------------------------------------------------

class UniqueExecution(Rule):
    name = "unique_execution"
    properties = ["UniqueExecution"]
    requires = ["executing_vms"]

    def on_observe(self, state, record):
        executing = record.obs.get("executing_vms") or []
        if len(executing) > 1:
            return self._finding(
                record, f"more than one executing VM: {sorted(executing)}", [record.seq])
        return None


class UniqueActiveSlot(Rule):
    name = "unique_active_slot"
    properties = ["UniqueActiveSlot"]
    requires = ["active_slots"]

    def on_observe(self, state, record):
        slots = record.obs.get("active_slots") or []
        if len(slots) > 1:
            return self._finding(
                record, f"more than one active network slot: {sorted(slots)}", [record.seq])
        return None


class ReclaimedNotCurrent(Rule):
    name = "reclaimed_not_current"
    properties = ["ReclaimedResourcesNotCurrent", "cleanupAdmission"]
    requires = ["reclaimed_vms", "reclaimed_slots", "current_vm", "current_slot"]

    def on_observe(self, state, record):
        reclaimed_vms = set(record.obs.get("reclaimed_vms") or [])
        reclaimed_slots = set(record.obs.get("reclaimed_slots") or [])
        current_vm = record.obs.get("current_vm")
        current_slot = record.obs.get("current_slot")
        if current_vm is not None and current_vm in reclaimed_vms:
            return self._finding(
                record, f"current VM {current_vm} is reclaimed", [record.seq])
        if current_slot is not None and current_slot in reclaimed_slots:
            return self._finding(
                record, f"current slot {current_slot} is reclaimed", [record.seq])
        return None


# --------------------------------------------------------------------------
# Ledgers
# --------------------------------------------------------------------------

class OperationStable(Rule):
    """Operation identity and final result are immutable under one key."""
    name = "operation_stable"
    properties = ["OperationResultStable", "OperationIdentityStable"]
    requires = ["operations"]

    def on_observe(self, state, record):
        seen = state.setdefault("ops", {})
        for op in record.obs.get("operations") or []:
            key = op.get("id")
            previous = seen.get(key)
            if previous is None:
                seen[key] = dict(op)
                continue
            for field in ("kind", "epoch"):
                if op.get(field) != previous.get(field):
                    return self._finding(
                        record, f"operation {key} {field} changed "
                                f"from {previous.get(field)!r} to {op.get(field)!r}",
                        [record.seq])
            if previous.get("result") is not None and op.get("result") != previous.get("result"):
                return self._finding(
                    record, f"operation {key} final result rewritten "
                            f"from {previous.get('result')!r} to {op.get('result')!r}",
                    [record.seq])
            seen[key] = dict(op)
        return None


class EffectAdmission(Rule):
    """Effects never regress, acknowledged results are stable, and no ack
    appears for an effect that was not first observed committed/dispatched."""
    name = "effect_admission"
    properties = ["effectAdmission", "effectResultStable", "EffectIdentityValid"]
    requires = ["effects"]
    _rank = {"committed": 0, "dispatched": 1, "acknowledged": 2}

    def on_observe(self, state, record):
        seen = state.setdefault("eff", {})
        for effect in record.obs.get("effects") or []:
            key = effect.get("id")
            status = effect.get("status")
            previous = seen.get(key)
            if previous is None:
                seen[key] = dict(effect)
                continue
            if self._rank.get(status, -1) < self._rank.get(previous.get("status"), -1):
                return self._finding(
                    record, f"effect {key} status regressed from "
                            f"{previous.get('status')!r} to {status!r}", [record.seq])
            if previous.get("status") == "acknowledged" and \
                    previous.get("result") != effect.get("result"):
                return self._finding(
                    record, f"effect {key} acknowledged result changed", [record.seq])
            seen[key] = dict(effect)
        return None


# --------------------------------------------------------------------------
# Quiescence, slots, snapshots, fences, admission, accounting
# --------------------------------------------------------------------------

class StableStatesQuiescent(Rule):
    name = "stable_states_quiescent"
    properties = ["StableStatesQuiescent", "EffectiveRuntimeValid"]
    requires = ["executing_vms", "active_slots"]

    def on_observe(self, state, record):
        phase = record.obs.get("phase_hint")
        if phase not in ("paused", "destroyed"):
            return None
        if record.obs.get("executing_vms"):
            return self._finding(record, f"{phase} state has an executing VM", [record.seq])
        if record.obs.get("active_slots"):
            return self._finding(record, f"{phase} state has an active network slot", [record.seq])
        return None


class ActiveSlotValid(Rule):
    name = "active_slot_valid"
    properties = ["ActiveSlotValid", "CurrentResourcesValid"]
    requires = ["active_slots", "current_slot"]

    def on_observe(self, state, record):
        active = record.obs.get("active_slots")
        current = record.obs.get("current_slot")
        if active and current is not None and current not in active:
            return self._finding(
                record, f"active slot {active} does not match current slot {current}",
                [record.seq])
        return None


class DurableSnapshotImmutable(Rule):
    name = "durable_snapshot_immutable"
    properties = ["DurableSnapshotsImmutable"]
    requires = ["snapshot_stage", "snapshot_immutable"]

    def on_observe(self, state, record):
        if record.obs.get("snapshot_stage") == "durable" and \
                record.obs.get("snapshot_immutable") is False:
            return self._finding(record, "a durable snapshot is marked immutable = false", [record.seq])
        return None


class FenceWellFormed(Rule):
    name = "fence_well_formed"
    properties = ["FenceWellFormed", "FenceClosesAuthority"]
    requires = ["fence_epoch", "epoch"]

    def on_observe(self, state, record):
        fence_epoch = record.obs.get("fence_epoch")
        epoch = record.obs.get("epoch")
        if fence_epoch is not None and epoch is not None and fence_epoch > epoch:
            return self._finding(
                record, f"fence epoch {fence_epoch} exceeds record epoch {epoch}", [record.seq])
        if record.obs.get("fence_terminal") and record.obs.get("terminal") is False:
            return self._finding(record, "terminal fence on a non-terminal record", [record.seq])
        return None


class EventAuthorized(Rule):
    name = "event_authorized"
    properties = ["eventAuthorized"]
    requires = ["event_authorized"]

    def on_observe(self, state, record):
        if record.obs.get("event_authorized") is False:
            return self._finding(record, "user-facing event was not authorized", [record.seq])
        return None


class EgressEnforced(Rule):
    name = "egress_enforced"
    properties = ["egressEnforced"]
    requires = ["egress_destination", "egress_allowlist"]

    def on_observe(self, state, record):
        destination = record.obs.get("egress_destination")
        allowlist = record.obs.get("egress_allowlist")
        if destination is not None and isinstance(allowlist, list) and destination not in allowlist:
            return self._finding(
                record, f"egress to {destination!r} outside allowlist {allowlist}", [record.seq])
        return None


class FileWorkspaceBound(Rule):
    name = "file_workspace_bound"
    properties = ["fileWorkspaceBound", "WorkspaceBindingValid"]
    requires = ["file_event_workspace", "workspace"]

    def on_observe(self, state, record):
        event_ws = record.obs.get("file_event_workspace")
        workspace = record.obs.get("workspace")
        if event_ws is not None and workspace is not None and event_ws != workspace:
            return self._finding(
                record, f"file event targets {event_ws!r}, sandbox workspace is {workspace!r}",
                [record.seq])
        return None


class NoStaleIndexReferences(Rule):
    """Model-gap candidate (runtime#3566): index/ledger entries must reference
    an existing sandbox."""
    name = "stale_index"
    properties = ["NoStaleIndexReferences (model-gap)"]
    requires = ["index_members", "existing_sandboxes"]

    def on_observe(self, state, record):
        members = set(record.obs.get("index_members") or [])
        existing = set(record.obs.get("existing_sandboxes") or [])
        stale = members - existing
        if stale:
            return self._finding(
                record, f"index references non-existent sandboxes: {sorted(stale)}",
                [record.seq])
        return None


class ExecutionNotReused(Rule):
    """A retired execution identity must never become current again."""
    name = "execution_not_reused"
    properties = ["generationMonotone", "ReclaimedResourcesNotCurrent", "RestoreSourceValid"]
    requires = ["execution_id"]

    def on_observe(self, state, record):
        execution = record.obs.get("execution_id")
        if execution is None:
            return None
        current = state.get("current")
        retired = state.setdefault("retired", set())
        if current is None:
            state["current"] = execution
        elif execution != current:
            if execution in retired:
                return self._finding(
                    record, f"retired execution {execution} became current again",
                    [state["seq"], record.seq])
            retired.add(current)
            state["current"] = execution
            state["seq"] = record.seq
        return None


class NodeOrphan(Rule):
    """A sandbox held by the node but absent from the running store is an
    orphan: a failed create that did not reclaim its resource (E2B#1498 class).
    Checked at end of trace, when a create window has settled."""
    name = "node_orphan"
    properties = ["CreateCompensation", "NoOrphanOnNode"]

    def on_observe(self, state, record):
        node = record.obs.get("node_sandboxes")
        store = record.obs.get("existing_sandboxes")
        if node is not None and store is not None:
            state["node"] = set(node)
            state["store"] = set(store)
        return None

    def on_finish(self, state, sandbox):
        node = state.get("node")
        store = state.get("store")
        if not node or store is None:
            return None
        orphan = node - store
        if orphan:
            return Finding(
                rule=self.name, sandbox=sandbox, seq=0,
                detail=f"node holds sandboxes absent from the running store: {sorted(orphan)}",
                minimal_subsequence=[], properties=list(self.properties))
        return None


class LifecycleCompletion(Rule):
    """A trace must end in a stable phase; ending in a transient phase is a
    liveness signal (inconclusive), not a safety violation."""
    name = "lifecycle_completion"
    properties = ["LifecycleCompletionSchema"]
    requires = ["phase_hint"]
    _transient = {"starting", "pausing", "revoking"}

    def on_observe(self, state, record):
        phase = record.obs.get("phase_hint")
        if phase is not None:
            state["last_phase"] = phase
            state["last_seq"] = record.seq
        return None

    def on_finish(self, state, sandbox):
        phase = state.get("last_phase")
        if phase in self._transient:
            return Finding(
                rule=self.name, sandbox=sandbox, seq=state.get("last_seq", 0),
                detail=f"trace ends in transient phase {phase!r}; lifecycle did not complete",
                minimal_subsequence=[state.get("last_seq", 0)],
                properties=list(self.properties), inconclusive=True)
        return None


REGISTRY: list[Rule] = [
    KillIrreversible(),
    TerminalMonotone(),
    RunningRequiresStart(),
    ResumeRequiresSnapshot(),
    PauseRequiresSnapshot(),
    RunningRequiresAppliedPolicy(),
    SnapshotManifestStable(),
    WorkspaceStable(),
    FilePolicyStable(),
    GenerationMonotone(),
    EpochMonotone(),
    UniqueExecution(),
    UniqueActiveSlot(),
    ReclaimedNotCurrent(),
    OperationStable(),
    EffectAdmission(),
    StableStatesQuiescent(),
    ActiveSlotValid(),
    DurableSnapshotImmutable(),
    FenceWellFormed(),
    EventAuthorized(),
    EgressEnforced(),
    FileWorkspaceBound(),
    NoStaleIndexReferences(),
    ExecutionNotReused(),
    NodeOrphan(),
    LifecycleCompletion(),
]
