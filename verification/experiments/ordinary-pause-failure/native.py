"""Strict replay of the existing ordinary-Pause failure policy and property.

Keeps the historical capture-handoff definitions and proofs unchanged. Only
the original phase assertion, reached after the actual compensation call,
qualifies as this new entry's native rejection. No physical VM is stopped.
"""
import importlib.util
from pathlib import Path
import re
import sys

HERE = Path('verification/experiments/ordinary-pause-failure')
ROOT = Path(__file__).resolve().parents[3]


def load(name, relative):
    spec = importlib.util.spec_from_file_location(name, ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


runner = load('ordinary_pause_native_runner', 'verification/experiments/unknown-host-release/native.py')
legacy = load('ordinary_pause_existing_policy', 'verification/experiments/capture-handoff/native.py')
CASES = [case for case in legacy.CASES if case[0] == 'terminal-pause']
if len(CASES) != 1:
    raise ValueError('Require the one existing ordinary-Pause wrong policy')
CASE = CASES[0]
SOURCE = Path(CASE[1])
BEFORE, AFTER = CASE[2:4]
FIXTURE = Path('controller-core/src/capture/tests.rs')
PACKAGE = 'sandbox-controller-core'
PACKAGE_DIR = Path('controller-core')
CRATE = 'sandbox_controller_core'
MUTANT = CASE[0]
KIND = 'ordinary-pause-failure-native'
FIXTURE_SCOPE = 'complete original capture tests including the helper phase assertion; all other committed inputs also checked'
FIRST = '\npub fn begin_compensation('
LAST = '\npub fn mark_operation_outcome_unknown('
PROPERTY = 'capture::tests::' + CASE[4]
TESTS = (PROPERTY,
    'checkpoint::tests::unload::each_real_pause_microstep_roundtrips_in_the_only_current_schema',
    'capture::tests::owner_destroy_revokes_eligibility_but_never_releases_capture_material',
    'workflow::tests::exact_abort_allows_failed_publication_but_keeps_unknown_prepare_and_occupied_slot')
HELPER = '\nfn assert_original_kept('
MARKER = '    assert_eq!(held.phase, Phase::Pausing);'
CALL = '    assert_original_kept(&before, &after);'


def fixture_digest(project):
    return runner.sha(project / FIXTURE)


def mutate(project):
    path = project / SOURCE
    original = path.read_text()
    first = original.index(FIRST)
    last = original.index(LAST, first)
    if original.count(BEFORE) != 1 or original[first:last].count(BEFORE) != 1:
        raise ValueError('Require the existing unique wrong policy in original begin_compensation')
    fixture = fixture_digest(project)
    legacy.mutate(project, CASE)
    if path.read_text() != original.replace(BEFORE, AFTER, 1):
        raise ValueError('Existing mutation changed more than its original policy')
    if fixture_digest(project) != fixture:
        raise ValueError('Mutation changed the complete original capture tests')


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    helper = text.index(HELPER)
    helper_end = text.index('\n#[test]', helper)
    first = text.index('\nfn ' + PROPERTY.rsplit('::', 1)[1] + '() {')
    last = text.index('\n#[test]', first)
    if (text[helper:helper_end].count(MARKER) != 1
            or text[first:last].count(CALL) != 1 or helper_end > first):
        raise ValueError('Require the original property and its unique original helper phase assertion')
    return text[:text.index(MARKER, helper)].count('\n') + 1


def checked_execution(stdout, stderr, code, name, rejected, line):
    if not rejected:
        return runner.checked_execution(stdout, stderr, code, name, False, line)
    if name != PROPERTY:
        raise ValueError('The one original ordinary-Pause property did not fail as required')
    message = runner.checked_execution(stdout, stderr, code, name, True, line,
        fixture=FIXTURE, assertion=b'left: Revoking',
        assertion_header=b'assertion `left == right` failed')
    if (not re.search(rb'^[ \t]*left: Revoking[ \t]*$', message, re.M)
            or not re.search(rb'^[ \t]*right: Pausing[ \t]*$', message, re.M)):
        raise ValueError('Require the original phase assertion observing Revoking instead of Pausing')


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('This strict ordinary-Pause entry requires the fixed Linux toolchain')
    runner.main(case=sys.modules[__name__])
