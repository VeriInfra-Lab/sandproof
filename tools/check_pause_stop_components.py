"""Connect the current pause stop to existing native/formal receipts.

Business preparation, extraction, kernel checks and file services remain in
the original component CLIs. This adapter supplies source/role/receipt binding;
it never substitutes a standalone value model for the actual worker route.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
HERE = ROOT / 'verification/experiments/pause-stop'
sys.path.insert(0, str(ROOT))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def admit_native(cache):
    from tools.acceptance_receipts import native_case, validate_native
    validate_native(cache, scope='pause-stop')
    native_case('functional', ROOT, None, scope='pause-stop')
    return native_case('pause-stop', ROOT, None, scope='pause-stop')


def check_native_result(folder, source, archive_builder):
    result = json.loads((folder / 'result.json').read_text())
    if (result.get('complete') is not True or result.get('head') != source['commit']
            or result.get('archive_sha256') != source['archive_sha256']):
        raise ValueError('original native pause stop did not finish on its archived source')
    driver = load('pause_stop_original_native_cases', HERE / 'native.py')
    expected = [case[0] for case in driver.CASES]
    if (not expected or len(set(expected)) != len(expected)
            or [row['name'] for row in result.get('variants', [])] != expected):
        raise ValueError('original native pause stop case inventory differs')
    if not result.get('commands'):
        raise ValueError('original native pause stop command evidence is missing')
    for command in result['commands']:
        for stream in ('stdout', 'stderr'):
            path = folder / (command['label'] + '.' + stream)
            if (path.is_symlink() or not path.is_file()
                    or archive_builder.sha(path) != command[stream + '_sha256']):
                raise ValueError('original native pause stop stream changed: ' + str(path))
    return result


def run(output, mode):
    output.mkdir(mode=0o700, parents=True)
    record = dict(kind='current-pause-stop-component', scope='pause-stop',
                  mode=mode, complete=False, whole_V4=False, whole_system=False)
    started = time.monotonic()
    try:
        if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != mode:
            raise ValueError('pause stop requires its explicit ' + mode + ' host role')
        # Use the actual Commit/Stop CLI, never a historical model replay.
        driver = HERE / ('native.py' if mode == 'native' else 'replay.py')
        if driver.is_symlink() or not driver.is_file():
            raise ValueError('original current pause stop CLI is missing: ' + str(driver))
        archive_builder = load('pause_stop_existing_committed_archive',
                               ROOT / 'verification/experiments/stage1-protocol/archive.py')
        native = None
        if mode == 'formal':
            from tools.acceptance_receipts import digest
            receipt = admit_native(Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']))
            evidence = (Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']) / 'commands'
                        / digest(receipt['key']) / 'evidence')
            native = archive_builder.verify(evidence / 'source.tar', evidence / 'source-manifest.json')
            checked = check_native_result(evidence / 'native', native, archive_builder)
            record['native_cases'] = [row['name'] for row in checked['variants']]
            record['native_source_archive_sha256'] = native['archive_sha256']
        archive, manifest = output / 'source.tar', output / 'source-manifest.json'
        source = archive_builder.build(ROOT, archive, manifest)
        record.update(head=source['commit'], source_archive_sha256=source['archive_sha256'],
                      source_manifest_sha256=archive_builder.sha(manifest))
        if native is not None:
            relevant = set(receipt['key']['rust']) | set(receipt['key']['rules'])
            for files in receipt['key']['contracts'].values():
                relevant.update(files)
            for name in sorted(relevant):
                if name in native['files'] or name in source['files']:
                    if native['files'].get(name) != source['files'].get(name):
                        raise ValueError('native pause stop source differs from current formal source: ' + name)
        destination = output / mode
        argv = [sys.executable, str(driver), '--archive', str(archive),
                '--archive-sha256', source['archive_sha256'], '--head', source['commit'],
                '--output', str(destination), '--lock', str(output / 'component.lock')]
        if mode == 'formal':
            argv += ['--tools', os.environ.get('SANDPROOF_PROOF_TOOL_ROOT', str(ROOT / '.tools')),
                     '--packages', str(ROOT / '.lake/packages')]
        record['argv'] = argv
        # Preserve the original CLI's kernel/audit stdout for the public total.
        subprocess.run(argv, cwd=ROOT, stdin=subprocess.DEVNULL, check=True)
        result = json.loads((destination / 'result.json').read_text())
        if (result.get('complete') is not True or result.get('head') != source['commit']
                or result.get('archive_sha256') != source['archive_sha256']):
            raise ValueError('original current pause stop CLI is incomplete or bound to another source')
        if mode == 'native':
            checked = check_native_result(destination, source, archive_builder)
            record['native_cases'] = [row['name'] for row in checked['variants']]
        after, _ = archive_builder.committed_inputs(ROOT)
        if after != {name: source[name] for name in ('schema', 'commit', 'tree', 'files')}:
            raise ValueError('current pause stop committed source changed during checking')
        if archive_builder.sha(archive) != source['archive_sha256']:
            raise ValueError('current pause stop source archive changed')
        record['complete'] = True
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        record['error'] = repr(error)
    record['seconds'] = time.monotonic() - started
    (output / 'component-result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('native', 'formal'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.mode)
    print(json.dumps(result))
    raise SystemExit(0 if result['complete'] else 1)


if __name__ == '__main__':
    main()
