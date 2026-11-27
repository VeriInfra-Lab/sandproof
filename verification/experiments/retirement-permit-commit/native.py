"""Deletion permission requires the original persisted plan/use closure.

One private wrong policy keeps all original eligibility and validation checks
but omits saving the source rows before the transaction returns its permit.
The unchanged journal test must observe that the old material use is open.
"""
import importlib.util
from pathlib import Path
import sys

HERE = Path('verification/experiments/retirement-permit-commit')
ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('retirement_permit_native_runner',
    ROOT / 'verification/experiments/unknown-host-release/native.py')
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)

SOURCE = Path('controller-service/src/controller/lifecycle_inbox/source_work/retirement.rs')
FIXTURE = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests/material_retirement.rs')
PACKAGE = 'sandproof-controller-service'
PACKAGE_DIR = Path('controller-service')
CRATE = 'sandproof_controller_service'
MUTANT = 'permit-without-source-plan-save'
KIND = 'retirement-permit-commit-native'
FIXTURE_SCOPE = 'complete original Rust material-retirement test file; all other committed inputs also checked'
FIRST = '    pub(crate) fn prepare_lifecycle_material_retirement('
LAST = '    /// Only a matching actual deletion receipt settles the original reservation.'
BEFORE = '''        validate_record(record, &world)?;
        validate_relations(&sources.records, &world)?;
        codec::save(&tx, &sources, sources.max_sessions)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(MaterialRetirementPermit::after_plan_commit(
'''
AFTER = '''        validate_record(record, &world)?;
        validate_relations(&sources.records, &world)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(MaterialRetirementPermit::after_plan_commit(
'''
PREFIX = 'controller::lifecycle_inbox::source_work::tests::'
PROPERTY = PREFIX + 'material_retirement::old_material_settlement_preserves_live_second_restore_credentials_and_history'
TESTS = (PROPERTY,
    PREFIX + 'material_retirement::superseded_material_waits_for_durable_successor_and_exact_consumer_release',
    PREFIX + 'material_retirement::current_source_schema_requires_both_explicit_release_options',
    PREFIX + 'second_full_capture_keeps_first_source_and_running_backing_bound_to_their_original_operations')
MARKER = '    assert!(!retiring.restore.as_ref().unwrap().material_use);'
ASSERTION = b'!retiring.restore.as_ref().unwrap().material_use'


def fixture_digest(project):
    return runner.sha(project / FIXTURE)


def mutate(project):
    path = project / SOURCE
    original = path.read_text()
    first = original.index(FIRST)
    last = original.index(LAST, first)
    if original.count(BEFORE) != 1 or original[first:last].count(BEFORE) != 1:
        raise ValueError('Require the unique original save/commit/permit boundary in its function')
    fixture = fixture_digest(project)
    path.write_text(original.replace(BEFORE, AFTER, 1))
    if fixture_digest(project) != fixture:
        raise ValueError('Mutation changed the complete original material-retirement tests')


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    first = text.index('\nfn ' + PROPERTY.rsplit('::', 1)[1] + '() {')
    last = text.index('\n#[test]', first)
    if text[first:last].count(MARKER) != 1:
        raise ValueError('Require the exact original post-permit material-use closure assertion')
    return text[:text.index(MARKER, first)].count('\n') + 1


def checked_execution(stdout, stderr, code, name, rejected, line):
    if rejected and name != PROPERTY:
        raise ValueError('Only the original post-permit closure assertion is the counterexample')
    return runner.checked_execution(stdout, stderr, code, name, rejected, line,
        fixture=FIXTURE, assertion=ASSERTION)


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('This actual journal retirement-permit entry requires the fixed Linux toolchain')
    runner.main(case=sys.modules[__name__])
