"""Reproduce local native evidence; the shared ning suite owns acceptance."""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import shutil
import subprocess
import time
import tomllib

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/envelope-budget')
spec = importlib.util.spec_from_file_location('budget_native_helpers', ROOT / HERE / 'formal.py')
formal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formal)


def registry_archives():
    """Check every registry crate against the unchanged Cargo.lock checksum."""
    lock = tomllib.loads((ROOT / HERE / 'Cargo.lock').read_text())
    cache = Path(os.environ.get('CARGO_HOME', str(Path.home() / '.cargo'))) / 'registry/cache'
    result = {}
    for package in lock['package']:
        if 'checksum' not in package:
            continue
        matches = list(cache.glob('*/' + package['name'] + '-' + package['version'] + '.crate'))
        if not matches or any(formal.sha(path) != package['checksum'] for path in matches):
            raise ValueError('missing or changed locked registry archive: ' + package['name'])
        result[package['name'] + '@' + package['version']] = package['checksum']
    return result


def run(output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='protocol4-envelope-budget-native-development', complete=False,
                  full_acceptance=False, commands=[])
    try:
        formal.inventory(ROOT)
        inputs = formal.snapshot(ROOT)
        archives = registry_archives()
        record.update(inputs=inputs, registry_archives=archives,
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            rustc=subprocess.check_output(['rustc', '-Vv'], cwd=ROOT, text=True),
            cargo=subprocess.check_output(['cargo', '-V'], cwd=ROOT, text=True))
        project = output / 'source'
        project.mkdir()
        for name, digest in inputs.items():
            target = project / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
            if formal.sha(target) != digest:
                raise ValueError('source changed during native preparation: ' + name)
        env = dict(os.environ, CARGO_TARGET_DIR=str(output / 'target'), CARGO_BUILD_JOBS='4',
                   CARGO_NET_OFFLINE='true')
        for label, args in [('test', []), ('clippy', ['--all-targets'])]:
            argv = ['cargo', label, '--manifest-path', str(HERE / 'Cargo.toml'),
                    '--locked', '--offline', *args]
            if label == 'clippy':
                argv += ['--', '-D', 'warnings']
            log = output / (label + '.log')
            with log.open('xb') as stream:
                done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
            record['commands'].append(dict(argv=argv, exit_code=done.returncode, log_sha256=formal.sha(log)))
            if done.returncode:
                raise ValueError('native command failed: ' + label)
        if inputs != formal.snapshot(ROOT) or any(formal.sha(project / n) != h for n, h in inputs.items()):
            raise ValueError('source changed during native checking')
        if archives != registry_archives():
            raise ValueError('registry archives changed during native checking')
        record.update(complete=True, sources_unchanged=True, registry_archives_unchanged=True)
    except Exception as error:
        record['error'] = repr(error)
    record['seconds'] = time.monotonic() - started
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute())
    print(json.dumps({k: result.get(k) for k in ['complete', 'seconds', 'error', 'full_acceptance']}))
    raise SystemExit(0 if result['complete'] else 1)
