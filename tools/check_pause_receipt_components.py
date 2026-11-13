"""Connect the current pause receipt to existing native/formal receipts.

Business preparation, extraction, kernel checks and file services remain in
the original component CLIs. This adapter supplies source/role/receipt binding;
it never substitutes a standalone value model for the actual worker route.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
HERE = ROOT / 'verification/experiments/pause-receipt'
sys.path.insert(0, str(ROOT))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def admit_native(cache):
    from tools.acceptance_receipts import native_case, validate_native
    validate_native(cache, scope='pause-receipt')
    native_case('functional', ROOT, None, scope='pause-receipt')
    return native_case('pause-receipt', ROOT, None, scope='pause-receipt')


def check_native_result(folder, source, archive_builder):
    result = json.loads((folder / 'result.json').read_text())
    if (result.get('complete') is not True or result.get('head') != source['commit']
            or result.get('archive_sha256') != source['archive_sha256']):
        raise ValueError('original native pause receipt did not finish on its archived source')
    driver = load('pause_receipt_original_native_cases', HERE / 'native.py')
    expected = [case[0] for case in driver.CASES]
    if (not expected or len(set(expected)) != len(expected)
            or [row['name'] for row in result.get('variants', [])] != expected):
        raise ValueError('original native pause receipt case inventory differs')
    if not result.get('commands'):
        raise ValueError('original native pause receipt command evidence is missing')
    fixture = HERE / 'tests.rs'
    expected_tests = sorted(driver.TEST_PREFIX + name for name in re.findall(
        r'#\[test\]\s+fn\s+(\w+)', fixture.read_text()))
    if (len(expected_tests) != 8 or result.get('tests') != expected_tests
            or result.get('fixture_sha256') != archive_builder.sha(fixture)):
        raise ValueError('original native receipt fixture or eight-test inventory changed')
    phases = ('format', 'fixture-format', 'build', 'assertions', 'clippy')
    roles = ('correct', *expected, 'restored')
    if [row.get('label') for row in result['commands']] != [
            role + '-' + phase for role in roles for phase in phases]:
        raise ValueError('original native receipt command inventory differs')
    for role in ('correct', 'restored'):
        item = result.get(role, {})
        if item.get('complete') is not True or item.get('observed_failures') != []:
            raise ValueError('original native receipt correct/restored case is incomplete')
    for item, case in zip(result['variants'], driver.CASES, strict=True):
        failures = sorted(driver.TEST_PREFIX + name for name in case[4])
        if (item.get('complete') is not True or item.get('build_exit') != 0
                or item.get('assertion_exit') != 101
                or item.get('observed_failures') != failures):
            raise ValueError('original wrong receipt did not reject the exact outcome-evidence property')
    for command in result['commands']:
        expected_exit = 101 if command['label'] in {
            name + '-assertions' for name in expected} else 0
        if command.get('timed_out') is not False or command.get('exit_code') != expected_exit:
            raise ValueError('native receipt command did not finish with its expected result')
        for stream in ('stdout', 'stderr'):
            path = folder / (command['label'] + '.' + stream)
            if (path.is_symlink() or not path.is_file()
                    or archive_builder.sha(path) != command[stream + '_sha256']):
                raise ValueError('original native pause receipt stream changed: ' + str(path))
    return result


def run(output, mode):
    output.mkdir(mode=0o700, parents=True)
    record = dict(kind='current-pause-receipt-component', scope='pause-receipt',
                  mode=mode, complete=False, whole_V4=False, whole_system=False)
    started = time.monotonic()
    try:
        if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != mode:
            raise ValueError('pause receipt requires its explicit ' + mode + ' host role')
        # Use the actual Stop dispatch/receipt CLI, never a historical model replay.
        driver = HERE / ('native.py' if mode == 'native' else 'replay.py')
        if driver.is_symlink() or not driver.is_file():
            raise ValueError('original current pause receipt CLI is missing: ' + str(driver))
        archive_builder = load('pause_receipt_existing_committed_archive',
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
                        raise ValueError('native pause receipt source differs from current formal source: ' + name)
        destination = output / mode
        argv = [sys.executable, str(driver), '--archive', str(archive),
                '--archive-sha256', source['archive_sha256'], '--head', source['commit'],
                '--output', str(destination), '--lock', str(output / 'component.lock')]
        if mode == 'formal':
            argv += ['--tools', os.environ.get('SANDPROOF_PROOF_TOOL_ROOT', str(ROOT / '.tools')),
                     '--packages', str(ROOT / '.lake/packages')]
            for role in ('correct', 'wrong'):
                value = os.environ.get('SANDPROOF_PAUSE_RECEIPT_'+role.upper()+'_CAPTURE')
                if not value:
                    raise ValueError('Original '+role+' receipt capture evidence is required')
                argv += ['--'+role+'-capture', value]
                project = os.environ.get('SANDPROOF_PAUSE_RECEIPT_'+role.upper()+'_PROJECT')
                if project:
                    argv += ['--'+role+'-project', project]
        record['argv'] = argv
        # Preserve the original CLI's kernel/audit stdout for the public total.
        subprocess.run(argv, cwd=ROOT, stdin=subprocess.DEVNULL, check=True)
        result = json.loads((destination / 'result.json').read_text())
        if (result.get('complete') is not True or result.get('head') != source['commit']
                or result.get('archive_sha256') != source['archive_sha256']):
            raise ValueError('original current pause receipt CLI is incomplete or bound to another source')
        if mode == 'native':
            checked = check_native_result(destination, source, archive_builder)
            record['native_cases'] = [row['name'] for row in checked['variants']]
        after, _ = archive_builder.committed_inputs(ROOT)
        if after != {name: source[name] for name in ('schema', 'commit', 'tree', 'files')}:
            raise ValueError('current pause receipt committed source changed during checking')
        if archive_builder.sha(archive) != source['archive_sha256']:
            raise ValueError('current pause receipt source archive changed')
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
