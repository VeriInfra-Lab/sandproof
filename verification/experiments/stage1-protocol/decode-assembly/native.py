"""Run the original full-field assertion against real original/swapped/restored Rust.

Development evidence only. A source-admission rejection is not a Lean theorem.
The previously delivered real Writer formal counterexample remains unchanged.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

import support as admit

ROOT, HERE = admit.ROOT, admit.HERE
sha = admit.base.sha


def run(output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='decode-assembly-native-development', complete=False,
                  ning_acceptance=False, shared_acceptance=False, full_v3=False, commands=[])
    try:
        inputs = admit.base.snapshot(ROOT)
        registry_module = admit.registry_module
        registry = registry_module.registry()
        admit.schema.inventory(ROOT)
        record['library_inputs'] = admit.libraries()
        record.update(inputs=inputs, registry=registry, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(), tools={})
        for tool, arguments in [('cargo', ['-V']), ('rustc', ['-Vv']), ('clippy-driver', ['--version'])]:
            path = Path(subprocess.check_output(['rustup', 'which', tool], text=True).strip())
            record['tools'][tool] = dict(path=str(path), sha256=sha(path),
                version=subprocess.check_output([str(path), *arguments], text=True))
        record['variants'] = {}
        for variant in ('original', admit.MUTANT, 'restored'):
            folder = output / variant
            project = folder / 'source'
            project.mkdir(parents=True, mode=0o700)
            expected = dict(inputs)
            for name, wanted in inputs.items():
                target = project / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, target)
                if sha(target) != wanted:
                    raise ValueError('source changed while copying')
            if variant == admit.MUTANT:
                record['mutation'] = admit.mutate(project)
                expected[admit.MUTATION_PATH] = record['mutation']['after']
            env = dict(os.environ, CARGO_TARGET_DIR=str(folder / 'target'), CARGO_NET_OFFLINE='true',
                       CARGO_BUILD_JOBS='4', PYTHONDONTWRITEBYTECODE='1')
            manifest = str(HERE.parent / 'envelope-schema/Cargo.toml')
            for label, args in [('test', ['--lib', '--message-format=json',
                    'tests::actual_five_branch_fields_and_roundtrips', '--', '--exact', '--nocapture']),
                    ('clippy', ['--all-targets', '--', '-D', 'warnings'])]:
                command = ['cargo', label, '--manifest-path', manifest, '--locked', '--offline', *args]
                log = folder / (label + '.log')
                begin = time.monotonic()
                with log.open('xb') as stream:
                    result = subprocess.run(command, cwd=project, env=env, stdin=subprocess.DEVNULL,
                        stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
                row = dict(variant=variant, argv=command, seconds=time.monotonic()-begin,
                           exit_code=result.returncode, log_sha256=sha(log))
                record['commands'].append(row)
                if label == 'test':
                    artifacts = []
                    text = log.read_text()
                    for line in text.splitlines():
                        try:
                            message = json.loads(line)
                        except ValueError:
                            continue
                        if message.get('reason') == 'compiler-artifact' and message['target']['name'] in (
                                'sandproof_protocol', 'sandproof_envelope_schema_check'):
                            artifacts.append(message)
                    if len(artifacts) != 2 or any(a['fresh'] for a in artifacts):
                        raise ValueError('actual protocol library/test not newly compiled')
                    row['artifacts'] = [dict(message=a, files={n: sha(Path(n)) for n in a['filenames']
                                                            if Path(n).is_file()}) for a in artifacts]
                    if variant == admit.MUTANT:
                        if (result.returncode != 101 or '0 passed; 1 failed; 0 ignored;' not in text or
                                'actual_five_branch_fields_and_roundtrips ... FAILED' not in text or
                                'assertion `left == right` failed' not in text):
                            raise ValueError('real swapped fields did not reach original full-field assertion')
                        row['property_rejected'] = True
                    elif result.returncode or '1 passed; 0 failed; 0 ignored;' not in text:
                        raise ValueError('original full-field native assertion failed')
                elif result.returncode:
                    raise ValueError('Clippy failed: ' + str(log))
            for name, wanted in expected.items():
                if sha(project / name) != wanted:
                    raise ValueError('variant source changed during native checks')
            record['variants'][variant] = dict(source_inputs=expected, native_checked=True)
        if inputs != admit.base.snapshot(ROOT) or registry != registry_module.registry():
            raise ValueError('source or registry changed during native checks')
        if record['library_inputs'] != admit.libraries():
            raise ValueError('library sources/features changed during native checks')
        record['complete'] = True
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute())
    print(json.dumps({k: result.get(k) for k in ('kind', 'complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
