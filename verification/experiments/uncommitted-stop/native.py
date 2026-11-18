"""Actual worker refusal of Stop before snapshot registration; no VM or Lean.

The original test queues and dispatches the original Stop against a sealed,
uncommitted capture. A private mutant bypasses the worker's complete snapshot
gate, retaining its earlier source, epoch, VM state and fence checks. The same
original equality assertion must observe an erroneous StopVm decision.
"""
import importlib.util
from pathlib import Path
import re
import sys

HERE = Path('verification/experiments/uncommitted-stop')
ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('uncommitted_stop_native_runner',
    ROOT / 'verification/experiments/unknown-host-release/native.py')
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)

SOURCE = Path('controller-core/src/worker/pause_stop.rs')
FIXTURE = Path('controller-core/src/capture/tests.rs')
PACKAGE = 'sandbox-controller-core'
PACKAGE_DIR = Path('controller-core')
CRATE = 'sandbox_controller_core'
MUTANT = 'skip-snapshot-stop-gate'
KIND = 'uncommitted-stop-native'
FIXTURE_SCOPE = 'complete original Rust capture test file; all other committed inputs also checked'
BEFORE = '''        if plan.vm != Some(vm)
            || !crate::transition::current_capture(world, sandbox, record)
                .is_some_and(crate::capture::CaptureHandoff::may_stop_original)
        {
            return Err(WorkerReject::SnapshotNotReady);
        }
        let Some(target) = plan.target_snapshot else {
            return Err(WorkerReject::SnapshotNotReady);
        };
        let Some(stored_snapshot) = world.snapshots.get(&target.snapshot) else {
            return Err(WorkerReject::SnapshotNotReady);
        };
        if stored_snapshot.owner != record.owner
            || stored_snapshot.sandbox != sandbox
            || stored_snapshot.source_generation != record.generation
            || stored_snapshot.workspace != record.workspace
            || stored_snapshot.manifest != target.manifest
            || stored_snapshot.stage != SnapshotStage::Durable
            || !stored_snapshot.immutable
        {
            return Err(WorkerReject::SnapshotNotReady);
        }
'''
AFTER = '        if false {\n' + ''.join('    '+line for line in BEFORE.splitlines(keepends=True)) + '        }\n'
PREFIX = 'capture::tests::'
PROPERTY = PREFIX + 'early_stop_without_registered_handoff_is_refused_by_real_worker'
TESTS = (PROPERTY,
    PREFIX + 'registered_handoff_keeps_material_responsibility_and_allows_original_unload',
    PREFIX + 'ordinary_failure_keeps_live_original_and_invalidates_already_queued_freeze')


def fixture_digest(project):
    return runner.sha(project / FIXTURE)


def mutate(project):
    path = project / SOURCE
    text = path.read_text()
    start = text.index('\nfn stop_vm(')
    end = text.index('\n    let mut next = world.clone();', start)
    fixture = fixture_digest(project)
    if text.count(BEFORE) != 1 or text[start:end].count(BEFORE) != 1:
        raise ValueError('Require the unique actual Stop snapshot gate after original fence checks')
    path.write_text(text.replace(BEFORE, AFTER, 1))
    if fixture_digest(project) != fixture:
        raise ValueError('Mutation changed the complete original Rust tests')


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    start = text.index('\nfn ' + PROPERTY.rsplit('::', 1)[1] + '() {')
    end = text.index('\n#[test]', start)
    marker = '    assert_eq!(\n        handle_worker(&dispatched, &id),\n        Err(WorkerReject::SnapshotNotReady)\n    );'
    if text[start:end].count(marker) != 1:
        raise ValueError('Require the exact original worker SnapshotNotReady equality assertion')
    return text[:text.index(marker, start)].count('\n') + 1


def checked_execution(stdout, stderr, code, name, rejected, line):
    if not rejected:
        return runner.checked_execution(stdout, stderr, code, name, False, line)
    # This original Rust assertion is assert_eq!, unlike the older assert!
    # cases. Inspect its real output without rewriting it to the other shape.
    if name != PROPERTY:
        raise ValueError('The exact original worker test did not fail as required')
    message = runner.checked_execution(stdout, stderr, code, name, True, line,
        fixture=FIXTURE, assertion=b'event: StopVm {',
        assertion_header=b'assertion `left == right` failed')
    if (not re.search(rb'^[ \t]*left: Ok\(WorkerDecision \{ event: StopVm \{', message, re.M)
            or not re.search(rb'^[ \t]*right: Err\(SnapshotNotReady\)[ \t]*$', message, re.M)):
        raise ValueError('Require the original assertion rejecting an actual successful StopVm decision')


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('This actual worker native entry requires the fixed Linux toolchain')
    runner.main(case=sys.modules[__name__])
