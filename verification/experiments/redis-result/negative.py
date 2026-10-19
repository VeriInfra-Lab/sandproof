"""Reject a freshly compiled/extracted real Rust early-unfence mutation.

The successful replay stays untouched. A new private candidate changes one
actual Rust assignment. Original Aeneas generates a separate module; the
original Results proofs are copied with only their module import renamed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def messages(path):
    found = []
    for line in path.read_text().splitlines():
        if line.startswith('{'):
            message = json.loads(line)
            if 'severity' in message:
                found.append(message)
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-base', type=Path, required=True)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--correct-replay', type=Path, required=True)
    parser.add_argument('--tool-root', type=Path, required=True)
    args = parser.parse_args()
    assets = Path(__file__).resolve().parent
    repository = assets.parents[2]
    sys.path.insert(0, str(repository))
    from tools.check_first_chain_extraction import check_tools
    from tools.check_foundations_extraction import llbc_digest
    check_tools(repository, args.tool_root)
    baseline = json.loads((args.correct_replay / 'evidence/replay.json').read_text())
    profile = json.loads((assets / 'candidate.json').read_text())
    if (baseline.get('exit_code') != 0 or not baseline.get('source_unchanged')
            or baseline['core_llbc_digest'] != profile['core_llbc_digest']
            or baseline['public_llbc_digest'] != profile['public_binding_llbc_digest']):
        raise ValueError('a successful exact correct replay is required')
    subprocess.run([sys.executable, str(assets / 'prepare.py'), '--source-base', str(args.source_base),
                    '--destination', str(args.destination)], check=True)
    root = args.destination / 'source'
    evidence = args.destination / 'evidence'
    evidence.mkdir(exist_ok=False)
    store = root / 'sandbox-center-store/src/store.rs'
    original = '        self.write_fenced = true;\n        self.connection = None;\n        Err(StoreError::OutcomeUnknown)'
    faulty = original.replace('= true;', '= false;')
    correct = store.read_text()
    if correct.count(original) != 1 or sha(store) != profile['candidate_files']['sandbox-center-store/src/store.rs']:
        raise ValueError('unique real fence_unknown assignment changed')
    store.write_text(correct.replace(original, faulty))
    before = {name: sha(root / name) for name in profile['candidate_files']}
    record = {'mutation': 'only actual fence_unknown sets write_fenced=false',
              'correct_store': profile['candidate_files']['sandbox-center-store/src/store.rs'],
              'mutated_store': sha(store), 'results': []}

    def run(stage, command, expected=0, environment=None, cwd=root):
        print('START', stage, flush=True)
        start = time.monotonic()
        output = evidence / (stage + '.log')
        with output.open('xb') as log:
            process = subprocess.Popen([str(x) for x in command], cwd=cwd, env=environment,
                                       stdout=log, stderr=subprocess.STDOUT)
            (evidence / (stage + '.pid')).write_text(str(process.pid) + '\n')
            code = process.wait()
        record['results'].append({'stage': stage, 'command': [str(x) for x in command],
                                  'exit_code': code, 'expected_exit': expected,
                                  'seconds': round(time.monotonic() - start, 3)})
        (evidence / 'negative.json').write_text(json.dumps(record, indent=2) + '\n')
        print('END', stage, code, record['results'][-1]['seconds'], flush=True)
        if code != expected:
            raise ValueError(stage + ' had unexpected exit status: ' + str(code))
        return output

    run('mutant-native', ['cargo', 'check', '--manifest-path', 'verification/extract-storage/Cargo.toml',
                         '--locked', '--offline'])
    recipes = [item for item in baseline['results'] if item['stage'] == 'charon-core']
    if len(recipes) != 1:
        raise ValueError('correct Charon recipe missing')
    environment = dict(os.environ, RUSTFLAGS='--remap-path-prefix=' + str(root) + '=.')
    run('mutant-charon', recipes[0]['command'], environment=environment)
    llbc = root / 'verification/aeneas/llbc/sandproof_storage.llbc'
    data = json.loads(llbc.read_text())
    if data.get('has_errors') is not False or llbc_digest(data) == profile['core_llbc_digest']:
        raise ValueError('mutant LLBC missing, erroneous or unchanged')
    record['mutant_llbc_digest'] = llbc_digest(data)
    generated = evidence / 'generated'
    run('mutant-aeneas', [args.tool_root / 'aeneas/bin/aeneas', '-backend', 'lean', '-abort-on-error',
        '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', 'SandproofStorageRust',
        '-subdir', 'SandproofStorageMutant', '-dest', generated, llbc])
    module = generated / 'SandproofStorageMutant'
    # Compare, never rewrite, the files generated by the original translator.
    for name in ('Types.lean', 'Funs.lean'):
        original_lean = (repository / 'verification/aeneas/storage/SandproofStorage' / name).read_text()
        mutant_lean = (module / name).read_text().replace('import SandproofStorageMutant.', 'import SandproofStorage.')
        if name == 'Funs.lean':
            old = '{ self with connection := none, write_fenced := true })'
            if original_lean.count(old) != 1:
                raise ValueError('original generated assignment is not unique')
            original_lean = original_lean.replace(old, old.replace(':= true', ':= false'))
        if mutant_lean != original_lean:
            raise ValueError('unreviewed additional generated mutant difference: ' + name)
    for name in ('TypesExternal', 'FunsExternal', 'Library'):
        text = (repository / f'verification/aeneas/storage/SandproofStorage/{name}.lean').read_text()
        (module / (name + '.lean')).write_text(text.replace('import SandproofStorage.', 'import SandproofStorageMutant.'))
    for template in module.glob('*_Template.lean'):
        template.rename(evidence / template.name)
    proofs = (repository / 'verification/aeneas/storage/SandproofStorage/Results.lean').read_text()
    if proofs.count('import SandproofStorage.Funs') != 1:
        raise ValueError('original proof import changed')
    same = evidence / 'SameProperties.lean'
    same.write_text(proofs.replace('import SandproofStorage.Funs', 'import SandproofStorageMutant.Funs'))
    counter = evidence / 'Counterexample.lean'
    counter.write_bytes((assets / 'Counterexample.lean').read_bytes())
    record['same_property_original_sha256'] = hashlib.sha256(proofs.encode()).hexdigest()
    record['same_property_mutant_sha256'] = sha(same)
    record['generated_mutant_sha256'] = {name: sha(module / name) for name in ('Types.lean', 'Funs.lean')}
    # An additive view in the repository root preserves every original package
    # setting, without editing main Lake or importing both Rust namespaces.
    handle = tempfile.NamedTemporaryFile(dir=repository, prefix='lake-storage-mutant-', suffix='.toml', delete=False)
    view = Path(handle.name)
    handle.close()
    try:
        view.write_text((repository / 'lake-storage.toml').read_text() + '\n[[lean_lib]]\n'
                        'name = "SandproofStorageMutant"\nsrcDir = ' + json.dumps(str(generated)) + '\n')
        run('mutant-funs-build', ['lake', '-f', view, '--no-cache', 'build', 'SandproofStorageMutant.Funs'], cwd=repository)
        successful = run('kernel-counterexample', ['lake', '-f', view, 'env', 'lean', '--json', counter], cwd=repository)
        positive_messages = messages(successful)
        if (any(item['severity'] in {'error', 'warning'} for item in positive_messages)
                or sum(str(item.get('data', '')).startswith('Checked SandproofStorageRust.FaultWitness.')
                       for item in positive_messages) != 3):
            raise ValueError('counterexample diagnostics or dependency audit incomplete')
        rejected = run('same-properties-rejected', ['lake', '-f', view, 'env', 'lean', '--json', same],
                       expected=1, cwd=repository)
        errors = [item for item in messages(rejected) if item['severity'] == 'error']
        starts = [(match.start(), match.group(1)) for match in re.finditer(r'^theorem (\w+)', proofs, re.M)]
        def owner(line):
            prior = [name for offset, name in starts if proofs.count('\n', 0, offset) + 1 <= line]
            return prior[-1] if prior else None
        failed = {owner(item['pos']['line']) for item in errors}
        expected = {'fence_unknown_exact', 'eval_error_returns_unknown'}
        if failed != expected:
            raise ValueError('not the expected same-property rejection: ' + repr(failed))
        record['same_property_failures'] = sorted(failed)
    finally:
        view.unlink()
    after = {name: sha(root / name) for name in profile['candidate_files']}
    if before != after:
        raise ValueError('mutant sources changed during checking')
    record.update(exit_code=0, source_unchanged=True, actual_rust_mutant=True,
                  same_properties_rejected=True, kernel_counterexample=True)
    (evidence / 'negative.json').write_text(json.dumps(record, indent=2) + '\n')
    print('Actual Rust mutant compiled/extracted; unchanged properties rejected it; kernel counterexample passed.', flush=True)


if __name__ == '__main__':
    main()
