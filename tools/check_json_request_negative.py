"""Reproduce a real bypassed lifecycle binding guard and refute GuardContract."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = 'sandbox-protocol/src/internal/lifecycle.rs'
MANIFEST = 'verification/extract-foundations/Cargo.toml'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def materialize(source, project):
    tracked = subprocess.check_output(['git', 'ls-files', '-z'], cwd=source)
    for item in tracked.split(b'\0'):
        if item:
            relative = Path(item.decode())
            target = project / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / relative, target)
    # Keep the exact source inventory for native evidence on both machines.
    # This is an index only, without a fabricated commit or build artifacts.
    subprocess.run(['git', 'init', '--quiet', str(project)], check=True)
    subprocess.run(['git', 'add', '--force', '--pathspec-from-file=-',
                    '--pathspec-file-nul'], cwd=project, input=tracked, check=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument("--native-only", action="store_true")
    args = parser.parse_args(argv)
    output = args.output.resolve()
    if output.exists() or output == ROOT or ROOT.is_relative_to(output):
        parser.error('output must be a fresh private directory, not a project ancestor')
    output.mkdir(parents=True)
    before = sha(ROOT / SOURCE)

    def run(label, command, cwd=ROOT, expected=0):
        if command[0] == 'cargo':
            try:
                from tools.acceptance_receipts import native_run
            except ModuleNotFoundError:
                from acceptance_receipts import native_run
            native_run('json-' + label, command, cwd, output / (label + '.log'), expected=expected)
            return
        with (output / (label + '.log')).open('w') as stream:
            result = subprocess.run(command, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT)
        (output / (label + '.exit')).write_text(str(result.returncode) + '\n')
        if result.returncode != expected:
            raise ValueError(f'{label}: expected exit {expected}, got {result.returncode}')

    # A committed source checkout, without build caches, .tools or private logs.
    project = output / 'mutant'
    materialize(ROOT, project)
    for relative in ('.lake/packages', '.tools/lean-mcp'):
        target = project / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.symlink_to((ROOT / relative).resolve(), target_is_directory=True)
    original = (project / SOURCE).read_text()
    guard = '        self.identity.validate_node(node)?;'
    if original.count(guard) != 1:
        raise ValueError('production binding guard changed; re-review the mutation')
    (project / SOURCE).write_text(original.replace(guard, '        let _ = node;'))
    run('original-inputs', ['shasum', '-a', '256', '-c',
                           'verification/json-request-inputs.sha256'])
    run('mutant-rust-build', ['cargo', 'test', '--manifest-path', MANIFEST,
                              '--locked', '--offline', '--no-run'], cwd=project)
    run('mutant-rust', ['cargo', 'test', '--manifest-path', MANIFEST,
                        '--locked', '--offline', 'json_request::tests::own_validation_order_and_shape_are_separate_from_authority',
                        '--', '--exact'], cwd=project, expected=101)
    log = (output / 'mutant-rust.log').read_text()
    if ('left: Err(BindingMismatch)' not in log
            or 'right: Err(UnsupportedVersion)' not in log
            or 'test result: FAILED. 0 passed; 1 failed;' not in log):
        raise ValueError('mutant did not fail the original binding/order assertion')
    if args.native_only:
        return 0
    run('mutant-extraction', ['bash', 'scripts/regenerate-foundations-lean.sh'], cwd=project)
    shutil.copyfile(project / 'verification/fixtures/json-request-guard-negative.lean.in',
                    project / 'Evidence.lean')
    run('mutant-imports', ['lake', 'build', 'SandproofFoundations.Funs',
                          'SandboxModel.Boundaries.JsonRequest.Model',
                          'SandboxModel.ProofAuditCommand'], cwd=project)
    request = {'tool': 'lean_diagnostic_messages',
               'arguments': {'file_path': 'Evidence.lean'},
               'expected_sha256': sha(project / 'Evidence.lean')}
    with (output / 'service.log').open('w') as stream:
        result = subprocess.run([str(project / '.tools/lean-mcp/bin/python'),
                                 'tools/lean_session.py'], cwd=project,
                                input=json.dumps(request) + '\n{"tool":"exit"}\n',
                                text=True, stdout=stream, stderr=subprocess.STDOUT, timeout=300)
    if result.returncode:
        raise ValueError('negative service transport failed')
    records = []
    for line in (output / 'service.log').read_text().splitlines():
        try:
            record = json.loads(line)
        except ValueError:
            continue
        if record.get('request', {}).get('tool') == request['tool']:
            records.append(record)
    if (len(records) != 1 or records[0]['stale']
            or records[0]['revision']['file_sha256'] != request['expected_sha256']):
        raise ValueError('missing, changed or stale negative proof receipt')
    diagnostic = records[0]['result']
    for key in ('structuredContent', 'result'):
        if key in diagnostic:
            diagnostic = diagnostic[key]
    if (diagnostic.get('success') is not True or diagnostic.get('partial') is not False
            or diagnostic.get('timed_out') or diagnostic.get('failed_dependencies')
            or any(x.get('severity') == 'error' for x in diagnostic.get('items', []))):
        raise ValueError('same-property counterexample was not verified by the real service')
    run('mutant-kernel-audit', ['lake', 'env', 'lean', 'Evidence.lean'], cwd=project)
    if sha(ROOT / SOURCE) != before:
        raise ValueError('original production source changed during the negative experiment')
    receipt = {'success': True, 'source_sha256': before,
               'mutant_source_sha256': sha(project / SOURCE),
               'specification_sha256': sha(project / 'SandboxModel/Boundaries/JsonRequest/Model.lean'),
               'same_property': 'SandboxModel.Boundaries.JsonRequest.GuardContract',
               'rust_mutant_exit': 101, 'service_stale': False,
               'service_partial': False, 'service_log_sha256': sha(output / 'service.log')}
    (output / 'result.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt))
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print('JSON request negative check failed: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
