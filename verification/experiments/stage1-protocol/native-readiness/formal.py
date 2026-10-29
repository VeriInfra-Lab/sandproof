"""Re-extract and check the original native snapshot checker and real status mutant."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/native-readiness')
PARENT = HERE.parent/'inbox-record'
GENERATED = Path('verification/aeneas/stage1-protocol/native-readiness/SandproofNativeReadiness')
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('native_completion_source_inputs', ROOT/HERE.parent/'schema-leaves/formal.py')
checker = module('native_completion_actual_checker', ROOT/HERE.parent/'formal.py')
resolver = module('native_completion_original_closure', ROOT/'tools/proof_batch.py')
preparation = module('native_completion_prepare', ROOT/HERE/'prepare.py')
admission = module('native_completion_admission', ROOT/HERE/'admission.py')
library = module('native_completion_library', ROOT/HERE/'library.py')
sha = base.sha


def wrapper(project):
    # Same readonly-home/private-source projection as the original successful
    # preflight. Explicit runtime seeds support the existing two-host setup.
    runtime = module('native_completion_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
    directories = [Path.home(), runtime.tool_root(ROOT), runtime.package_root(ROOT)]
    for key in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
        if os.environ.get(key):
            seed = Path(os.environ[key]).resolve(strict=True)
            if not seed.is_dir():
                raise ValueError('missing fixed runtime seed: '+key)
            directories.append(seed)
    return ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
        '--tmpfs', '/tmp', '--tmpfs', '/home',
        *[part for path in dict.fromkeys(directories) for part in ('--ro-bind', str(path), str(path))],
        '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
        '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL/'artifacts/extract-target'),
        '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
        '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']


def run(output, variant, require_native=False, prepare_only=False):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='native-readiness-formal-development', variant=variant, complete=False,
        prepared=False, ning_acceptance=False, shared_acceptance=False, full_v3=False,
        production_modified=False, original_snapshot_facts_proved=False,
        new_external_symbols=[], commands=[])
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
            raise ValueError('command failed: '+str(log))

    try:
        if require_native and prepare_only:
            raise ValueError('preparation is not native/formal acceptance')
        inventory = preparation.inventory(ROOT)
        native_result = None
        if require_native:
            sys.path.insert(0, str(ROOT))
            from tools.acceptance_receipts import validate_native, native_case, digest
            validate_native(Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']), scope='stage1-protocol')
            native_case('functional', ROOT, None, scope='stage1-protocol')
            receipt = native_case('stage1-protocol-native-readiness', ROOT, None, scope='stage1-protocol')
            evidence = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])/'commands'/digest(receipt['key'])/'evidence'
            native_result = json.loads((evidence/'result.json').read_text())
            if (native_result.get('kind') != 'native-readiness-native-development'
                    or native_result.get('complete') is not True
                    or set(native_result.get('variants', {})) != set(preparation.VARIANTS)):
                raise ValueError('incomplete original/wrong/restored native evidence')
            record['native_receipt'] = receipt
        record['library_binding'] = library.check(ROOT)
        inputs = base.snapshot(ROOT)
        record.update(inputs=inputs, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool differs: '+name)
        record['tools'] = fixed
        record['registry_lock'] = 'Cargo.lock' if require_native else str(PARENT/'Cargo.lock')
        registry = module('native_completion_existing_registry', ROOT/HERE.parent/'envelope-writer/native.py')
        registry.HERE = Path('.') if require_native else PARENT
        libraries = registry.registry()
        record['registry'] = libraries
        if native_result is not None:
            admitted = module('native_completion_native_admission', ROOT/HERE.parent/'inbox-lookup/native_admission.py')
            admitted.admit(record, native_result, inputs, libraries, ROOT, Path(record['registry_lock']))
        project = output/'source'
        project.mkdir(mode=0o700)
        expected = dict(inputs)
        for name, digest in inputs.items():
            target = project/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, target)
            if sha(target) != digest:
                raise ValueError('source changed while copying: '+name)
        runtime = module('native_completion_execution', ROOT/HERE.parent/'formal_runtime.py')
        (project/'.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        changes, prepared = preparation.prepare(project, variant)
        expected.update(changes)
        record['preparation'] = prepared
        if native_result is not None:
            native_variant = 'original' if variant == 'correct' else variant
            if any(native_result['variants'][native_variant]['inputs'].get(n) != h for n,h in changes.items()):
                raise ValueError('native and formal original business source differs')
        raw_path = Path('artifacts/native-readiness.llbc')
        argv = [str(VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                '--error-on-warnings', '--sysroot', 'default']
        for name in admission.INCLUDE:
            argv += ['--include', name]
        for name in admission.ROOTS:
            argv += ['--start-from', name]
        argv += ['--dest-file', str(VIRTUAL/raw_path), '--', '--manifest-path', str(PARENT/'Cargo.toml'),
                 '--lib', '--locked', '--offline', '--target', fixed['target']]
        command(project, output, 'charon', wrapper(project)+argv)
        raw = json.loads((project/raw_path).read_text())
        record['original_ir'] = dict(sha256=sha(project/raw_path), admission=admission.admit_ir(raw))
        generated = output/'generated'
        generated.mkdir()
        command(project, output, 'aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', 'SandproofNativeReadinessRust', '-subdir', 'SandproofNativeReadiness',
            '-dest', str(generated), str(project/raw_path)])
        record['translation'] = admission.admit_generated(generated, variant)
        command(project, output, 'admission-regressions', [sys.executable, str(HERE/'admission_tests.py'),
            '--llbc', str(raw_path), '--generated', str(generated), '--variant', variant])
        record['raw_generated'] = {str(p.relative_to(generated)):sha(p) for p in generated.rglob('*') if p.is_file()}
        (project/GENERATED).mkdir(parents=True, exist_ok=True)
        for name in ('Funs.lean','Types.lean'):
            shutil.copy2(generated/'SandproofNativeReadiness'/name, project/GENERATED/name)
            expected[str(GENERATED/name)] = sha(project/GENERATED/name)
        shutil.copy2(ROOT/HERE/'FunsExternal.lean.in', project/GENERATED/'FunsExternal.lean')
        expected[str(GENERATED/'FunsExternal.lean')] = sha(project/GENERATED/'FunsExternal.lean')
        lake = project/'lakefile.toml'
        text = lake.read_text()
        import tomllib
        registrations = [v for v in tomllib.loads(text).get('lean_lib', [])
                         if v.get('name') == 'SandproofNativeReadiness']
        wanted = dict(name='SandproofNativeReadiness', srcDir='verification/aeneas/stage1-protocol/native-readiness')
        if registrations and registrations != [wanted]:
            raise ValueError('ambiguous private library registration')
        if not registrations:
            lake.write_text(text+'\n[[lean_lib]]\nname = "SandproofNativeReadiness"\nsrcDir = "verification/aeneas/stage1-protocol/native-readiness"\n')
        expected['lakefile.toml'] = sha(lake)
        stems = ['NativeCompletionModel','NativeCompletionValues']
        stems += ['NativeCompletionProofs' if variant == 'correct' else 'NativeCompletionWrongStatus']
        for stem in stems:
            target = Path('SandboxModel/Lifecycle/Protocol')/(stem+'.lean')
            shutil.copy2(ROOT/HERE/(stem+'.lean.in'), project/target)
            expected[str(target)] = sha(project/target)
        files = resolver.closure(project, ['SandboxModel/Lifecycle/Protocol/'+s+'.lean' for s in stems],
                                 active_view='lakefile.toml')
        declared = [a for f in files for a in re.findall(r'^#audit_controller_proof (\S+)$', (project/f).read_text(), re.M)]
        reviewed = [a for s in stems for a in inventory['providers'][s]['audits']]
        if sorted(declared) != sorted(reviewed) or len(declared) != len(set(declared)):
            raise ValueError('complete reviewed audit closure differs')
        record.update(prepared=True, prepared_files=files, prepared_audits=declared)
        if not prepare_only:
            command(project, output, 'build', ['lake','build',*['SandboxModel.Lifecycle.Protocol.'+s for s in stems]])
            actual = output/'actual-files'
            actual.mkdir()
            checked = checker.actual_files(project, sorted(files), actual, env, command)
            if sorted(checked['audits']) != sorted(declared):
                raise ValueError('incomplete actual audit results')
            record['actual'] = checked
            record['complete'] = True
        if any(sha(project/n) != h for n,h in expected.items()) or base.snapshot(ROOT) != inputs:
            raise ValueError('source/prepared input changed during checking')
        if registry.registry() != libraries:
            raise ValueError('original dependency sources changed during checking')
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=('correct','ignore-status'), default='correct')
    parser.add_argument('--prepare-only', action='store_true')
    parser.add_argument('--require-native', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.variant, args.require_native, args.prepare_only)
    print(json.dumps({k:result.get(k) for k in ('kind','variant','prepared','complete','seconds','error')}))
    raise SystemExit(not (result['complete'] or (args.prepare_only and result['prepared'] and 'error' not in result)))
