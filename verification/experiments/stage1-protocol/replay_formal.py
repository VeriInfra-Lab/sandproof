"""Local request/replay/envelope proofs on fresh extractions; not batch acceptance.

Retains the historical component freeze. Each case has an isolated source and
Lean build directory; both use the same independently stated property.
Ordinary tests and native mutant assertions still belong on the test machine.
"""
from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
HERE = Path('verification/experiments/stage1-protocol')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
LIBRARY = Path('verification/aeneas/stage1-protocol/request')
# Preserve the original extractor's source paths, without editing its output.
VIRTUAL_ROOT = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
POSITIVE = ['Library', 'ReplayContract', 'ReplayEquality']
NEGATIVE = ['ReplayContract', 'Model', 'RequestValues']
AUDITS = ['bytes_eq', 'array_eq', 'vec_eq', 'ok_and', 'resources_eq',
          'create_eq', 'node_eq', 'key_eq', 'expected_eq', 'binding_eq',
          'operation_eq', 'request_eq', 'replay_complete',
          'replay_accepts_iff', 'replay_rejects_iff', 'center_eq', 'retention_eq', 'accepted_eq']
REPLAY_AUDITS = ['SandboxModel.Stage1Protocol.Replay.' + n for n in AUDITS]
REPLAY_AUDITS += ['SandboxModel.Stage1Protocol.Library.' + n for n in
    ('bool_ne_model_exact', 'option_some_calls_original_dictionary',
     'option_mismatch_calls_no_dictionary', 'option_u64_eq')]
VALIDATION_AUDITS = ['SandboxModel.Stage1Protocol.Validation.' + n for n in
    ['resources_positive', 'checked_add_present', 'ok_unless', 'create_valid',
     'ok_ite', 'validate_refines', 'expected_success_iff', 'expected_error_iff',
     'validate_accepts_iff', 'validate_error_iff', 'validate_complete', 'validate_total',
     'validate_body_refines']]
EXAMPLE_AUDITS = ['SandboxModel.Stage1Protocol.Validation.Examples.' + n for n in
                  ['create_accepted', 'unload_accepted', 'resume_accepted', 'destroy_accepted', 'renew_accepted',
                   'resume_gap_accepted', 'previous_protocol_rejected_before_node',
                   'changed_node_generation_rejected', 'expectation_precedes_same_incarnation']]
ENVELOPE_VALUE_AUDITS = ['SandboxModel.Stage1Protocol.EnvelopeValues.' + n for n in
    ['same_session', 'changed_complete_intent', 'both_inputs_have_valid_shape']]
ACCEPTED_AUDITS = ['SandboxModel.Stage1Protocol.Validation.' + n for n in
    ['retention_valid', 'version_comparison', 'envelope_refines', 'envelope_complete',
     'expectedAccepted_success_iff', 'envelope_accepts_iff', 'envelope_keeps_context_center',
     'envelope_keeps_context_node', 'envelope_retention_shape', 'envelope_version_condition',
     'foreign_center_rejected_first', 'expectedAccepted_error_iff', 'envelope_error_iff',
     'normal_envelope_accepted', 'distinct_retention_window_accepted',
     'invalid_retention_precedes_invalid_version']]
RECORDED_AUDITS = ['SandboxModel.Stage1Protocol.Recorded.' + n for n in
    ['matches_refines', 'complete', 'accepts_iff', 'rejects_changed_intent',
     'matches_keeps_original', 'terminal_flag_not_certified',
     'older_version_rejected_after_full_intent', 'wrong_current_rejected_after_version',
     'normal_recorded_matches', 'same_session_changed_capture_rejected',
     'changed_retention_window_rejected', 'matching_does_not_establish_validation']]
PROPERTIES = {
    'replay': dict(kind='stage1-full-replay-local-formal', mutation='key-only-replay',
        positive=POSITIVE, negative=NEGATIVE,
        contract='ReplayContract', template='NegativeFullReplay', audits=REPLAY_AUDITS,
        witnesses=['RequestValues'],
        negative_audits=['SandboxModel.Stage1Protocol.Request.' + n for n in
                        ['mutant_accepts_changed_capture', 'key_only_not_full_replay']]),
    'validation': dict(kind='stage1-request-validation-local-formal', mutation='same-incarnation',
        positive=[*POSITIVE, 'ValidationModel', 'Validation', 'ValidationExamples', 'Model', 'RequestValues'],
        negative=['ValidationModel', 'Model', 'RequestValues'],
        contract='ValidationModel', template='NegativeValidation', witnesses=['RequestValues'],
        audits=REPLAY_AUDITS + VALIDATION_AUDITS + EXAMPLE_AUDITS,
        negative_audits=['SandboxModel.Stage1Protocol.Request.' + n for n in
                        ['mutant_accepts_same_incarnation', 'mutant_not_full_validation']]),
    'envelope': dict(kind='stage1-envelope-local-formal', mutation='session-only-envelope',
        positive=[*POSITIVE, 'ValidationModel', 'Validation', 'EnvelopeValues', 'Accepted', 'Recorded'],
        negative=['Model', 'ValidationModel', 'EnvelopeValues'],
        contract='ValidationModel', template='NegativeSessionOnly', witnesses=['EnvelopeValues'],
        audits=REPLAY_AUDITS + VALIDATION_AUDITS + ENVELOPE_VALUE_AUDITS + ACCEPTED_AUDITS + RECORDED_AUDITS,
        negative_audits=ENVELOPE_VALUE_AUDITS + [
            'SandboxModel.Stage1Protocol.NegativeSessionOnly.actual_mutant_accepts_changed_capture',
            'SandboxModel.Stage1Protocol.NegativeSessionOnly.actual_session_only_violates_same_contract']),
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def snapshot():
    names = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
        cwd=ROOT).decode().split('\0')
    result = {}
    for name in sorted(set(filter(None, names))):
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            raise ValueError('snapshot requires regular source files: ' + name)
        result[name] = sha(path)
    return result


def prepare_extract_target(project):
    """Allocate this case's target before the original extractor inspects it."""
    target = project / 'artifacts/extract-target'
    target.mkdir(mode=0o700, parents=True)
    return target


def run(output, property_name='replay'):
    config = PROPERTIES[property_name]
    mutant = config['mutation']
    output.mkdir(mode=0o700, parents=True)
    start = time.monotonic()
    record = dict(kind=config['kind'], cases={}, commands=[],
                  native_checks='not-run', full_acceptance=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536')

    def command(project, folder, name, argv):
        log = folder / (name + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(argv, cwd=project, env=env,
                                  stdin=subprocess.DEVNULL, stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=1800).returncode
        record['commands'].append(dict(argv=argv, cwd=str(project),
            exit_code=code, seconds=time.monotonic()-begin, log_sha256=sha(log)))
        if code:
            raise ValueError('formal command failed: ' + str(log))

    try:
        inputs = snapshot()
        record['source_commit'] = subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        record['inputs'] = inputs  # Includes uncommitted proof changes, not just HEAD.
        formal = load_module('replay_actual_files', ROOT / HERE / 'formal.py')
        mutations = load_module('replay_mutations', ROOT / HERE / 'negative_native.py')
        mutation = next(m for m in mutations.MUTATIONS if m['name'] == mutant)
        for case in ['correct', mutant]:
            folder = output / case
            project = folder / 'source'
            project.mkdir(mode=0o700, parents=True)
            for name, digest in inputs.items():
                destination = project / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, destination)
                if sha(destination) != digest:
                    raise ValueError('source changed while copying: ' + name)
            (project / '.tools').symlink_to(ROOT / '.tools', target_is_directory=True)
            (project / '.lake').mkdir()
            command(ROOT, folder, 'dependencies', ['cp', '-a', '--reflink=auto',
                str(ROOT / '.lake/packages'), str(project / '.lake/packages')])
            source = project / mutation['source']
            if case != 'correct':
                original = source.read_text()
                if original.count(mutation['old']) != 1:
                    raise ValueError('mutation must change exactly one actual Rust expression')
                source.write_text(original.replace(mutation['old'], mutation['new'], 1))
            expected_inputs = dict(inputs)
            expected_inputs[mutation['source']] = sha(source)

            # extract.py requires the allocated target to exist. It belongs to
            # this source copy; no case shares its own-package build artifacts.
            prepare_extract_target(project)

            # Only the private source is writable in this extraction namespace.
            # Host tools remain read-only; the target is private to this case.
            command(project, folder, 'extract', ['bwrap', '--ro-bind', '/', '/',
                '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp',
                '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
                '--bind', str(project), str(VIRTUAL_ROOT), '--chdir', str(VIRTUAL_ROOT),
                '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL_ROOT / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS',
                '--', sys.executable, str(HERE / 'extract.py'), '--profile', 'request',
                '--output', 'artifacts/extraction'])
            extraction = project / 'artifacts/extraction'
            generated = extraction / 'request/generated'
            extraction_record = json.loads((extraction / 'result.json').read_text())
            if extraction_record.get('success') is not True:
                raise ValueError('actual extraction incomplete')
            installed = {}
            for path in generated.rglob('*'):
                if path.is_file() and '_Template' not in path.name:
                    relative = LIBRARY / path.relative_to(generated)
                    shutil.copy2(path, project / relative)
                    installed[str(relative)] = sha(path)
            shutil.copy2(extraction / 'request/original.llbc', project / LIBRARY / 'original.llbc')
            installed[str(LIBRARY / 'original.llbc')] = sha(extraction / 'request/original.llbc')
            expected_inputs.update(installed)

            names = config['positive'] if case == 'correct' else config['negative']
            targets = ['SandboxModel.Lifecycle.Protocol.' + name for name in names]
            command(project, folder, 'build', ['lake', 'build', 'SandproofStage1Request', *targets])
            extra = []
            if case == 'correct':
                expected_audits = config['audits']
            else:
                relative = 'artifacts/' + config['template'] + '.lean'
                template = ROOT / HERE / (config['template'] + '.lean.in')
                shutil.copy2(template, project / relative)
                extra.append(relative)
                expected_inputs[relative] = sha(template)
                expected_audits = config['negative_audits']
            # Reuse the full component's physical import closure and strict
            # kernel/service/audit checks, including every transitive helper.
            actual = formal.check_actual(project, names, 'request', folder, env, command,
                                         expected_audits, extra)
            for name, digest in expected_inputs.items():
                if sha(project / name) != digest:
                    raise ValueError('unexpected source/proof/generated change: ' + name)
            record['cases'][case] = dict(complete=True, **actual,
                rust_sha256=sha(source), installed_generated=installed,
                contract_sha256=sha(project / PROOFS / (config['contract'] + '.lean')),
                witness_sha256={n: sha(project / PROOFS / (n + '.lean')) for n in config['witnesses']},
                extraction_record_sha256=sha(extraction / 'result.json'))
            (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if record['cases']['correct']['contract_sha256'] != record['cases'][mutant]['contract_sha256']:
            raise ValueError('correct and wrong versions used different properties')
        if record['cases']['correct']['witness_sha256'] != record['cases'][mutant]['witness_sha256']:
            raise ValueError('correct and wrong versions used different witnesses')
        if snapshot() != inputs:
            raise ValueError('source changed during the formal checks')
        record['complete'] = True
    except Exception as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--property', choices=PROPERTIES, default='replay')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.resolve(), args.property)
    print(json.dumps({k: result.get(k) for k in
                     ['kind', 'complete', 'native_checks', 'full_acceptance', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
