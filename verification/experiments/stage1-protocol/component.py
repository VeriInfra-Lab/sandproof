"""Connect one Stage1 component to the existing native/formal batch records."""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = ROOT / 'verification/experiments/stage1-protocol'
sys.path.insert(0, str(ROOT))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def admit_native(cache, root):
    from tools.acceptance_receipts import native_case, validate_native
    validate_native(cache)  # Original seal/receipt integrity, not current docs.
    native_case('functional', root, None)
    return native_case('stage1-protocol', root, None)


def run(output, native_only):
    output.mkdir(mode=0o700, parents=True)
    archive_builder = load('stage1_committed_archive', HERE / 'archive.py')
    archive, manifest = output / 'source.tar', output / 'source-manifest.json'
    original = archive_builder.build(ROOT, archive, manifest)
    record = dict(kind='stage1-protocol-batch-component', complete=False,
                  full_acceptance=False, source_manifest_sha256=archive_builder.sha(manifest))
    start = time.monotonic()
    try:
        if native_only:
            if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native':
                raise ValueError('ordinary component requires the native host role')
            allocated = Path(os.environ['CARGO_TARGET_DIR']).resolve(strict=True)
            target = allocated / 'stage1-protocol'
            for driver, args in [
                ('native.py', ['--output', output/'correct', '--target', target]),
                ('negative_native.py', ['--source-archive', archive,
                    '--output', output/'negative', '--target', target]),
            ]:
                subprocess.run([sys.executable, str(HERE/driver), *map(str, args)],
                               cwd=ROOT, check=True)
            for name in ('correct', 'negative'):
                if json.loads((output/name/'result.json').read_text()).get('success') is not True:
                    raise ValueError('native component incomplete: ' + name)
        else:
            if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
                raise ValueError('formal component requires the proof host role')
            from tools.acceptance_receipts import digest
            cache = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])
            receipt = admit_native(cache, ROOT)
            entry = cache/'commands'/digest(receipt['key'])
            evidence = entry/'evidence'
            native_archive = evidence/'source.tar'
            historical = archive_builder.verify(native_archive, evidence/'source-manifest.json')
            correct = json.loads((evidence/'correct/result.json').read_text())
            if (correct.get('success') is not True or correct.get('source_sha256') !=
                    {n: row['sha256'] for n, row in historical['files'].items()}):
                raise ValueError('native correct component differs from its historical full source archive')
            # native_case already checks the current Rust/rules/contracts/runtime
            # key. Independently connect its archived tracked inputs, including
            # types/execute bits. External inherited configs remain in that key.
            relevant = set(receipt['key']['rust']) | set(receipt['key']['rules'])
            for files in receipt['key']['contracts'].values():
                relevant.update(files)
            for name in sorted(relevant):
                if name in historical['files'] or name in original['files']:
                    if historical['files'].get(name) != original['files'].get(name):
                        raise ValueError('historical native input differs from current formal source: ' + name)
            record['native_source_manifest_sha256'] = archive_builder.sha(evidence/'source-manifest.json')
            record['native_source_archive_sha256'] = historical['archive_sha256']
            record['native_docs_scope'] = 'historical native snapshot; current docs require the functional seal'
            if [step['name'] for step in correct['steps']] != [
                    'format', 'extract-format', 'clean-own', 'build', 'test', 'clippy', 'docs', 'fmt-check']:
                raise ValueError('native positive required phase inventory differs')
            for step in correct['steps']:
                if (step['exit_code'] != 0 or archive_builder.sha(
                        evidence/'correct'/(step['name']+'.log')) != step['log_sha256']):
                    raise ValueError('native positive phase log or result differs')
            native_driver = load('stage1_positive_inventory', HERE/'native.py')
            integration = correct['integration_test_results']['current_envelope']
            if (sorted(integration['test_names']) != sorted(native_driver.ENVELOPE_TESTS)
                    or (integration['passed'], integration['failed'], integration['ignored']) !=
                        (len(native_driver.ENVELOPE_TESTS), 0, 0)
                    or integration['test_source_sha256'] != historical['files'][
                        'sandbox-protocol/tests/current_envelope.rs']['sha256']):
                raise ValueError('native positive integration inventory differs')
            formal = load('stage1_registered_formal', HERE/'formal.py')
            result = formal.replay(output/'formal', archive, evidence/'negative/result.json',
                                   native_archive, ['correct', *formal.NEGATIVE])
            if result.get('complete') is not True or result.get('all_cases_complete') is not True:
                raise ValueError('formal component did not finish all retained cases')
            record['formal'] = result
        after, _ = archive_builder.committed_inputs(ROOT)
        if after != {n: original[n] for n in ('schema', 'commit', 'tree', 'files')}:
            raise ValueError('component committed source changed during acceptance')
        if archive_builder.sha(archive) != original['archive_sha256']:
            raise ValueError('component source archive changed during acceptance')
        record['complete'] = True
    except Exception as error:
        record['error'] = repr(error)
    record['seconds'] = time.monotonic() - start
    (output/'component-result.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps({k: v for k, v in record.items() if k != 'formal'}))
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--native-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.native_only)
    raise SystemExit(0 if result['complete'] else 1)
