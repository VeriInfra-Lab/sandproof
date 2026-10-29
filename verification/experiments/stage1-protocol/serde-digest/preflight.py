"""Capture the original digest/array callbacks; a tool failure is never a property rejection."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/serde-digest')
AREA = HERE.parent
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


base = module('digest_original_sources', ROOT/AREA/'schema-leaves/formal.py')
runtime = module('digest_fixed_runtime', ROOT/AREA/'formal_runtime.py')
sha = base.sha


def run(output, mode):
    output.mkdir(parents=True, mode=0o700)
    rec = dict(kind='original-serde-digest-tool-preflight', mode=mode, commands=[],
        capture_complete=False, translation_complete=False, proof_complete=False,
        property_rejection=False, models_imported=False, production_modified=False,
        library_modified=False, generated_bodies_modified=False, formal_acceptance=False,
        ning_acceptance=False, shared_acceptance=False, full_v3=False)
    started = time.monotonic()
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true')

    def command(project, label, argv):
        begin = time.monotonic(); log = output/(label+'.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec, indent=2)+'\n')
        if result.returncode:
            raise ValueError('fixed tool failed: '+label)

    try:
        inv = json.loads((ROOT/HERE/'inventory.json').read_text()); recipe = inv['modes'][mode]
        for name, digest in inv['source_inputs'].items():
            if sha(ROOT/name) != digest:
                raise ValueError('reviewed original source changed: '+name)
        for name, digest in inv['templates'].items():
            if sha(ROOT/HERE/name) != digest:
                raise ValueError('thin root changed: '+name)
        cargo = Path(os.environ.get('CARGO_HOME', str(Path.home()/'.cargo')))
        registry = cargo/'registry/src'; providers = {}
        for name, digest in inv['provider_sources'].items():
            matches = [p/name for p in registry.iterdir() if p.is_dir() and (p/name).is_file()]
            if not matches or any(sha(p) != digest for p in matches):
                raise ValueError('fixed provider source missing or different: '+name)
            providers[name] = dict(paths=[str(p) for p in matches], sha256=digest)
        rec['provider_sources'] = providers
        rec['inputs'] = base.snapshot(ROOT)
        rec['input_modes'] = {n:('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') for n in rec['inputs']}
        rec['source_commit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool differs: '+name)
        rec['tools'] = fixed; rec['recipe'] = recipe
        project = output/'source'; project.mkdir()
        expected = dict(rec['inputs'])
        for name, digest in rec['inputs'].items():
            dest = project/name; dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, dest)
            if sha(dest) != digest:
                raise ValueError('source changed during copy: '+name)
        (project/'.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        root = project/AREA/'envelope-bytes/roots.rs.in'
        root.write_text(root.read_text().rstrip()+'\n'+(ROOT/HERE/recipe['root_file']).read_text())
        expected[str(root.relative_to(project))] = sha(root)
        manifest = project/AREA/'envelope-bytes/Cargo.toml'; text = manifest.read_text()
        original = 'serde_json = "=1.0.142"'
        if text.count(original) != 1:
            raise ValueError('ambiguous original feature configuration')
        manifest.write_text(text.replace(original, 'serde_json = { version = "=1.0.142", features = ["raw_value"] }'))
        expected[str(manifest.relative_to(project))] = sha(manifest)
        paths = [Path.home(), runtime.tool_root(ROOT), runtime.package_root(ROOT)]
        for key in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
            if os.environ.get(key):
                paths.append(Path(os.environ[key]).resolve(strict=True))
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home',
            *[a for p in dict.fromkeys(paths) for a in ('--ro-bind', str(p), str(p))],
            '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL/'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        args = [str(VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
        for name in recipe['include']: args += ['--include', name]
        for name in recipe['roots']: args += ['--start-from', name]
        args += ['--dest-file', str(VIRTUAL/'artifacts/digest-serde.llbc'), '--', '--manifest-path',
            str(AREA/'envelope-bytes/Cargo.toml'), '--lib', '--locked', '--offline', '--target', fixed['target']]
        command(project, 'charon', wrapper+args)
        ir = project/'artifacts/digest-serde.llbc'; raw = json.loads(ir.read_text())
        if raw.get('has_errors') is not False:
            raise ValueError('incomplete original capture')
        options = raw['translated']['options']
        if (options['start_from'] != recipe['roots'] or options['include'] != recipe['include']
                or options['opaque'] or options['exclude'] or options['skip_borrowck']
                or options['no_typecheck'] or options['monomorphize'] or options['remove_unused_clauses']):
            raise ValueError('unreviewed capture configuration')
        rec.update(capture_complete=True, llbc_sha256=sha(ir), ir_options=options)
        generated = output/'generated'; generated.mkdir()
        command(project, 'aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', 'SandproofDigestSerdeRust', '-subdir', 'SandproofDigestSerde',
            '-dest', str(generated), str(ir)])
        rec['translation_complete'] = True
        rec['functions'] = json.loads((generated/'translation.json').read_text())['functions']
    except BaseException as error:
        rec.update(error=repr(error), traceback=traceback.format_exc())
    finally:
        generated = output/'generated'
        rec['generated'] = {str(p.relative_to(generated)):sha(p) for p in generated.rglob('*') if p.is_file()}
        if 'inputs' in rec:
            rec['public_source_unchanged'] = base.snapshot(ROOT) == rec['inputs']
            rec['public_modes_unchanged'] = all(('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') == m
                for n,m in rec['input_modes'].items())
            if not rec['public_source_unchanged'] or not rec['public_modes_unchanged']:
                rec.update(translation_complete=False, error='original inputs changed during preflight')
        if 'expected' in locals():
            rec['private_source_unchanged'] = all(sha(project/n) == h for n,h in expected.items())
            if not rec['private_source_unchanged']:
                rec.update(translation_complete=False, error='private inputs changed during preflight')
        if 'providers' in locals():
            rec['provider_sources_unchanged'] = all(sha(Path(p)) == row['sha256']
                for row in providers.values() for p in row['paths'])
            if not rec['provider_sources_unchanged']:
                rec.update(translation_complete=False, error='fixed provider sources changed during preflight')
        rec['seconds'] = time.monotonic()-started
        (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')
    return rec


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=('digest', 'array-constructor', 'array-callback'), default='digest')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True, help='existing shared proof-heavy lock')
    args = parser.parse_args(); os.umask(0o077)
    with args.lock.open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        rec = run(args.output.absolute(), args.mode)
    print(json.dumps({k:rec.get(k) for k in ('capture_complete', 'translation_complete', 'proof_complete', 'property_rejection', 'error')}))
    raise SystemExit(not rec['translation_complete'])
