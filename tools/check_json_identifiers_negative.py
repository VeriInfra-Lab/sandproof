"""Reproduce the real identifier-guard bypass and refute the unchanged IdentifierSafe."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import time

ROOT = Path(__file__).resolve().parents[1]
SOURCE = 'sandbox-protocol/src/lib.rs'
MANIFEST = 'verification/extract-foundations/Cargo.toml'
PROPERTY = 'SandboxModel/Boundaries/JsonIdentifiers/Model.lean'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--native-only', action='store_true')
    args = parser.parse_args(argv)
    output = args.output.resolve()
    if output.exists() or output == ROOT or ROOT.is_relative_to(output):
        parser.error('output must be a fresh private directory, not a project ancestor')
    output.mkdir(parents=True)
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', '--format=tar', commit], cwd=ROOT)
    (output / 'source.tar').write_bytes(archive)
    tracked = subprocess.check_output(['git', 'ls-tree', '-rz', '--name-only', commit], cwd=ROOT)
    try:
        from tools.check_json_request_negative import index_project
    except ModuleNotFoundError:
        from check_json_request_negative import index_project
    projects = {}
    for label in ('correct', 'mutant'):
        project = output / label
        project.mkdir()
        with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as tree:
            tree.extractall(project, filter='data')
        index_project(project, tracked)
        for relative in ('.lake/packages', '.tools'):
            target = project / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            tool_root = Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT', ROOT / '.tools'))
            destination = tool_root if relative == '.tools' else ROOT / relative
            target.symlink_to(destination.resolve(), target_is_directory=True)
        projects[label] = project
    before = sha(ROOT / SOURCE)
    records = []

    def run(label, command, project, expected=0, data=None):
        # Never share correct/mutant compiler artifacts.
        environment = dict(os.environ)
        environment['CARGO_TARGET_DIR'] = str(output / ('target-' + project.name))
        if command[0] == 'cargo':
            try:
                from tools.acceptance_receipts import native_run
            except ModuleNotFoundError:
                from acceptance_receipts import native_run
            native_run('json-identifiers-' + label, command, project,
                       output / (label + '.log'), environment, expected)
            return
        started = time.monotonic()
        with (output / (label + '.log')).open('w') as stream:
            result = subprocess.run(command, cwd=project, env=environment, input=data,
                                    text=True, stdout=stream, stderr=subprocess.STDOUT)
        records.append({'label': label, 'exit': result.returncode,
                        'seconds': round(time.monotonic() - started, 3)})
        (output / (label + '.exit')).write_text(str(result.returncode) + '\n')
        if result.returncode != expected:
            raise ValueError(f'{label}: expected {expected}, got {result.returncode}')

    correct, mutant = projects['correct'], projects['mutant']
    if not args.native_only:
        run('correct-module', ['bash', 'scripts/verify-json-identifiers.sh'], correct)
    original = (mutant / SOURCE).read_text()
    needle = 'if valid_identifier(&value) {'
    if original.count(needle) != 1:
        raise ValueError('production macro guard changed; review this mutation')
    (mutant / SOURCE).write_text(original.replace(needle, 'if true {'))
    run('mutant-build', ['cargo', 'test', '--manifest-path', MANIFEST,
                        '--locked', '--offline', '--no-run'], mutant)
    run('mutant-rust', ['cargo', 'test', '--manifest-path', MANIFEST,
                       '--locked', '--offline', 'successful_decode_is_valid'], mutant, 101)
    log = (output / 'mutant-rust.log').read_text()
    for kind in ('tenant', 'sandbox', 'operation', 'logical_action', 'resource',
                 'client_request', 'deployment', 'node'):
        if f'json_ids::tests::{kind}::successful_decode_is_valid' not in log:
            raise ValueError('missing same-property Rust failure for ' + kind)
    if ('successful decode violated opaque identifier contract: ""' not in log
            or 'test result: FAILED. 0 passed; 8 failed;' not in log):
        raise ValueError('Rust mutant did not fail the intended safety assertion')
    if args.native_only:
        if sha(ROOT / SOURCE) != before:
            raise ValueError('original production source changed')
        return 0
    run('mutant-extraction', ['bash', 'scripts/regenerate-foundations-lean.sh'], mutant)
    run('mutant-imports', ['lake', 'build', 'SandproofFoundations.Funs',
                          'SandboxModel.Boundaries.JsonIdentifiers.Calls',
                          'SandboxModel.Boundaries.JsonIdentifiers.Model',
                          'SandboxModel.Boundaries.JsonIdentifiers.ValueReader',
                          'SandboxModel.ProofAuditCommand'], mutant)
    for relative in (PROPERTY, 'SandboxModel/Foundations/IdentifierModel.lean',
                     'SandboxModel/Boundaries/JsonIdentifiers/Calls.lean',
                     'SandboxModel/Boundaries/JsonIdentifiers/ValueReader.lean'):
        if sha(correct / relative) != sha(mutant / relative):
            raise ValueError('specification or call-through was changed for the mutant')
    evidence = mutant / 'Evidence.lean'
    evidence.write_bytes((mutant / 'verification/fixtures/json-identifiers-negative.lean.in').read_bytes())
    request = {'tool': 'lean_diagnostic_messages',
               'arguments': {'file_path': 'Evidence.lean'},
               'expected_sha256': sha(evidence)}
    run('negative-service', [str(mutant / '.tools/lean-mcp/bin/python'),
                            'tools/lean_session.py'], mutant,
        data=json.dumps(request) + '\n{"tool":"exit"}\n')
    receipts = []
    for line in (output / 'negative-service.log').read_text().splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if record.get('request', {}).get('tool') == request['tool']:
            receipts.append(record)
    if (len(receipts) != 1 or receipts[0]['stale']
            or receipts[0]['revision']['file_sha256'] != request['expected_sha256']):
        raise ValueError('missing, changed or stale counterexample receipt')
    diagnostic = receipts[0]['result']
    for key in ('structuredContent', 'result'):
        if key in diagnostic:
            diagnostic = diagnostic[key]
    if (diagnostic.get('success') is not True or diagnostic.get('partial') is not False
            or diagnostic.get('timed_out') or diagnostic.get('failed_dependencies')
            or any(x.get('severity') == 'error' for x in diagnostic.get('items', []))):
        raise ValueError('same-property counterexample was not checked by the real Lean service')
    run('negative-kernel-audit', ['lake', 'env', 'lean', 'Evidence.lean'], mutant)
    if sha(ROOT / SOURCE) != before:
        raise ValueError('original production source changed')
    receipt = {
        'success': True, 'commit': commit, 'source_archive_sha256': sha(output / 'source.tar'),
        'source_sha256': before, 'mutant_source_sha256': sha(mutant / SOURCE),
        'property_sha256': sha(correct / PROPERTY),
        'same_property': 'SandboxModel.Boundaries.JsonIdentifiers.IdentifierSafe',
        'rust_mutant_exit': 101, 'service_stale': False, 'service_partial': False,
        'commands': records,
    }
    (output / 'result.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt))
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print('JSON identifier negative check failed: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
