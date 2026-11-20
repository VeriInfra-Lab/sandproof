"""Recapture four actual Rust views. Capture completion is never formal acceptance."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

import support

HERE = Path(__file__).resolve().parent
VIEWS = ('correct', 'skip-version', 'allow-expired', 'ignore-startup')
ROOTS = ['crate::controller::lifecycle_inbox::source_work::snapshot::live_native']
INCLUDE = ['sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service', 'core::tuple']
ERROR_OPAQUE = ['rusqlite::error::Error']
ROOT_NAME = [{'Ident': [x, 0]} for x in
             ('sandproof_controller_service', 'controller', 'lifecycle_inbox', 'source_work', 'snapshot', 'live_native')]
FLOOR = 64 * 2**30
WRITE_BUDGET = 4 * 4 * 2**30


def resource_admission(available):
    support.require(available >= FLOOR + WRITE_BUDGET, 'capture needs unchanged64GiB floor plus16GiB four-view budget')


def validate_ir(raw, target):
    support.require(raw.get('has_errors') is False, 'compiler/IR failure is not a property rejection')
    translated = raw['translated']
    options = translated['options']
    support.require(options['include'] == INCLUDE and options['start_from'] == ROOTS and
                    options['opaque'] == ERROR_OPAQUE and options['exclude'] == [] and
                    all(options[k] is False for k in
                    ('skip_borrowck', 'no_typecheck', 'monomorphize', 'remove_unused_clauses')),
                    'incomplete or altered capture options')
    support.require({x['key'] for x in translated['target_information']} == {target}, 'capture target differs')
    functions = [f for f in translated['fun_decls'] if f]
    owned = [f for f in functions if f['item_meta']['name'][0].get('Ident', [None])[0] in INCLUDE]
    support.require(owned and all(f['item_meta']['opacity'] == 'Transparent' and
                    isinstance(f.get('body'), dict) and list(f['body']) == ['Structured'] for f in owned),
                    'owned business body is hidden')
    roots = [f for f in owned if f['item_meta']['name'] == ROOT_NAME]
    support.require(len(roots) == 1 and len(roots[0]['signature']['inputs']) == 5,
                    'actual five-input live_native root not reached')
    guards = [f for f in owned if f['item_meta']['name'][:2] ==
              [{'Ident': ['sandbox_controller_core', 0]}, {'Ident': ['source_observation', 0]}]
              and f['item_meta']['name'][-1] == {'Ident': ['allows', 0]}]
    support.require(len(guards) == 1, 'actual V4 guard not reached')
    callbacks = [f for f in owned if f['item_meta']['name'][:6] == ROOT_NAME and
                 f['item_meta']['name'][-1] == {'Ident': ['call_once', 0]}]
    support.require(len(callbacks) == 1, 'actual business callback not reached')
    return dict(function_count=len(functions), owned_transparent_functions=len(owned),
                actual_root_id=roots[0]['def_id'], actual_V4_guard_id=guards[0]['def_id'],
                actual_callback_id=callbacks[0]['def_id'], actual_root_metadata=roots[0]['item_meta'])


def input_admission(root):
    head, _ = support.source_guard(root)
    data = json.loads((HERE / 'inventory-capture.json').read_text())
    support.require(data['source_base'] == support.BASE and tuple(data['views']) == VIEWS,
                    'capture source/view contract differs')
    for section in ('providers', 'drivers', 'proof_templates'):
        for name, wanted in data[section].items():
            path = (root if section == 'providers' else HERE) / name
            support.require(path.is_file() and not path.is_symlink() and support.sha(path) == wanted,
                            'reviewed capture input changed: ' + name)
    original = (root / support.SOURCE).read_text()
    views = {v: hashlib.sha256(support.mutate(original, 'original' if v == 'correct' else v).encode()).hexdigest()
             for v in VIEWS}
    return dict(kind='source-live-four-view-capture-preflight-not-proof', complete=False,
                input_admission_complete=True, capture_complete=False, source_head=head,
                source_base=support.BASE, views=views, formal_acceptance=False,
                formal_counterexamples_reached=False, ning_acceptance=False, full_v3=False)


def run(root, output, lock_path, lock_fd=None):
    output.mkdir(parents=True, mode=0o700)
    record = dict(kind='source-live-four-view-capture-preflight-not-proof', complete=False,
                  capture_complete=False, cases={}, commands=[], formal_acceptance=False,
                  formal_counterexamples_reached=False, ning_acceptance=False, shared_acceptance=False,
                  full_v3=False, cache_or_evidence_removed=False)
    started = time.monotonic()
    prepared = []
    inputs = modes = tools = libraries = batch = registry = None
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
               CARGO_BUILD_JOBS='4', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')

    def save():
        (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')

    def command(project, folder, label, argv):
        log = folder / (label + '.log')
        begin = time.monotonic()
        row = dict(argv=argv, cwd=str(project), log=str(log), completed=False)
        record['commands'].append(row)
        save()
        with log.open('xb') as stream:
            process = subprocess.Popen(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                       stdout=stream, stderr=subprocess.STDOUT)
            row['pid'] = process.pid
            save()
            code = process.wait()
        row.update(exit_code=code, seconds=time.monotonic()-begin, completed=True,
                   log_sha256=support.sha(log))
        save()
        support.require(code == 0, 'compiler/translator failure is not a property rejection: ' + label)

    def allocated():
        return sum(p.lstat().st_blocks * 512 for p in output.rglob('*') if p.is_file() and not p.is_symlink())

    try:
        support.require(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal',
                        'actual capture belongs to the existing proof-host role')
        support.require(Path.cwd().resolve() == root, 'actual capture must run in its admitted repository')
        support.require(1 <= len(os.sched_getaffinity(0)) <= 4, 'capture CPU allocation differs')
        with (lock_path.open('r+') if lock_fd is None else os.fdopen(os.dup(lock_fd), 'r+')) as lock:
            actual, wanted = os.fstat(lock.fileno()), lock_path.stat()
            support.require((actual.st_dev, actual.st_ino) == (wanted.st_dev, wanted.st_ino),
                            'capture inherited descriptor identifies another lock')
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            free = shutil.disk_usage(root).free
            record['resource'] = dict(floor_bytes=FLOOR, write_budget_bytes=WRITE_BUDGET,
                                      free_before=free, shared_floor_lowered=False)
            resource_admission(free)
            admitted = input_admission(root)
            fixed = json.loads((root / 'verification/foundations-environment.json').read_text())
            runtime = support.module('source_live_fixed_runtime', root /
                                     'verification/experiments/stage1-protocol/formal_runtime.py')
            base = support.module('source_live_capture_snapshot', root /
                                  'verification/experiments/stage1-protocol/schema-leaves/formal.py')
            registry = support.module('source_live_capture_registry', root /
                                      'verification/experiments/stage1-protocol/envelope-writer/native.py')
            registry.HERE = Path('.')
            error = support.module('source_live_existing_error_admission', root /
                                   'verification/experiments/stage1-protocol/inbox-load/admission.py')
            support.require(error.OPAQUE == ERROR_OPAQUE, 'existing external error boundary differs')
            batch = support.module('source_live_original_runtime', root / 'tools/proof_batch.py')
            batch.ROOT, batch.MODULES = root, ('stage1-protocol',)
            os.environ.update({k: env[k] for k in ('CARGO_NET_OFFLINE', 'LEAN_NUM_THREADS', 'LEAN_STACK_SIZE_KB')})
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output / 'runtime-cache'))
            installation = runtime.tool_root(root)
            for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
                support.require(support.sha(installation / 'aeneas' / suffix) == fixed[name]['binary_sha256'],
                                'fixed extraction tool binary differs: ' + name)
            inputs = base.snapshot(root)
            modes = {n: support.mode(root / n) for n in inputs}
            tools, libraries = batch.runtime(), registry.registry()
            record.update(source_head=admitted['source_head'], source_base=support.BASE,
                inputs=inputs, input_modes=modes, runtime=tools, library_registry=libraries,
                cpus=sorted(os.sched_getaffinity(0)), coordination_lock=dict(path=str(lock_path),
                inherited=lock_fd is not None, device=actual.st_dev, inode=actual.st_ino))
            original = (root / support.SOURCE).read_text()
            virtual = runtime.VIRTUAL
            for view in VIEWS:
                support.require(shutil.disk_usage(root).free >= FLOOR, 'capture free-space floor reached')
                folder = output / view
                project = folder / 'source'
                project.mkdir(parents=True, mode=0o700)
                for name, digest in inputs.items():
                    path = project / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(root / name, path)
                    support.require(support.sha(path) == digest, 'input changed during source copy')
                (project / '.tools').symlink_to(installation, target_is_directory=True)
                (project / 'artifacts/extract-target').mkdir(parents=True)
                body = support.mutate(original, 'original' if view == 'correct' else view)
                (project / support.SOURCE).write_text(body)
                expected = dict(inputs, **{support.SOURCE: support.sha(project / support.SOURCE)})
                prepared.append((project, expected))
                wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
                    '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
                    *runtime.tool_bind(root),
                    '--bind', str(project), str(virtual), '--chdir', str(virtual), '--setenv',
                    'CARGO_TARGET_DIR', str(virtual / 'artifacts/extract-target'), '--setenv',
                    'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4', '--unsetenv',
                    'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
                argv = [str(virtual / '.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                        '--error-on-warnings', '--sysroot', 'default']
                for flag, values in [('--include', INCLUDE), ('--start-from', ROOTS), ('--opaque', ERROR_OPAQUE)]:
                    for value in values:
                        argv += [flag, value]
                ir = project / 'artifacts/source-live.llbc'
                argv += ['--dest-file', str(virtual / 'artifacts/source-live.llbc'), '--', '--manifest-path',
                         'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']]
                support.require(shutil.disk_usage(root).free >= FLOOR and allocated() <= WRITE_BUDGET,
                                'prepared sources exceeded capture resource admission')
                command(project, folder, 'charon', wrapper + argv)
                metadata = validate_ir(json.loads(ir.read_text()), fixed['target'])
                original_ir_sha256 = support.sha(ir)
                support.require(shutil.disk_usage(root).free >= FLOOR and allocated() <= WRITE_BUDGET,
                                'Charon resource postguard failed; preserve all artifacts')
                generated = folder / 'generated'
                generated.mkdir()
                command(project, folder, 'aeneas', [str(installation / 'aeneas/bin/aeneas'), '-backend', 'lean',
                    '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json', '-namespace',
                    'SandproofSourceLiveRust', '-subdir', 'SandproofSourceLive', '-dest', str(generated), str(ir)])
                support.require(support.sha(ir) == original_ir_sha256, 'actual Charon IR changed during translation')
                rendered = generated / 'SandproofSourceLive/Funs.lean'
                support.require('controller.lifecycle_inbox.source_work.snapshot.live_native' in
                                re.findall(r'\bdef\s+([\w.]+)', rendered.read_text()), 'actual root erased from generated view')
                record['cases'][view] = dict(capture_complete=True, llbc=str(ir), llbc_sha256=original_ir_sha256,
                    metadata=metadata, source_inputs=expected, source_input_modes=modes,
                    generated={str(p.relative_to(generated)): support.sha(p) for p in generated.rglob('*') if p.is_file()},
                    generated_Lean_manually_changed=False, compiled_from_actual_Rust=True,
                    formal_counterexample_reached=False, property_rejected=False)
                support.require(shutil.disk_usage(root).free >= FLOOR and allocated() <= WRITE_BUDGET,
                                'capture resource postguard failed; preserve all artifacts')
                save()
            head, _ = support.source_guard(root)
            support.require(head == record['source_head'] and base.snapshot(root) == inputs and
                            all(support.mode(root / n) == m for n, m in modes.items()), 'root input/postguard differs')
            support.require(batch.runtime() == tools and registry.registry() == libraries, 'fixed tools/libraries changed')
            for project, expected in prepared:
                support.require(all(support.sha(project / n) == h and support.mode(project / n) == modes[n]
                                    for n, h in expected.items()), 'actual prepared source/modes changed')
            support.require(all(support.sha(Path(c['log'])) == c['log_sha256'] for c in record['commands']),
                            'original extraction log changed')
            for view, row in record['cases'].items():
                support.require(support.sha(Path(row['llbc'])) == row['llbc_sha256'] and
                                all(support.sha(output / view / 'generated' / n) == h
                                    for n, h in row['generated'].items()), 'original raw IR/generated files changed')
            record['resource']['free_after'] = shutil.disk_usage(root).free
            support.require(record['resource']['free_after'] >= FLOOR and allocated() <= WRITE_BUDGET,
                            'final capture capacity guard failed; preserve incomplete parent')
            record.update(capture_complete=True, source_mode_tool_library_and_log_postguards_complete=True)
    except Exception as error:
        record.update(capture_complete=False, error=repr(error), traceback=traceback.format_exc())
    finally:
        record['seconds'] = time.monotonic()-started
        record['allocated_output_bytes'] = allocated()
        (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=support.ROOT)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check-only', action='store_true')
    parser.add_argument('--lock', type=Path)
    parser.add_argument('--lock-fd', type=int)
    args = parser.parse_args()
    os.umask(0o077)
    root, output = args.repo.resolve(), args.output.resolve()
    lock = args.lock.resolve() if args.lock is not None else None
    os.chdir(root)
    if args.check_only:
        output.mkdir(parents=True, mode=0o700)
        result = input_admission(root)
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    else:
        if lock is None:
            parser.error('--lock is required for actual capture')
        result = run(root, output, lock, args.lock_fd)
    print(json.dumps({k: result.get(k) for k in ('input_admission_complete', 'capture_complete', 'formal_acceptance', 'error')}))
    raise SystemExit(0 if result.get('input_admission_complete') or result.get('capture_complete') else 1)
