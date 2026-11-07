"""Bind the real five native/four generated source-live views to public receipts."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
HERE = ROOT/'verification/experiments/stage1-protocol/source-live'
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(HERE))
import support
import native


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def check_native(folder, source, verify_root):
    """Recheck original commands, complete tests and exact rejection locations."""
    result = json.loads((folder/'result.json').read_text())
    support.require(result.get('complete') is True and result.get('source_head') == source['commit']
        and result.get('input_tool_library_and_log_postguards_complete') is True,
        'source-live native did not finish on its original source')
    views = tuple(native.FAILURES)
    support.require(tuple(result.get('views', {})) == views and
        result.get('assertion_inventory') == sorted(native.test_names()),
        'missing or extra native source-live view/assertion')
    commands = result.get('commands', [])
    phases = ('format', 'build', 'list', 'tests', 'clippy')
    support.require([(c['view'], c['label']) for c in commands] ==
        [(v, phase) for v in views for phase in phases], 'native command inventory differs')
    original = (ROOT/support.SOURCE).read_text()
    for view in views:
        row = result['views'][view]
        body = (support.restore(support.mutate(original, 'ignore-startup'), 'ignore-startup')
                if view == 'restored' else support.mutate(original, view))
        project = verify_root/view
        path = project/support.SOURCE
        path.parent.mkdir(parents=True)
        path.write_text(body+'\n'+(HERE/'native-tests.rs.in').read_text())
        support.require(row.get('complete') is True and row.get('target_initially_absent') is True
            and row.get('business_source_sha256') == hashlib.sha256(body.encode()).hexdigest()
            and row.get('test_names') == sorted(native.test_names())
            and row.get('source_inputs', {}).get(support.SOURCE) == support.sha(path),
            'native actual source/assertions differ: '+view)
        for command in (c for c in commands if c['view'] == view):
            log = folder/view/(command['label']+'.log')
            expected = 101 if command['label'] == 'tests' and native.FAILURES[view] else 0
            support.require(not log.is_symlink() and log.is_file()
                and support.sha(log) == command['log_sha256'] and command['exit_code'] == expected,
                'missing/changed/failed native command: '+view+':'+command['label'])
        tests = next(c for c in commands if c['view'] == view and c['label'] == 'tests')
        checked = native.assertions(folder/view/'tests.log', native.FAILURES[view], project, tests['exit_code'])
        support.require(all(row.get(k) == v for k, v in checked.items()),
                        'native exact contract rejection differs: '+view)
        listing = (folder/view/'list.log').read_text().splitlines()
        names = {line.removesuffix(': test').split('::')[-1] for line in listing
                 if line.endswith(': test') and native.PREFIX in line}
        support.require(names == native.test_names(), 'native executable test inventory differs')
        artifacts = row.get('artifacts', [])
        support.require(len(artifacts) == 2 and all(a.get('files') and
            a.get('message', {}).get('fresh') is False for a in artifacts),
            'fresh native library/program identities missing')
        support.require(row.get('actual_executable', {}).get('sha256') in {
            h for a in artifacts for h in a['files'].values()}, 'executed ELF identity missing')
    return result


def run(output, mode, scope, lock=None, lock_fd=None):
    output.mkdir(parents=True, mode=0o700)
    support.require(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == mode, 'wrong source-live host role')
    builder = load('source_live_committed_archive', ROOT/'verification/experiments/stage1-protocol/archive.py')
    original = receipt = None
    if mode == 'formal':
        from tools.acceptance_receipts import digest, native_case, validate_native
        cache = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])
        validate_native(cache, scope=scope)
        native_case('functional', ROOT, None, scope=scope)
        receipt = native_case('source-live', ROOT, None, scope=scope)
        folder = cache/'commands'/digest(receipt['key'])/'evidence'
        original = builder.verify(folder/'source.tar', folder/'source-manifest.json')
        check_native(folder/'native', original, output/'native-readback')
    source = builder.build(ROOT, output/'source.tar', output/'source-manifest.json')
    if original is not None:
        relevant = set(receipt['key']['rust']) | set(receipt['key']['rules'])
        for files in receipt['key']['contracts'].values():
            relevant.update(files)
        support.require(all(original['files'].get(n) == source['files'].get(n)
            for n in relevant if n in original['files'] or n in source['files']),
            'source-live native consumed source differs from formal candidate')
    argv = [sys.executable, str(HERE/('native.py' if mode == 'native' else 'formal.py')),
            '--output', str(output/mode)]
    fds = ()
    if mode == 'formal':
        if lock is None:
            lock = output/'component.lock'
            lock.touch(exist_ok=False)
        argv += ['--lock', str(lock)]
        if lock_fd is not None:
            argv += ['--lock-fd', str(lock_fd)]
            fds = (lock_fd,)
    subprocess.run(argv, cwd=ROOT, stdin=subprocess.DEVNULL, check=True, pass_fds=fds)
    result = json.loads((output/mode/'result.json').read_text())
    support.require(result.get('complete') is True, 'original source-live component incomplete')
    if mode == 'native':
        check_native(output/'native', source, output/'native-readback')
    current, _ = builder.committed_inputs(ROOT)
    support.require(current == {n:source[n] for n in ('schema', 'commit', 'tree', 'files')},
                    'source-live committed source changed during checking')
    result.update(source_archive_sha256=source['archive_sha256'], source_head=source['commit'],
                  public_scope=scope, whole_V3=False, whole_system=False)
    (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('native', 'formal'))
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--scope', choices=('all', 'stage1-protocol', 'stage1-source-live'),
                        default='stage1-source-live')
    parser.add_argument('--lock', type=Path)
    parser.add_argument('--lock-fd', type=int)
    args = parser.parse_args()
    os.umask(0o077)
    run(args.output.resolve(), args.mode, args.scope, args.lock, args.lock_fd)


if __name__ == '__main__':
    main()
