"""Reextract the adopted reply helper and check the complete three source views."""
from pathlib import Path
import argparse
import fcntl
import importlib.util
import json
import os
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/record-reply')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


support = module('actual_reply_formal_support', ROOT/HERE/'support.py')
capture = module('actual_reply_capture', ROOT/HERE/'capture.py')
sha = support.sha


def run(output, variant, data, prepare_only):
    output.mkdir(mode=0o700)
    started = time.monotonic()
    record = dict(kind='actual-adopted-record-reply-view', variant=variant, complete=False,
        prepared=False, commands=[], property_rejected=False, production_adopted=True,
        ordinary_checks_run=False, native_checks_run=False, shared_acceptance=False, full_v3=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
        LEAN_STACK_SIZE_KB='65536', CARGO_NET_OFFLINE='true')

    def command(project, folder, label, argv):
        begin = time.monotonic()
        log = folder/(label+'.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(label=label, argv=argv, cwd=str(project),
            exit_code=done.returncode, seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if done.returncode:
            raise ValueError('compile/kernel/service failure is not a counterexample: '+label)

    try:
        if support.inventory() != data:
            raise ValueError('reply input inventory changed before capture')
        captured = capture.run(output/'capture', variant)
        record.update(capture_result_sha256=sha(output/'capture/result.json'),
            capture_complete=captured['capture_complete'], translation_complete=captured['translation_complete'],
            source_commit=captured['source_commit'], inputs=captured['inputs'], input_modes=captured['input_modes'])
        if not captured['capture_complete'] or not captured['translation_complete']:
            raise ValueError('actual adopted reply capture failed')
        project = output/'capture/source'
        generated = output/'capture/generated'
        expected = dict(captured['private_inputs'])
        expected.update(support.install(project, variant, data, generated))
        recipe = data['variants'][variant]
        files, audits = support.proof_inventory(project, data['common_proofs']+recipe['proofs'])
        index = json.loads((ROOT/HERE/'generated-index.json').read_text())['variants'][variant]
        if files != index['expected_files'] or audits != index['expected_audits']:
            raise ValueError('complete reply file/audit closure differs')
        forbidden = {'RecordReplyCore', 'RecordReplyWitnesses'}
        if variant != 'actual' and any(Path(name).stem in forbidden for name in files):
            raise ValueError('wrong reply view imports the correct theorem')
        record.update(project=str(project), files={name: sha(project/name) for name in files}, audits=audits,
            generated_bodies_modified=False, no_axiom_template_imported=True,
            common_contract_sha256=sha(project/support.PROOFS/'RecordReplyModel.lean'),
            common_values_sha256=sha(project/support.PROOFS/'RecordReplyValues.lean'),
            common_recorded_sha256=sha(project/support.PROOFS/'Recorded.lean'),
            common_library_sha256=sha(project/support.GEN/'FunsExternal.lean'))
        command(project, output, 'build', ['lake', 'build', *[
            'SandboxModel.Lifecycle.Protocol.'+name for name in data['common_proofs']+recipe['proofs']]])
        record['prepared'] = True
        if not prepare_only:
            actual = output/'actual-files'
            actual.mkdir()
            record['actual'] = support.checker.actual_files(project, files, actual, env, command)
            if record['actual']['files'] != record['files'] or record['actual']['audits'] != audits:
                raise ValueError('actual reply checks differ from declared full closure')
            support.reader._saved_physical_view(record['actual'], project,
                actual/'physical-files', record['commands'])
            record.update(complete=True, property_rejected=variant != 'actual',
                original_kernel_service_dependencies_read_back=True)
        if any(sha(project/name) != digest for name, digest in expected.items()):
            raise ValueError('private reply inputs or generated bodies changed')
        if any(support.mode(project/name) != value for name, value in captured['input_modes'].items()):
            raise ValueError('private reply source mode changed')
        record['private_derived_inputs'] = {name: digest for name, digest in expected.items()
            if captured['inputs'].get(name) != digest}
    except BaseException as error:
        record.update(complete=False, property_rejected=False, error=repr(error), traceback=traceback.format_exc())
    record['execution_seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


def batch(output, prepare_only):
    started = time.monotonic()
    record = dict(kind='actual-adopted-record-reply-complete-development-batch', complete=False,
        prepared=False, variants={}, prepare_only=prepare_only, production_adopted=True,
        ordinary_checks_run=False, native_checks_run=False, shared_acceptance=False, full_v3=False)
    try:
        if not os.sched_getaffinity(0) <= {0, 1, 2, 3}:
            raise ValueError('reply task exceeds its existing CPU admission')
        disk = os.statvfs(ROOT)
        if disk.f_bavail*disk.f_frsize < 64*2**30+2*4*2**30:
            raise ValueError('insufficient admitted proof workspace space')
        data = support.inventory()
        support.resolver.ROOT, support.resolver.MODULES = ROOT, ('stage1-protocol',)
        os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output/'runtime-cache'))
        before = support.resolver.runtime()
        # Admit the actual root Cargo.lock archives and unpacked compiler sources first.
        registry = support.registry_module.registry()
        inputs = support.base.snapshot(ROOT)
        modes = {name: support.mode(ROOT/name) for name in inputs}
        record.update(fixed_runtime_before=before, inputs=inputs, input_modes=modes,
            registry=registry, registry_source_admitted_before_capture=True)
        same = {key: set() for key in ('common_contract_sha256', 'common_values_sha256',
            'common_recorded_sha256', 'common_library_sha256')}
        for variant in support.CASES:
            view = run(output/variant, variant, data, prepare_only)
            record['variants'][variant] = dict(view, result_sha256=sha(output/variant/'result.json'))
            if not (view['prepared'] if prepare_only else view['complete']):
                raise ValueError('actual reply view failed: '+variant)
            for key, values in same.items():
                values.add(view[key])
        if any(len(values) != 1 for values in same.values()):
            raise ValueError('reply source views did not use the same complete model/values/providers')
        if support.base.snapshot(ROOT) != inputs or any(support.mode(ROOT/name) != value for name, value in modes.items()):
            raise ValueError('original reply source or mode changed during checks')
        if support.resolver.runtime() != before:
            raise ValueError('fixed proof runtime changed during reply checks')
        if support.registry_module.registry() != registry or support.inventory() != data:
            raise ValueError('locked library sources or reply inputs changed during checks')
        record.update(complete=not prepare_only, prepared=True, fixed_runtime_after=before,
            same_complete_contract_values_recorded_and_library=True,
            source_worktree_inputs_and_modes_unchanged=True)
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['execution_seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--lock-fd', type=int)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    output = args.output.absolute()
    output.mkdir(parents=True, mode=0o700)
    inherited = args.lock_fd is not None
    lock = os.fdopen(os.dup(args.lock_fd), 'r+') if inherited else args.lock.open('r+')
    with lock:
        observed, expected = os.fstat(lock.fileno()), args.lock.stat()
        if (observed.st_dev, observed.st_ino) != (expected.st_dev, expected.st_ino):
            raise ValueError('reply inherited lock is a different file')
        fcntl.flock(lock, fcntl.LOCK_EX | (fcntl.LOCK_NB if inherited else 0))
        result = batch(output, args.prepare_only)
    print(json.dumps({key: result.get(key) for key in ('complete', 'prepared', 'execution_seconds', 'error')}), flush=True)
    raise SystemExit(not (result['prepared'] if args.prepare_only else result['complete']))
