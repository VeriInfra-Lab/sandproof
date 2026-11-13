"""Current actual ID and two Rust mutants, using one complete value contract."""
from pathlib import Path
import argparse
import fcntl
import importlib.util
import json
import os
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-fixed-identity')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


support = module('current_fixed_identity_support', ROOT/HERE/'support.py')
sha, resolver, checker = support.sha, support.load.resolver, support.load.checker


def run(output, variant, data, inputs, modes, prepare_only):
    output.mkdir(mode=0o700)
    started = time.monotonic()
    rec = dict(kind='actual-fixed-identity-development', variant=variant, complete=False,
        prepared=False, capture_complete=False, property_rejected=False, commands=[],
        generated_bodies_modified=False, native_checks_run=False, shared_acceptance=False,
        full_v3=False, current_rust_recaptured=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
        LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    def command(project, folder, label, argv):
        now = time.monotonic()
        log = folder/(label+'.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(label=label, argv=argv, cwd=str(project),
            exit_code=done.returncode, seconds=time.monotonic()-now, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec, indent=2)+'\n')
        if done.returncode:
            raise ValueError('compiler/kernel/service failure; not property rejection: '+label)
    try:
        assert support.inventory() == data
        rec['library_review'] = support.library_review(data)
        disk = os.statvfs(ROOT)
        available, required = disk.f_bavail*disk.f_frsize, 64*2**30+2*858993459
        rec['resource_admission'] = dict(available_bytes=available, required_bytes=required,
            original_floor_bytes=64*2**30, declared_budget_bytes=2*858993459)
        if available < required:
            raise ValueError('original resource floor refused before compiler')
        rec.update(inputs=inputs, input_modes=modes,
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            source_tree=subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=ROOT, text=True).strip())
        project = output/'source'
        project.mkdir(mode=0o700)
        expected = dict(inputs)
        for name, digest in inputs.items():
            path = project/name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, path)
            if sha(path) != digest:
                raise ValueError('source changed during copy: '+name)
        (project/'.tools').symlink_to(support.runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(support.runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        recipe = data['variants'][variant]
        rec['private_source_derivation'] = support.mutate(project, recipe['mutation'])
        with (project/support.SOURCE).open('a') as stream:
            stream.write((ROOT/HERE/'root.rs.in').read_text())
        expected[support.SOURCE] = sha(project/support.SOURCE)
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for key, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            assert sha(ROOT/'.tools/aeneas'/suffix) == fixed[key]['binary_sha256']
        rec['tools'] = fixed
        ir = Path('artifacts/fixed-identity.llbc')
        argv = [str(support.runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo',
            '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default']
        for name in support.INCLUDE:
            argv += ['--include', name]
        for name in support.OPAQUE:
            argv += ['--opaque', name]
        for name in support.ROOTS:
            argv += ['--start-from', name]
        argv += ['--dest-file', str(support.runtime.VIRTUAL/ir), '--', '--manifest-path',
            'verification/experiments/stage1-protocol/inbox-record/Cargo.toml', '--lib',
            '--locked', '--offline', '--target', fixed['target']]
        command(project, output, 'charon', support.capacity.wrapper(project)+argv)
        raw = json.loads((project/ir).read_text())
        rec['call_binding'] = support.binding.admit(raw, support.ROOTS, support.INCLUDE, support.OPAQUE)
        rec['llbc_sha256'] = sha(project/ir)
        generated = output/'generated'
        generated.mkdir()
        namespace = 'SandproofStage1CreateAttachment'
        command(project, output, 'aeneas', [str(ROOT/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json', '-namespace',
            namespace+'Rust', '-subdir', namespace, '-dest', str(generated), str(project/ir)])
        translation = json.loads((generated/'translation.json').read_text())
        rec['translation'] = translation
        functions = [{key:f[key] for key in ('lean_name', 'rust_name', 'is_opaque', 'lean_file')}
            for f in translation['functions']]
        if functions != recipe['generated_functions']:
            raise ValueError('actual complete generated function inventory differs')
        external = {f['lean_name'].removeprefix(namespace+'Rust.') for f in translation['functions']
            if f['is_opaque']}
        if external != set(data['external_functions']):
            raise ValueError('unreviewed external function inventory: '+repr(external))
        actual = [f for f in translation['functions'] if f['rust_name'] ==
            'sandproof_controller_service::controller::identity::sandbox_id']
        if len(actual) != 1 or actual[0]['is_opaque']:
            raise ValueError('actual identity missing or opaque')
        rec['raw_generated'] = {str(p.relative_to(generated)):sha(p)
            for p in generated.rglob('*') if p.is_file()}
        for leaf in ['Types.lean', 'Funs.lean']:
            target = project/support.GEN/leaf
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(generated/namespace/leaf, target)
            expected[str(target.relative_to(project))] = sha(target)
        rec.update(capture_complete=True, current_rust_recaptured=True)
        expected.update(support.install(project, variant, data))
        seeds = [str(support.PROOFS/(stem+'.lean')) for stem in recipe['seeds']]
        files = sorted(resolver.closure(project, seeds, active_view='lakefile.toml'))
        audits = [a for name in files for a in re.findall(r'^#audit_controller_proof (\S+)$',
            (project/name).read_text(), re.M)]
        if len(audits) != len(set(audits)):
            raise ValueError('duplicate audit names')
        if variant != 'actual' and str(support.PROOFS/'CreateFixedIdentity.lean') in files:
            raise ValueError('mutant imports correct identity correspondence')
        if recipe.get('expected_files') is not None and files != recipe['expected_files']:
            raise ValueError('actual file inventory differs')
        if recipe.get('expected_audits') is not None and audits != recipe['expected_audits']:
            raise ValueError('actual audit inventory differs')
        rec.update(files={name:sha(project/name) for name in files}, audits=audits,
            common_contract_sha256=sha(project/support.PROOFS/'CreateFixedIdentityModel.lean'),
            common_binding_sha256=sha(project/support.GEN/'FixedFormatBinding.lean'))
        command(project, output, 'build', ['lake', 'build',
            *['SandboxModel.Lifecycle.Protocol.'+stem for stem in recipe['seeds']]])
        rec['prepared'] = True
        if not prepare_only:
            physical = output/'actual-files'
            physical.mkdir()
            rec['actual'] = checker.actual_files(project, files, physical, env, command)
            assert rec['actual']['files'] == rec['files'] and rec['actual']['audits'] == audits
            rec.update(complete=True, property_rejected=variant != 'actual')
        if any(sha(project/name) != digest for name, digest in expected.items()):
            raise ValueError('private inputs/generated body/proof changed')
        if any(('100755' if (project/name).stat().st_mode & 0o111 else '100644') != mode
               for name, mode in modes.items()):
            raise ValueError('private source mode changed')
        if support.library_review(data) != rec['library_review']:
            raise ValueError('selected fixed library sources changed during this view')
        rec['private_derived_inputs'] = {n:h for n,h in expected.items() if inputs.get(n) != h}
    except BaseException as error:
        rec.update(complete=False, property_rejected=False, error=repr(error), traceback=traceback.format_exc())
    finally:
        rec['original_inputs_unchanged'] = support.load.base.snapshot(ROOT) == inputs
        rec['original_modes_unchanged'] = all(
            ('100755' if (ROOT/name).stat().st_mode & 0o111 else '100644') == mode
            for name, mode in modes.items())
        if not rec['original_inputs_unchanged'] or not rec['original_modes_unchanged']:
            rec.update(complete=False, prepared=False, property_rejected=False)
        rec['execution_seconds'] = time.monotonic()-started
        (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')
    return rec


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--lock-fd', type=int)
    parser.add_argument('--variant', choices=('all', 'actual', 'empty-namespace', 'truncate-number'), default='all')
    parser.add_argument('--prepare-only', action='store_true', help='capture/build only; never a complete formal result')
    args = parser.parse_args()
    os.umask(0o077)
    output = args.output.absolute()
    output.mkdir(parents=True, mode=0o700)
    batch = dict(kind='actual-fixed-identity-development-batch', complete=False, prepared=False,
        variants={}, native_checks_run=False, shared_acceptance=False, full_v3=False,
        prepare_only=args.prepare_only)
    waiting = time.monotonic()
    try:
        inherited = args.lock_fd is not None
        lock = os.fdopen(os.dup(args.lock_fd), 'r+') if inherited else args.lock.open('r+')
        with lock:
            identity = os.fstat(lock.fileno())
            expected = args.lock.stat()
            if (identity.st_dev, identity.st_ino) != (expected.st_dev, expected.st_ino):
                raise ValueError('inherited lock is a different file')
            fcntl.flock(lock, fcntl.LOCK_EX | (fcntl.LOCK_NB if inherited else 0))
            batch['lock_wait_seconds'] = time.monotonic()-waiting
            batch['lock'] = dict(inherited_same_open_description=inherited,
                path=str(args.lock), device=identity.st_dev, inode=identity.st_ino)
            started = time.monotonic()
            data = support.inventory()
            resolver.ROOT, resolver.MODULES = ROOT, ('stage1-protocol',)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output/'runtime-cache'))
            fixed_runtime = resolver.runtime()
            batch['fixed_runtime_before'] = fixed_runtime
            registry = support.load.native.registry_module.registry()
            inputs = support.load.base.snapshot(ROOT)
            modes = {n:('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') for n in inputs}
            contracts, bindings = set(), set()
            for variant in data['variants'] if args.variant == 'all' else [args.variant]:
                result = run(output/variant, variant, data, inputs, modes, args.prepare_only)
                batch['variants'][variant] = dict(result,
                    result_sha256=sha(output/variant/'result.json'))
                if not (result['prepared'] if args.prepare_only else result['complete']):
                    raise ValueError('actual identity view failed: '+variant)
                contracts.add(result['common_contract_sha256'])
                bindings.add(result['common_binding_sha256'])
            assert len(contracts) == len(bindings) == 1
            after = resolver.runtime()
            if after != fixed_runtime or support.load.native.registry_module.registry() != registry:
                raise ValueError('fixed runtime/library changed')
            batch.update(complete=not args.prepare_only, prepared=True, registry=registry,
                fixed_runtime_after=after, common_contract_sha256=next(iter(contracts)),
                common_binding_sha256=next(iter(bindings)), execution_seconds=time.monotonic()-started)
            batch['all_three_variants_complete'] = (batch['complete'] and
                set(batch['variants']) == {'actual', 'empty-namespace', 'truncate-number'})
    except BaseException as error:
        batch.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    (output/'result.json').write_text(json.dumps(batch, indent=2)+'\n')
    print(json.dumps({k:batch.get(k) for k in ['complete', 'prepared', 'execution_seconds', 'error']}), flush=True)
    raise SystemExit(not (batch['prepared'] if args.prepare_only else batch['complete']))
