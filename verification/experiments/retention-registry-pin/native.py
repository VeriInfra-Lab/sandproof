"""A saved terminal confirmation cannot retire an active registry obligation.

Use the original SQLite window tests unchanged. The private wrong body still
queries the original pin (including errors), but ignores a positive result.
This is a finite journal boundary check, not physical cleanup or a Lean proof.
"""
import importlib.util
from pathlib import Path
import sys

HERE = Path('verification/experiments/retention-registry-pin')
ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('retention_registry_pin_runner',
    ROOT / 'verification/experiments/unknown-host-release/native.py')
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)

SOURCE = Path('controller-service/src/controller/lifecycle_inbox/window.rs')
FIXTURE = Path('controller-service/src/controller/lifecycle_inbox/window_tests.rs')
PACKAGE = 'sandproof-controller-service'
PACKAGE_DIR = Path('controller-service')
CRATE = 'sandproof_controller_service'
MUTANT = 'ignore-positive-registry-pin'
KIND = 'retention-registry-pin-native'
FIXTURE_SCOPE = 'complete original Rust window tests; all other committed inputs also checked'
FIRST = '    pub fn close_lifecycle_window('
LAST = '    /// # Errors\n    /// Only the protected original node may read finite lane state.'
BEFORE = '            && self.host_slot_release_pinned(local.sandbox)?\n'
AFTER = '            && self.host_slot_release_pinned(local.sandbox)? && false\n'
PREFIX = 'controller::lifecycle_inbox::window::tests::'
PROPERTY = PREFIX + 'active_native_registry_pin_vetoes_close_until_original_certificate_settlement'
TESTS = (PROPERTY,
    PREFIX + 'close_known_crashes_keep_original_confirmation_or_replay_the_exact_retired_stamp',
    PREFIX + 'unknown_close_commit_does_not_mint_a_new_window_or_acknowledge_another_request',
    PREFIX + 'foreign_or_changed_close_cannot_mutate_the_original_confirmed_window')
MARKER = '''    assert!(matches!(
        journal.close_lifecycle_window(&close, &context(), &mut NeverCrash),
        Err(JournalError::UnresolvedAction)
    ));'''
ASSERTION = b'journal.close_lifecycle_window(&close, &context(), &mut NeverCrash)'


def fixture_digest(project):
    return runner.sha(project / FIXTURE)


def mutate(project):
    path = project / SOURCE
    original = path.read_text()
    first = original.index(FIRST)
    last = original.index(LAST, first)
    if original.count(BEFORE) != 1 or original[first:last].count(BEFORE) != 1:
        raise ValueError('Require the unique original pin query inside close_lifecycle_window')
    fixture = fixture_digest(project)
    path.write_text(original.replace(BEFORE, AFTER, 1))
    if fixture_digest(project) != fixture:
        raise ValueError('Mutation changed the complete original window tests')


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    first = text.index('\nfn ' + PROPERTY.rsplit('::', 1)[1] + '() {')
    last = text.index('\n#[test]', first)
    if text[first:last].count(MARKER) != 1:
        raise ValueError('Require the original active-pin UnresolvedAction assertion')
    return text[:text.index(MARKER, first)].count('\n') + 1


def checked_execution(stdout, stderr, code, name, rejected, line):
    if rejected and name != PROPERTY:
        raise ValueError('Only the active-pin window refusal is the counterexample')
    runner.checked_execution(stdout, stderr, code, name, rejected, line,
        fixture=FIXTURE, assertion=ASSERTION)
    if rejected and b'Err(JournalError::UnresolvedAction)' not in stdout + stderr:
        raise ValueError('Require the same original UnresolvedAction contract')


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('This actual SQLite window entry requires the fixed Linux toolchain')
    runner.main(case=sys.modules[__name__])
