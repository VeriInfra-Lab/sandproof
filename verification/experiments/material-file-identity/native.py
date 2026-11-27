"""Original Rust retirement assertions against an actual file-identity mutant.

Uses private real files and saved plans, without a VM, concurrent reader or
service-side deletion permission. No Lean/extraction acceptance is claimed.
"""
import importlib.util
from pathlib import Path
import sys

HERE = Path('verification/experiments/material-file-identity')
ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('material_file_identity_native_runner',
    ROOT / 'verification/experiments/unknown-host-release/native.py')
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)

SOURCE = Path('sandbox-runtime/src/snapshot_material_retirement.rs')
FIXTURE = Path('sandbox-runtime/src/snapshot_material_retirement_tests.rs')
PACKAGE = 'sandbox-runtime'
PACKAGE_DIR = Path('sandbox-runtime')
CRATE = 'sandbox_runtime'
MUTANT = 'accept-planned-path'
KIND = 'material-file-identity-native'
FIXTURE_SCOPE = 'complete original Rust test file'
BEFORE = '        Some(actual) if Some(&actual) == expected => Ok(true),\n'
AFTER = '        Some(actual) if expected.is_some() || Some(&actual) == expected => Ok(true),\n'
PREFIX = 'snapshot_store::material_retirement::tests::'
PROPERTY = PREFIX + 'changed_files_foreign_entries_and_late_output_are_rejected_before_first_unlink'
TESTS = (PROPERTY,
    PREFIX + 'never_sealed_partial_files_and_both_marker_stages_retire_with_same_saved_plan',
    PREFIX + 'absent_output_plan_rejects_new_directory_or_file_instead_of_adopting_it')


def fixture_digest(project):
    return runner.sha(project / FIXTURE)


def mutate(project):
    path = project / SOURCE
    text = path.read_text()
    start = text.index('\nfn verify_leaf(')
    end = text.index('\nimpl CaptureStoreRetirement {', start)
    fixture = fixture_digest(project)
    if text.count(BEFORE) != 1 or text[start:end].count(BEFORE) != 1:
        raise ValueError('Require the unique actual saved file identity check in verify_leaf')
    path.write_text(text.replace(BEFORE, AFTER, 1))
    if fixture_digest(project) != fixture:
        raise ValueError('Mutation changed the complete original Rust tests')


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    start = text.index('\nfn ' + PROPERTY.rsplit('::', 1)[1] + '() {')
    end = text.index('\n#[test]', start)
    marker = '        assert!(fixture.store.retire_capture_material(&plan).is_err());'
    if text[start:end].count(marker) != 1:
        raise ValueError('Require the exact unchanged original pre-unlink rejection assertion')
    return text[:text.index(marker, start)].count('\n') + 1


def checked_execution(stdout, stderr, code, name, rejected, line):
    runner.checked_execution(stdout, stderr, code, name, rejected, line, fixture=FIXTURE,
        assertion=b'fixture.store.retire_capture_material(&plan).is_err()')


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('This actual Runtime retirement native entry requires Linux')
    runner.main(case=sys.modules[__name__])
