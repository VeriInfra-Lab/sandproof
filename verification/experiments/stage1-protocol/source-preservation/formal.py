"""Recapture and check five real saved-fact views; never signs ning acceptance."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
BASE = '9c50da7cc9391518514f14405e6c779e5c5cbe2c'
LIBRARY = Path('verification/aeneas/stage1-protocol/source-preservation')
PROOFS = Path('SandboxModel/Lifecycle/Protocol/SourcePreservation')
ROOTS = ['crate::controller::lifecycle_inbox::source_work::retirement::preserves_saved_facts']

INCLUDE = ['sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service']
VIEWS = {'correct': ['Spec', 'Bytes', 'Equality', 'StateEquality', 'Core', 'Trace', 'Inhabited', 'Values', 'Examples', 'Witnesses'], 'drop-producer': ['Spec', 'Bytes', 'Equality', 'Inhabited', 'Values', 'Examples', 'NegativeProducer'], 'drop-consumer': ['Spec', 'Bytes', 'Equality', 'Inhabited', 'Values', 'Examples', 'NegativeConsumer'], 'plan-binding-only': ['Spec', 'Bytes', 'Equality', 'Inhabited', 'Values', 'Examples', 'NegativePlan'], 'forget-settled': ['Spec', 'Bytes', 'Equality', 'Inhabited', 'Values', 'Examples', 'NegativeSettled']}
FRAGMENTS = {}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


native = module('source_preservation_native', HERE / 'native.py')
runtime = module('source_preservation_runtime', HERE.parent / 'formal_runtime.py')
batch = module('source_preservation_original_batch', ROOT / 'tools/proof_batch.py')
checker = module('source_preservation_actual_files', HERE.parent / 'formal.py')
sha, require, mode = native.sha, native.require, native.mode


def inventory():
    data = json.loads((HERE / 'inventory.json').read_text())
    require(data['source_base'] == BASE and data['views'] == VIEWS, 'view contract differs')
    for section in ('inputs', 'providers', 'templates', 'drivers'):
        for name, wanted in data[section].items():
            path = ROOT / name
            require(path.is_file() and not path.is_symlink() and sha(path) == wanted,
                    'reviewed input changed: ' + name)
    for case, files in data['generated'].items():
        for name, wanted in files.items():
            path = ROOT / LIBRARY / case / 'SandproofSourcePreservation' / name
            require(path.is_file() and not path.is_symlink() and sha(path) == wanted,
                    'literal generated input changed: ' + str(path))
    return data


def run(output, lock_path, lock_fd=None):
    output.mkdir(parents=True, mode=0o700)
    record = dict(kind='source-preservation-formal-development', complete=False, cases={}, commands=[],
                  ning_acceptance=False, shared_acceptance=False, full_v3=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
               CARGO_BUILD_JOBS='4', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    os.environ.update({name: env[name] for name in ('CARGO_NET_OFFLINE', 'LEAN_NUM_THREADS', 'LEAN_STACK_SIZE_KB')})

    def save():
        (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')

    def command(project, folder, label, argv):
        log = folder / (label + '.log')
        started = time.monotonic()
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-started, log=str(log), log_sha256=sha(log)))
        save()
        require(result.returncode == 0, 'compiler/kernel failure is not a property rejection: ' + str(log))

    waiting = time.monotonic()
    save()
    with (lock_path.open('r+') if lock_fd is None else os.fdopen(os.dup(lock_fd), 'r+')) as lock:
        actual_lock, expected_lock = os.fstat(lock.fileno()), lock_path.stat()
        require((actual_lock.st_dev, actual_lock.st_ino) == (expected_lock.st_dev, expected_lock.st_ino),
                'inherited descriptor identifies another lock')
        fcntl.flock(lock, fcntl.LOCK_EX | (fcntl.LOCK_NB if lock_fd is not None else 0))
        started = time.monotonic()
        inputs = modes = tools = libraries = None
        try:
            require(1 <= len(os.sched_getaffinity(0)) <= 4, 'proof CPU allocation differs')
            require(os.statvfs(ROOT).f_bavail * os.statvfs(ROOT).f_frsize >= 72 * 2**30,
                    'insufficient private capture space')
            head = native.source_guard()
            data = inventory()
            inputs = native.base.snapshot(ROOT)
            modes = {name: mode(ROOT / name) for name in inputs}
            batch.ROOT, batch.MODULES = ROOT, ('stage1-protocol',)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output / 'runtime-cache'))
            tools, libraries = batch.runtime(), native.registry.registry()
            record.update(inputs=inputs, input_modes=modes, runtime=tools, library_registry=libraries,
                          source_head=head,
                          source_base=BASE, cpus=sorted(os.sched_getaffinity(0)), lock_wait_seconds=started-waiting,
                          coordination_lock=dict(inherited=lock_fd is not None, path=str(lock_path),
                                                 device=actual_lock.st_dev, inode=actual_lock.st_ino))
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
            for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
                require(sha(ROOT / '.tools/aeneas' / suffix) == fixed[name]['binary_sha256'], 'fixed tool changed')
            for case, names in VIEWS.items():
                folder = output / case
                project = folder / 'source'
                project.mkdir(parents=True, mode=0o700)
                for name, wanted in inputs.items():
                    path = project / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(ROOT / name, path)
                    require(sha(path) == wanted, 'input changed during copy')
                (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
                (project / '.lake').mkdir()
                (project / '.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
                (project / 'artifacts/extract-target').mkdir(parents=True)
                expected = dict(inputs)
                prepared_modes = dict(modes)
                for name, template in FRAGMENTS.items():
                    path = project / name
                    original = path.read_bytes()
                    fragment = (HERE / template).read_bytes()
                    require(fragment not in original, 'probe already installed')
                    path.write_bytes(original + fragment)
                    require(path.read_bytes().removesuffix(fragment) == original, 'probe replaced production')
                    expected[name] = sha(path)
                mutation = native.MUTATIONS.get(case)
                if mutation:
                    name, before, after = mutation
                    path = project / name
                    text = path.read_text()
                    require(text.count(before) == 1, 'actual mutation site differs')
                    path.write_text(text.replace(before, after))
                    require(path.read_text() != text, 'actual Rust mutation did not occur')
                    expected[name] = sha(path)
                virtual = runtime.VIRTUAL
                wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
                    '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
                    '--bind', str(project), str(virtual), '--chdir', str(virtual), '--setenv',
                    'CARGO_TARGET_DIR', str(virtual / 'artifacts/extract-target'), '--setenv',
                    'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4', '--unsetenv',
                    'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
                argv = [str(virtual / '.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                        '--error-on-warnings', '--sysroot', 'default']
                for flag, values in [('--include', INCLUDE), ('--start-from', ROOTS)]:
                    for value in values:
                        argv += [flag, value]
                ir = project / 'artifacts/source-preservation.llbc'
                argv += ['--dest-file', str(virtual / 'artifacts' / ir.name), '--', '--manifest-path',
                         'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']]
                command(project, folder, 'charon', wrapper + argv)
                raw = json.loads(ir.read_text())
                opt = raw['translated']['options']
                require(raw['has_errors'] is False and opt['include'] == INCLUDE and opt['start_from'] == ROOTS and
                        not any(opt[k] for k in ('opaque', 'exclude', 'skip_borrowck', 'no_typecheck',
                                                'monomorphize', 'remove_unused_clauses')), 'capture is incomplete')
                require({t['key'] for t in raw['translated']['target_information']} == {fixed['target']}, 'target differs')
                funcs = [f for f in raw['translated']['fun_decls'] if f]
                require(len(funcs) == data['function_counts'][case], 'original declaration closure differs')
                owned = [f for f in funcs if f['item_meta']['name'][0]['Ident'][0] in INCLUDE]
                require(all(f['item_meta']['opacity'] == 'Transparent' and isinstance(f['body'], dict) and
                            'Structured' in f['body'] for f in owned), 'owned business body is hidden')
                methods = [f for f in owned if f['item_meta']['name'][-1] == {'Ident': ['preserves_saved_facts', 0]}]
                require(len(methods) == 1, 'actual saved-fact guard not reached')
                generated = folder / 'generated'
                generated.mkdir()
                command(project, folder, 'aeneas', [str(ROOT / '.tools/aeneas/bin/aeneas'), '-backend', 'lean',
                    '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json', '-namespace',
                    'SandproofSourcePreservationRust', '-subdir', 'SandproofSourcePreservation', '-dest', str(generated), str(ir)])
                target = LIBRARY / case / 'SandproofSourcePreservation'
                for name, wanted in data['generated'][case].items():
                    source = generated / 'SandproofSourcePreservation' / name
                    require(sha(source) == wanted, 'real generated body differs: ' + case + '/' + name)
                    shutil.copy2(source, project / target / name)
                shutil.copy2(HERE / 'FunsExternal.lean.in', project / target / 'FunsExternal.lean')
                external = str(target / 'FunsExternal.lean')
                expected[external] = sha(project / external)
                prepared_modes[external] = mode(project / external)
                for name in names:
                    destination = project / PROOFS / (name + '.lean')
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(HERE / (name + '.lean.in'), destination)
                    relative = str(destination.relative_to(project))
                    expected[relative] = sha(destination)
                    prepared_modes[relative] = mode(destination)
                lake = project / 'lakefile.toml'
                correct = '[[lean_lib]]\nname = "SandproofSourcePreservation"\nsrcDir = "' + str(LIBRARY / 'correct') + '"\n'
                text = lake.read_text()
                if 'name = "SandproofSourcePreservation"' not in text:
                    text += '\n' + correct
                require(text.count('name = "SandproofSourcePreservation"') == 1 and text.count(correct) == 1,
                        'different or duplicate public library entry')
                lake.write_text(text)
                seeded = sha(lake)
                selected = correct.replace(str(LIBRARY / 'correct'), str(LIBRARY / case))
                lake.write_text(text.replace(correct, selected))
                expected['lakefile.toml'] = sha(lake)
                command(project, folder, 'build', ['lake', 'build', str(PROOFS / names[-1]).replace('/', '.')])
                seeds = [str(PROOFS / (name + '.lean')) for name in names]
                seeds += [str(target / (name + '.lean')) for name in ('Types', 'FunsExternal', 'Funs')]
                files = sorted(batch.closure(project, seeds, active_view='lakefile.toml'))
                require(files == data['files'][case], 'actual physical file closure differs')
                require(case == 'correct' or not any(str(PROOFS / n) in f for f in files for n in ('Core.lean', 'StateEquality.lean', 'Trace.lean', 'Witnesses.lean')),
                        'wrong view imports a correct proof')
                audits = [a for f in files for a in re.findall(r'^#audit_controller_proof (\S+)$', (project / f).read_text(), re.M)]
                require(sorted(audits) == data['audits'][case], 'actual audit closure differs')
                actual = checker.actual_files(project, files, folder, env, command)
                require(sorted(actual['audits']) == data['audits'][case], 'complete actual proof result differs')
                for name, wanted in expected.items():
                    require(sha(project / name) == wanted and mode(project / name) == prepared_modes[name], 'prepared source/mode changed')
                record['cases'][case] = dict(complete=True, llbc=str(ir), llbc_sha256=sha(ir), actual=actual,
                    property_rejected=case != 'correct', files=actual['files'], audits=actual['audits'],
                    service_log_sha256=actual['service_log_sha256'],
                    source_inputs=expected, source_input_modes=prepared_modes,
                    seeded_from_public_correct_entry=True, seeded_correct_lake_sha256=seeded,
                    generated={name: sha(generated / 'SandproofSourcePreservation' / name) for name in data['generated'][case]})
                save()
            record.update(complete=True, all_five_views_complete=True)
        except BaseException as error:
            record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
        try:
            require(inputs is not None and native.base.snapshot(ROOT) == inputs and
                    all(mode(ROOT / n) == m for n, m in modes.items()), 'source admission/postguard differs')
            require(batch.runtime() == tools and native.registry.registry() == libraries, 'tools/library changed')
            require(all(sha(Path(c['log'])) == c['log_sha256'] for c in record['commands']), 'original log changed')
            record['source_tools_library_and_log_postguards_complete'] = True
        except BaseException as error:
            record.update(complete=False, postguard_error=repr(error))
        record['seconds'] = time.monotonic()-started
        (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--lock-fd', type=int, help='inherited open description of the held heavy lock')
    args = parser.parse_args()
    os.umask(0o077)
    cpus = sorted(os.sched_getaffinity(0))
    if len(cpus) > 4:
        os.sched_setaffinity(0, set(cpus[:4]))
    result = run(args.output.absolute(), args.lock.resolve(strict=True), args.lock_fd)
    print(json.dumps({k: result.get(k) for k in ('complete', 'seconds', 'error', 'postguard_error')}))
    raise SystemExit(0 if result['complete'] else 1)
