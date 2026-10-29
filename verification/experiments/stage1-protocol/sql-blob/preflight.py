"""Capture original rusqlite blob conversions and params macro; no acceptance seal."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/sql-blob')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec); spec.loader.exec_module(value)
    return value


base = module('blob_original_source', ROOT/HERE.parent/'schema-leaves/formal.py')
runtime = module('blob_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
sha = base.sha


def canonical_packages(lock):
    rows = lock['package']

    def identity(row):
        return (row['name'], row['version'], row.get('source', ''))

    def dependency(name):
        parts = name.split()
        candidates = [row for row in rows if row['name'] == parts[0]
            and (len(parts) < 2 or row['version'] == parts[1])
            and (len(parts) < 3 or row.get('source') == parts[2].removeprefix('(').removesuffix(')'))]
        if len(candidates) != 1: raise ValueError('ambiguous lock dependency: '+name)
        return identity(candidates[0])

    result = {}
    for row in rows:
        normalized = dict(row)
        if 'dependencies' in row:
            normalized['dependencies'] = sorted(dependency(name) for name in row['dependencies'])
        result[identity(row)] = normalized
    return result


def run(output, mode):
    output.mkdir(parents=True, mode=0o700); started = time.monotonic()
    rec = dict(kind='original-sql-blob-tool-preflight', mode=mode, commands=[],
        capture_complete=False, translation_complete=False, proof_complete=False,
        property_rejection=False, models_imported=False, production_modified=False,
        library_modified=False, generated_bodies_modified=False, formal_acceptance=False,
        ning_acceptance=False, shared_acceptance=False, full_v3=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true')

    def command(project, label, argv):
        begin = time.monotonic(); log = output/(label+'.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec, indent=2)+'\n')
        if result.returncode: raise ValueError('fixed tool failed: '+label)

    try:
        inv = json.loads((ROOT/HERE/'inventory.json').read_text()); recipe = inv['modes'][mode]
        for name, digest in inv['source_inputs'].items():
            if sha(ROOT/name) != digest: raise ValueError('original source differs: '+name)
        for name, digest in inv['templates'].items():
            if sha(ROOT/HERE/name) != digest: raise ValueError('template differs: '+name)
        production_lock = tomllib.loads((ROOT/'Cargo.lock').read_text())
        isolated_lock = tomllib.loads((ROOT/HERE/'Cargo.lock').read_text())
        production_packages = canonical_packages(production_lock)
        if any(production_packages.get(key) != row for key, row in canonical_packages(isolated_lock).items()
               if row.get('source')):
            raise ValueError('isolated dependency resolution differs from production lock')
        cargo = Path(os.environ.get('CARGO_HOME', str(Path.home()/'.cargo')))
        providers = {}
        for name, digest in inv['provider_sources'].items():
            matches = [p/name for p in (cargo/'registry/src').iterdir() if p.is_dir() and (p/name).is_file()]
            if not matches or any(sha(p) != digest for p in matches):
                raise ValueError('fixed provider missing or different: '+name)
            providers[name] = dict(paths=[str(p) for p in matches], sha256=digest)
        rec['provider_sources'] = providers
        rec['isolated_dependency_resolution_matches_production'] = True
        rec['inputs'] = base.snapshot(ROOT)
        rec['input_modes'] = {n:('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') for n in rec['inputs']}
        rec['source_commit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool differs: '+name)
        rec['tools'] = fixed; rec['recipe'] = recipe
        project = output/'source'; project.mkdir(); expected = dict(rec['inputs'])
        for name, digest in rec['inputs'].items():
            dest = project/name; dest.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(ROOT/name, dest)
            if sha(dest) != digest: raise ValueError('input changed during copy: '+name)
        (project/'.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir(); (project/'.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        paths = [Path.home(), runtime.tool_root(ROOT), runtime.package_root(ROOT)]
        for key in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
            if os.environ.get(key): paths.append(Path(os.environ[key]).resolve(strict=True))
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home',
            *[a for p in dict.fromkeys(paths) for a in ('--ro-bind', str(p), str(p))],
            '--bind', str(project), str(runtime.VIRTUAL), '--chdir', str(runtime.VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(runtime.VIRTUAL/'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        args = [str(runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
        for name in recipe['include']: args += ['--include', name]
        for name in recipe['opaque']: args += ['--opaque', name]
        for name in recipe['roots']: args += ['--start-from', name]
        args += ['--dest-file', str(runtime.VIRTUAL/'artifacts/sql-blob.llbc'), '--', '--manifest-path',
            str(HERE/'Cargo.toml'), '--lib', '--locked', '--offline', '--target', fixed['target']]
        command(project, 'charon', wrapper+args)
        ir = project/'artifacts/sql-blob.llbc'; raw = json.loads(ir.read_text()); options = raw['translated']['options']
        if (raw.get('has_errors') is not False or options['start_from'] != recipe['roots']
                or options['include'] != recipe['include'] or options['opaque'] != recipe['opaque'] or options['exclude']
                or options['skip_borrowck'] or options['no_typecheck'] or options['monomorphize']
                or options['remove_unused_clauses']): raise ValueError('unreviewed capture')
        rec.update(capture_complete=True, llbc_sha256=sha(ir), ir_options=options)
        generated = output/'generated'; generated.mkdir()
        command(project, 'aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', 'SandproofSqlBlobRust', '-subdir', 'SandproofSqlBlob', '-dest', str(generated), str(ir)])
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
                rec.update(translation_complete=False, error='original source changed during preflight')
        if 'expected' in locals():
            rec['private_source_unchanged'] = all(sha(project/n) == h for n,h in expected.items())
            if not rec['private_source_unchanged']:
                rec.update(translation_complete=False, error='private inputs changed during preflight')
        if 'providers' in locals():
            rec['provider_sources_unchanged'] = all(sha(Path(p)) == row['sha256']
                for row in providers.values() for p in row['paths'])
            if not rec['provider_sources_unchanged']:
                rec.update(translation_complete=False, error='fixed providers changed during preflight')
        rec['seconds'] = time.monotonic()-started
        (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')
    return rec


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=('slice', 'params'), default='slice')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True, help='existing shared proof-heavy lock')
    args = parser.parse_args(); os.umask(0o077)
    with args.lock.open('r+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX); rec = run(args.output.absolute(), args.mode)
    print(json.dumps({k:rec.get(k) for k in ('capture_complete', 'translation_complete', 'proof_complete', 'property_rejection', 'error')}))
    raise SystemExit(not rec['translation_complete'])
