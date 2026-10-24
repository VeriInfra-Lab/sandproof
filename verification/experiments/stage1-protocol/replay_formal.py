"""Local protocol properties on fresh extractions; not batch acceptance.

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
LIBRARIES = Path('verification/aeneas/stage1-protocol')
# Preserve the original extractor's source paths, without editing its output.
VIRTUAL_ROOT = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
POSITIVE = ['ReplayContract', 'ReplayEquality']
NEGATIVE = ['ReplayContract', 'Model', 'RequestValues']
AUDITS = ['bytes_eq', 'array_eq', 'vec_eq', 'ok_and', 'resources_eq',
          'create_eq', 'node_eq', 'key_eq', 'expected_eq', 'binding_eq',
          'operation_eq', 'request_eq', 'replay_complete',
          'replay_accepts_iff', 'replay_rejects_iff', 'bool_ne_model_exact']
REPLAY_AUDITS = ['SandboxModel.Stage1Protocol.Replay.' + n for n in AUDITS]
VALIDATION_AUDITS = ['SandboxModel.Stage1Protocol.Validation.' + n for n in
    ['resources_positive', 'checked_add_present', 'ok_unless', 'create_valid',
     'ok_ite', 'validate_refines', 'expected_success_iff', 'expected_error_iff',
     'validate_accepts_iff', 'validate_error_iff', 'validate_complete', 'validate_total']]
EXAMPLE_AUDITS = ['SandboxModel.Stage1Protocol.Validation.Examples.' + n for n in
                  ['create_accepted', 'unload_accepted', 'resume_accepted', 'destroy_accepted']]
REQUEST_AUDITS = ['SandboxModel.Stage1Protocol.Request.' + n for n in
                 ['unsupported_version_first', 'resume_order', 'resume_equal_refused',
                  'resume_nonempty_success', 'replay_complete_pair']]
RENEW_AUDITS = ['SandboxModel.Stage1Protocol.Renew.' + n for n in
               ['accepts_all_represented', 'absent_rejected', 'replay_iff',
                'changed_incarnation_rejected', 'changed_deadline_rejected',
                'changed_mode_rejected', 'accepted', 'negative_deadline_passes_guard']]
PROPERTIES = {
    'replay': dict(kind='stage1-full-replay-local-formal', mutation='key-only-replay',
        positive=POSITIVE, negative=NEGATIVE, common_files=[],
        contract='ReplayContract', template='NegativeFullReplay', audits=REPLAY_AUDITS,
        negative_audits=['mutant_accepts_changed_capture', 'key_only_not_full_replay',
                         'mutant_accepts_changed_renew_mode', 'renew_mode_not_full_replay']),
    'validation': dict(kind='stage1-request-validation-local-formal', mutation='same-incarnation',
        positive=[*POSITIVE, 'ValidationModel', 'Validation', 'ValidationExamples',
                  'Model', 'RequestValues', 'Request', 'Renew'],
        negative=['ValidationModel', 'Model', 'RequestValues'],
        common_files=['SandboxModel/Foundations/Model.lean'],
        contract='ValidationModel', template='NegativeValidation',
        audits=REPLAY_AUDITS + VALIDATION_AUDITS + EXAMPLE_AUDITS + REQUEST_AUDITS + RENEW_AUDITS,
        negative_audits=['mutant_accepts_same_incarnation', 'mutant_not_full_validation']),
}

CONFIRMATION_AUDITS = ['SandboxModel.Stage1Protocol.Retention.' + n for n in
    ['option_eq', 'checked_successor', 'validate_is_err',
     'version_matches', 'expected_is_err', 'confirmation_refines']]
RETENTION_AUDITS = CONFIRMATION_AUDITS + ['SandboxModel.Stage1Protocol.Retention.' + n for n in
    [
     'intent_eq', 'matches_refines', 'discard_refines',
     'discard_iff', 'preserve_current', 'preserve_pinned',
     'preserve_unapplied', 'preserve_pending', 'preserve_other_intent',
     'deleted_is_older', 'preserve_public', 'preserve_nonrenew',
     'preserve_missing_previous', 'preserve_changed_end', 'eligible_checkpoint_discarded',
     'different_center_preserved',
    ]]
PROPERTIES['retention'] = dict(kind='stage1-renewal-retention-local-formal',
    mutation='key-only-confirmation',
    positive=PROPERTIES['validation']['positive'] + ['RetentionModel', 'RetentionValues', 'Confirmation', 'Retention'],
    negative=['ReplayContract', 'ReplayEquality', 'ValidationModel', 'Validation',
              'Model', 'RequestValues', 'RetentionModel', 'RetentionValues', 'Confirmation'],
    common_files=['SandboxModel/Foundations/Model.lean'], contract='RetentionModel',
    template='NegativeRetention', audits=PROPERTIES['validation']['audits'] + RETENTION_AUDITS,
    negative_namespace='SandboxModel.Stage1Protocol.Retention.',
    negative_import_audits=REPLAY_AUDITS + VALIDATION_AUDITS + CONFIRMATION_AUDITS,
    negative_audits=['mutant_discards_foreign_center', 'key_only_not_retention'])




MANIFEST_LOOP_AUDITS = ['SandboxModel.Stage1Protocol.ManifestLoop.' + n for n in
    [
     'loop_done',
     'loop_cont',
     'path_body_first',
     'add_exact',
     'path_loop',
     'path_valid']]
MANIFEST_VALUE_AUDITS = ['SandboxModel.Stage1Protocol.Manifest.' + n for n in
    [
     'bytes_eq_self',
     'digest_eq_self',
     'deref_r',
     'distinct_first',
     'profile_first_step',
     'profile_loop',
     'profile_valid',
     'profile_eq_self',
     'literal_owned',
     'sub_exact',
     'state_file_length',
     'memory_file_length',
     'disk_file_length']]
MANIFEST_WITNESS_AUDITS = ['SandboxModel.Stage1Protocol.Manifest.' + n for n in
    [
     'files_step_0',
     'files_step_1',
     'files_step_2',
     'files_loop']]
MANIFEST_ENTRY_AUDITS = ['SandboxModel.Stage1Protocol.Manifest.' + n for n in
    [
     'unsupported_version_first',
     'changed_capture_key_first',
     'normal_manifest_accepted',
     'original_capture_contract']]
LENGTH_AUDITS = ['SandboxModel.Stage1Protocol.ManifestLength.' + n for n in
    [
     'bind_success',
     'file_length_success',
     'scan_unfold',
     'scan_error',
     'scan_end',
     'index_exact',
     'index_next',
     'scan_success',
     'validate_reaches_scan',
     'validate_lengths',
     'overflow_never_accepted',
     'zero_length_never_accepted',
     'scan_file_error',
     'scan_overflow']]
LENGTH_VALUE_AUDITS = ['SandboxModel.Stage1Protocol.ManifestLength.Values.' + n for n in
    [
     'state_length',
     'memory_length',
     'disk_length',
     'validate_enters_length_scan']]
LENGTH_EXAMPLE_AUDITS = ['SandboxModel.Stage1Protocol.ManifestLength.Examples.' + n for n in
    [
     'maximum_total_accepted',
     'overflow_refused',
     'zero_precedes_addition',
     'overflow_precedes_later_zero']]
MANIFEST_AUDITS = MANIFEST_LOOP_AUDITS + MANIFEST_VALUE_AUDITS + MANIFEST_WITNESS_AUDITS + MANIFEST_ENTRY_AUDITS
MANIFEST_LENGTH_AUDITS = LENGTH_AUDITS + LENGTH_VALUE_AUDITS + LENGTH_EXAMPLE_AUDITS
MANIFEST_NEGATIVE_IMPORT_AUDITS = MANIFEST_LOOP_AUDITS + MANIFEST_VALUE_AUDITS + LENGTH_VALUE_AUDITS
PROPERTIES['manifest-length'] = dict(kind='stage1-manifest-length-local-formal',
    profile='manifest', mutation='wrapping-manifest-length',
    positive=['Model', 'LoopWitness', 'ManifestValues', 'ManifestWitnesses', 'Manifest',
              'ManifestLengthModel', 'ManifestLength', 'ManifestLengthValues', 'ManifestLengthExamples'],
    negative=['Model', 'LoopWitness', 'ManifestValues', 'ManifestLengthModel', 'ManifestLengthValues'],
    common_files=['SandboxModel/Foundations/' + name + '.lean' for name in
                  ['Model', 'IdentifierModel', 'Representation', 'Identifiers', 'ByteArray']] +
                 ['verification/aeneas/foundations/SandproofFoundations/Funs.lean'],
    contract='ManifestLengthModel', template='NegativeManifestLength',
    audits=MANIFEST_AUDITS + MANIFEST_LENGTH_AUDITS,
    negative_namespace='SandboxModel.Stage1Protocol.ManifestLength.Negative.',
    negative_import_audits=MANIFEST_NEGATIVE_IMPORT_AUDITS,
    negative_audits=['mutant_wraps_total', 'wrapping_not_length_contract'])

FILES_AUDITS = ['SandboxModel.Stage1Protocol.ManifestFiles.' + n for n in
    ['disk_path_exact', 'vector_index_success', 'path_gate_success', 'file_shape_success',
     'scan_checks_file', 'scan_file_shapes', 'profile_count_bounds', 'validate_shape_preconditions',
     'validate_files', 'wrong_count_rejected', 'wrong_file_rejected', 'validate_layout_and_lengths',
     'file_roles_unique', 'state_memory_unique', 'every_drive_present']]
FILES_VALUE_AUDITS = ['SandboxModel.Stage1Protocol.ManifestFiles.Values.' + n for n in
    ['validate_fixed_profile', 'state_step', 'memory_step', 'finish_after_headers']]
FILES_EXAMPLE_AUDITS = ['SandboxModel.Stage1Protocol.ManifestFiles.Examples.' + n for n in
    ['normal_manifest_complete', 'missing_disk_refused', 'extra_disk_refused',
     'wrong_disk_id_refused', 'wrong_disk_path_refused']]
PROPERTIES['manifest-files'] = dict(kind='stage1-manifest-files-local-formal',
    profile='manifest', mutation='skip-disk-id',
    positive=PROPERTIES['manifest-length']['positive'] +
             ['ManifestFilesModel', 'ManifestFiles', 'ManifestFilesValues', 'ManifestFilesExamples'],
    negative=['Model', 'LoopWitness', 'ManifestValues', 'ManifestLengthModel',
              'ManifestFilesModel', 'ManifestFilesValues'],
    common_files=PROPERTIES['manifest-length']['common_files'],
    contract='ManifestFilesModel', template='NegativeManifestFiles',
    audits=PROPERTIES['manifest-length']['audits'] + FILES_AUDITS + FILES_VALUE_AUDITS + FILES_EXAMPLE_AUDITS,
    negative_namespace='SandboxModel.Stage1Protocol.ManifestFiles.Negative.',
    negative_import_audits=MANIFEST_LOOP_AUDITS + MANIFEST_VALUE_AUDITS + FILES_VALUE_AUDITS,
    negative_audits=['mutant_accepts_foreign_disk', 'skipped_disk_id_not_files_contract'])


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


def run(output, property_name='replay'):
    config = PROPERTIES[property_name]
    mutant = config['mutation']
    profile = config.get('profile', 'request')
    library = LIBRARIES / profile
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

            # Only the private source is writable in this extraction namespace.
            # Host tools remain read-only; the target is private to this case.
            command(project, folder, 'extract', ['bwrap', '--ro-bind', '/', '/',
                '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp',
                '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
                '--bind', str(project), str(VIRTUAL_ROOT), '--chdir', str(VIRTUAL_ROOT),
                '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL_ROOT / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS',
                '--', sys.executable, str(HERE / 'extract.py'), '--profile', profile,
                '--output', 'artifacts/extraction'])
            extraction = project / 'artifacts/extraction'
            generated = extraction / profile / 'generated'
            extraction_record = json.loads((extraction / 'result.json').read_text())
            if extraction_record.get('success') is not True:
                raise ValueError('actual extraction incomplete')
            installed = {}
            for path in generated.rglob('*'):
                if path.is_file() and '_Template' not in path.name:
                    relative = library / path.relative_to(generated)
                    if case == 'correct' and profile == 'manifest' and sha(ROOT / relative) != sha(path):
                        raise ValueError('fresh manifest generation differs from tracked output: ' + str(relative))
                    shutil.copy2(path, project / relative)
                    installed[str(relative)] = sha(path)
            raw = extraction / profile / 'original.llbc'
            if case == 'correct' and profile == 'manifest':
                sys.path.insert(0, str(ROOT / 'tools'))
                from check_foundations_extraction import llbc_digest
                if llbc_digest(json.loads(raw.read_text())) != llbc_digest(
                        json.loads((ROOT / library / 'original.llbc').read_text())):
                    raise ValueError('fresh complete manifest IR differs from tracked output')
            shutil.copy2(raw, project / library / 'original.llbc')
            installed[str(library / 'original.llbc')] = sha(raw)
            expected_inputs.update(installed)

            names = config['positive'] if case == 'correct' else config['negative']
            targets = ['SandboxModel.Lifecycle.Protocol.' + name for name in names]
            command(project, folder, 'build', ['lake', 'build', 'SandproofStage1' + profile.title(), *targets])
            files = ['SandboxModel/ProofAuditCommand.lean']
            files += config['common_files']
            files += [str(PROOFS / (name + '.lean')) for name in names]
            files += [str(p.relative_to(project)) for p in sorted((project / library).rglob('*.lean'))
                      if '_Template' not in p.name]
            files += ['verification/aeneas/foundations/SandproofFoundations/' + name + '.lean'
                      for name in ['TypesExternal', 'IdentifierLibrary', 'Types', 'FunsExternal']]
            if profile == 'request':
                files += ['verification/aeneas/network/SandproofNetwork/' + name + '.lean'
                          for name in ['Types', 'FunsExternal']]
            if case == 'correct':
                expected_audits = config['audits']
            else:
                relative = 'artifacts/' + config['template'] + '.lean'
                template = ROOT / HERE / (config['template'] + '.lean.in')
                shutil.copy2(template, project / relative)
                files.append(relative)
                expected_inputs[relative] = sha(template)
                expected_audits = config.get('negative_import_audits', []) + [config.get('negative_namespace', 'SandboxModel.Stage1Protocol.Request.') + n for n in
                                   config['negative_audits']]
            actual = formal.actual_files(project, files, folder, env, command)
            if sorted(actual['audits']) != sorted(expected_audits):
                raise ValueError('protocol property audit inventory differs')
            for name, digest in expected_inputs.items():
                if sha(project / name) != digest:
                    raise ValueError('unexpected source/proof/generated change: ' + name)
            record['cases'][case] = dict(complete=True, **actual,
                rust_sha256=sha(source), installed_generated=installed,
                contract_sha256=sha(project / PROOFS / (config['contract'] + '.lean')),
                extraction_record_sha256=sha(extraction / 'result.json'))
            (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if record['cases']['correct']['contract_sha256'] != record['cases'][mutant]['contract_sha256']:
            raise ValueError('correct and wrong versions used different properties')
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
