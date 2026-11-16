"""Actual journal pin lifetime after Load and activation registration.

One private wrong policy treats the post-Load activation marker as the end of
material use. Its related use checks adopt that same policy, so an earlier
CorruptStore refusal cannot masquerade as the required counterexample. The
unchanged original test must read an actual false pin from the real journal.
"""
import importlib.util
from pathlib import Path
import sys

HERE = Path('verification/experiments/loaded-material-pin')
ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('loaded_pin_native_runner',
    ROOT / 'verification/experiments/unknown-host-release/native.py')
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)

SOURCE = Path('controller-service/src/controller/lifecycle_inbox/source_work/restore.rs')
FIXTURE = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests.rs')
PACKAGE = 'sandproof-controller-service'
PACKAGE_DIR = Path('controller-service')
CRATE = 'sandproof_controller_service'
MUTANT = 'activation-marker-closes-material-use'
KIND = 'loaded-material-pin-native'
FIXTURE_SCOPE = 'complete original Rust source-work test file; all other committed inputs also checked'
BEFORE = (
    '''    if !claim.material_use {
        return Err(JournalError::UnresolvedAction);
    }
''',
    '''    if !claim.material_use && !super::retirement::has_closed_material_use(record) {
        return Err(JournalError::CorruptStore);
    }
''',
    '''    let record = &mut sources.records[index];
    let claim = record.restore.as_mut().ok_or(JournalError::CorruptStore)?;
''',
    '''    if !claim.material_use {
        return Ok(false);
    }
''',
    '''    claim.activation = Some(action.clone());
    codec::save(connection, &sources, sources.max_sessions)?;
''',
)
AFTER = (
    '''    if !claim.material_use
        && (claim.activation.is_none() || super::retirement::has_closed_material_use(record))
    {
        return Err(JournalError::UnresolvedAction);
    }
''',
    '''    if !claim.material_use
        && claim.activation.is_none()
        && !super::retirement::has_closed_material_use(record)
    {
        return Err(JournalError::CorruptStore);
    }
''',
    '''    let record = &mut sources.records[index];
    let use_closed = super::retirement::has_closed_material_use(record);
    let claim = record.restore.as_mut().ok_or(JournalError::CorruptStore)?;
''',
    '''    if !claim.material_use && (claim.activation.is_none() || use_closed) {
        return Ok(false);
    }
''',
    '''    claim.activation = Some(action.clone());
    claim.material_use = false;
    codec::save(connection, &sources, sources.max_sessions)?;
''',
)
SCOPES = (
    ('\nfn work(', '\npub(super) fn validate_record('),
    ('\npub(super) fn validate_record(', '\npub(in crate::controller) fn claim_activation('),
    ('\npub(in crate::controller) fn claim_activation(', '\nfn admitted_source_generation('),
    ('\npub(in crate::controller) fn claim_activation(', '\nfn admitted_source_generation('),
    ('\npub(in crate::controller) fn claim_activation(', '\nfn admitted_source_generation('),
)
PREFIX = 'controller::lifecycle_inbox::source_work::tests::'
PROPERTY = PREFIX + 'second_full_capture_keeps_first_source_and_running_backing_bound_to_their_original_operations'
TESTS = (PROPERTY,
    PREFIX + 'restore_unknown_terminal::unknown_prepare_destroy_preserves_backing_and_finishes_without_second_load',
    PREFIX + 'material_retirement::superseded_material_waits_for_durable_successor_and_exact_consumer_release')
ASSERTION = b'first_before.restore.as_ref().unwrap().material_use'


def fixture_digest(project):
    return runner.sha(project / FIXTURE)


def mutate(project):
    path = project / SOURCE
    original = path.read_text()
    fixture = fixture_digest(project)
    for before, (first, last) in zip(BEFORE, SCOPES, strict=True):
        start = original.index(first)
        end = original.index(last, start)
        if original.count(before) != 1 or original[start:end].count(before) != 1:
            raise ValueError('Require the unique actual related material-use policy at its original function')
    changed = original
    for before, after in zip(BEFORE, AFTER, strict=True):
        changed = changed.replace(before, after, 1)
    path.write_text(changed)
    if fixture_digest(project) != fixture:
        raise ValueError('Mutation changed the complete original Rust test file')


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    start = text.index('\nfn ' + PROPERTY.rsplit('::', 1)[1] + '() {')
    end = text.index('\n#[test]', start)
    marker = '    assert!(first_before.restore.as_ref().unwrap().material_use);'
    if text[start:end].count(marker) != 1:
        raise ValueError('Require the exact original journal pin-retention assertion')
    return text[:text.index(marker, start)].count('\n') + 1


def checked_execution(stdout, stderr, code, name, rejected, line):
    if rejected and name != PROPERTY:
        raise ValueError('Only the original live-backing pin property is the required rejection')
    return runner.checked_execution(stdout, stderr, code, name, rejected, line,
        fixture=FIXTURE, assertion=ASSERTION)


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('This actual journal pin entry requires the fixed Linux toolchain')
    runner.main(case=sys.modules[__name__])
