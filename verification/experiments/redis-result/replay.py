"""Rebuild the frozen isolated Redis candidate using the original tools.

This checks extraction/byte identity and mechanical facade admission. It does
not run Lean proofs, native component tests or the original complete gates.
Every destination is new; failures and generated evidence are preserved.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-base', type=Path, required=True)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--tool-root', type=Path, required=True)
    parser.add_argument("--native-only", action="store_true")
    args = parser.parse_args()
    assets = Path(__file__).resolve().parent
    repository = assets.parents[2]
    sys.path.insert(0, str(repository))
    from tools.check_first_chain_extraction import check_tools
    from tools.check_foundations_extraction import llbc_digest
    from tools.check_storage_binding import check as check_bindings, check_mutations
    from tools.check_storage_extraction import check as check_extraction, check_libraries
    # Reuse the existing exact binary/frontend/backend/source-version admission.
    if not args.native_only:
        check_tools(repository, args.tool_root)
        check_libraries(repository, args.tool_root)
    subprocess.run([sys.executable, str(assets / 'prepare.py'),
                    '--source-base', str(args.source_base), '--destination', str(args.destination)], check=True)
    root = args.destination / 'source'
    evidence = args.destination / 'evidence'
    evidence.mkdir(exist_ok=False)
    profile = json.loads((assets / 'candidate.json').read_text())
    before = {name: digest(root / name) for name in profile['candidate_files']}
    results = []
    receipt = {'scope': 'source-bound isolated candidate extraction, not complete proof acceptance',
               'base': profile['base_anchor'], 'candidate_inputs': before, 'results': results}

    def save():
        (evidence / 'replay.json').write_text(json.dumps(receipt, indent=2) + '\n')

    def run(stage, command, environment=None):
        if str(command[0]) == 'cargo':
            from tools.acceptance_receipts import native_run
            native_run('redis-replay-' + stage, command, root, evidence / (stage + '.log'), environment)
            return
        print('START', stage, flush=True)
        start = time.monotonic()
        with (evidence / (stage + '.log')).open('xb') as log:
            process = subprocess.Popen([str(x) for x in command], cwd=root, env=environment,
                                       stdout=log, stderr=subprocess.STDOUT)
            (evidence / (stage + '.pid')).write_text(str(process.pid) + '\n')
            code = process.wait()
        results.append({'stage': stage, 'command': [str(x) for x in command],
                        'exit_code': code, 'seconds': round(time.monotonic() - start, 3)})
        save()
        print('END', stage, code, results[-1]['seconds'], flush=True)
        if code:
            raise subprocess.CalledProcessError(code, command)

    manifest = 'verification/extract-storage/Cargo.toml'
    run('native-check', ['cargo', 'check', '--manifest-path', manifest, '--locked', '--offline'])
    run('root-format', ['cargo', 'fmt', '--manifest-path', manifest, '--check'])
    if args.native_only:
        return
    charon = args.tool_root / 'aeneas/charon/bin/charon'
    aeneas = args.tool_root / 'aeneas/bin/aeneas'
    environment = dict(os.environ, RUSTFLAGS='--remap-path-prefix=' + str(root) + '=.')
    for role, roots, filename in [
        ('core', ['finish', 'commit', 'reaffirm'], 'sandproof_storage.llbc'),
        ('public', ['bindings::commit', 'bindings::reaffirm'], 'sandproof_storage_bindings.llbc'),
    ]:
        output = root / 'verification/aeneas/llbc' / filename
        if output.exists():
            raise ValueError('expected a new Charon output, found an old file')
        command = [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default']
        for name in roots:
            command += ['--start-from', 'crate::' + name]
        for name in ('eval', 'info', 'settings', 'trim_cr', 'split_colon'):
            command += ['--opaque', 'sandproof_storage_extract::store::storage_io::' + name]
        for name in ('sandproof_center_store', 'sandproof_protocol', 'sandproof_center_core'):
            command += ['--include', name]
        command += ['--dest-file', '../aeneas/llbc/' + filename,
                    '--', '--manifest-path', manifest, '--lib', '--locked', '--offline']
        run('charon-' + role, command, environment)
        if not output.is_file():
            raise ValueError('Charon did not create expected output')
        llbc = json.loads(output.read_text())
        if llbc.get('has_errors') is not False:
            raise ValueError('Charon output reported errors')
        receipt[role + '_llbc_digest'] = llbc_digest(llbc)
        frozen = profile['public_binding_llbc_digest' if role == 'public' else 'core_llbc_digest']
        if receipt[role + '_llbc_digest'] != frozen:
            raise ValueError('complete ' + role + ' LLBC changed')
        if role == 'public':
            admitted = check_bindings(repository, output)
            check_mutations(admitted)
            receipt['public_binding_mutations_rejected'] = 7
        save()
    generated = evidence / 'generated'
    run('aeneas-core', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
        '-split-files', '-emit-json', '-namespace', 'SandproofStorageRust', '-subdir', 'SandproofStorage',
        '-dest', generated, root / 'verification/aeneas/llbc/sandproof_storage.llbc'])
    outputs = {'SandproofStorage/Types.lean', 'SandproofStorage/Funs.lean', 'translation.json'}
    actual = {}
    for name in sorted(outputs):
        observed = generated / name
        registered = repository / 'verification/aeneas/storage' / name
        if not observed.is_file() or observed.read_bytes() != registered.read_bytes():
            raise ValueError('generated byte identity failed: ' + name)
        actual[name] = digest(observed)
    inventory = json.loads((generated / 'translation.json').read_text())
    core_llbc = json.loads((root / 'verification/aeneas/llbc/sandproof_storage.llbc').read_text())
    count, drops = check_extraction(repository, core_llbc, inventory)
    receipt['source_admission'] = {'functions': count, 'drop_sites': drops}
    after = {name: digest(root / name) for name in profile['candidate_files']}
    if after != before:
        raise ValueError('candidate source changed while checking')
    receipt.update(generated_byte_identity=actual, source_unchanged=True, exit_code=0)
    save()
    print('Original tools reproduced all three core outputs and admitted four actual public call-throughs.', flush=True)
    print('Concrete codec/Lua, Lean/native tests and complete gate acceptance are separate obligations.', flush=True)


if __name__ == '__main__':
    main()
