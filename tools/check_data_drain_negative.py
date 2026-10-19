"""Reproduce the real missing-unknown mutation and refute the same read policy."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SOURCE = 'controller-service/src/application/data_drain.rs'
TEST = 'tests::all_pins_must_drop_and_unknown_exit_never_becomes_drained'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def service(project, evidence):
    request = {'tool': 'lean_diagnostic_messages', 'arguments': {'file_path': 'Evidence.lean'},
               'expected_sha256': sha(project / 'Evidence.lean')}
    with (evidence / 'service.log').open('w') as stream:
        result = subprocess.run([str(project / '.tools/lean-mcp/bin/python'), 'tools/lean_session.py'],
                                cwd=project, input=json.dumps(request) + '\n{"tool":"exit"}\n',
                                text=True, stdout=stream, stderr=subprocess.STDOUT, timeout=300)
    if result.returncode:
        raise ValueError('negative evidence service transport failed')
    records = []
    for line in (evidence / 'service.log').read_text().splitlines():
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if value.get('request', {}).get('tool') == request['tool']:
            records.append(value)
    if len(records) != 1 or records[0]['stale']:
        raise ValueError('missing or stale negative evidence service receipt')
    record = records[0]
    if record['revision']['file_sha256'] != request['expected_sha256']:
        raise ValueError('negative evidence source changed')
    output = record['result']
    for key in ('structuredContent', 'result'):
        if key in output:
            output = output[key]
    if (output.get('success') is not True or output.get('partial') is not False
            or output.get('timed_out') or output.get('failed_dependencies')
            or any(item.get('severity') == 'error' for item in output.get('items', []))):
        raise ValueError('negative counterexample not verified by the real Lean service')
    return {'file_sha256': request['expected_sha256'], 'partial': False, 'stale': False,
            'log_sha256': sha(evidence / 'service.log')}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(argv)
    output = args.output.resolve()
    if output.exists() or output == ROOT or ROOT.is_relative_to(output):
        parser.error('output must be a fresh private directory, not an ancestor of the project')
    output.mkdir(parents=True)
    before = sha(ROOT / SOURCE)

    def run(label, command, cwd=ROOT, expected=0):
        with (output / (label + '.log')).open('w') as stream:
            result = subprocess.run(command, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT)
        (output / (label + '.exit')).write_text(str(result.returncode) + '\n')
        if result.returncode != expected:
            raise ValueError(f'{label}: expected exit {expected}, got {result.returncode}')

    project = output / 'mutant'
    for relative in [SOURCE, 'verification/extract-data-drain/src/lib.rs',
                     'scripts/regenerate-data-drain-lean.sh', 'scripts/lean-mcp-server.sh',
                     'tools/lean_session.py', 'SandboxModel/ProofAuditCommand.lean',
                     'SandboxModel/Boundaries/DataDrain/Model.lean',
                     'SandboxModel/Boundaries/DataDrain/Representation.lean',
                     'verification/aeneas/data-drain/SandproofDataDrain/TypesExternal.lean',
                     'lakefile.toml', 'lake-manifest.json', 'lean-toolchain']:
        target = project / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / relative, target)
    (project / '.lake').mkdir(exist_ok=True)
    (project / '.lake/packages').symlink_to((ROOT / '.lake/packages').resolve(), target_is_directory=True)
    (project / '.tools').mkdir()
    (project / '.tools/lean-mcp').symlink_to((ROOT / '.tools/lean-mcp').resolve(), target_is_directory=True)
    original = (project / SOURCE).read_text()
    expression = 'self.routes_dropped() && !self.unknown.load(Ordering::Acquire)'
    if original.count(expression) != 1:
        raise ValueError('production expression changed; re-review mutation')
    (project / SOURCE).write_text(original.replace(expression, 'self.routes_dropped()'))

    run('original-build', ['rustc', '--edition', '2024', '--test', SOURCE,
                           '-o', str(output / 'original-tests')])
    run('original-test', [str(output / 'original-tests'), TEST, '--exact', '--nocapture'])
    run('mutant-build', ['rustc', '--edition', '2024', '--test', SOURCE,
                         '-o', str(output / 'mutant-tests')], cwd=project)
    run('mutant-test', [str(output / 'mutant-tests'), TEST, '--exact', '--nocapture'], expected=101)
    if ('test result: ok. 1 passed;' not in (output / 'original-test.log').read_text()
            or 'assertion failed: !pin.drained()' not in (output / 'mutant-test.log').read_text()
            or 'test result: FAILED. 0 passed; 1 failed;' not in (output / 'mutant-test.log').read_text()):
        raise ValueError('mutation failed for a reason other than the original unknown assertion')
    run('mutant-extraction', ['bash', 'scripts/regenerate-data-drain-lean.sh'], cwd=project)
    # This mutation removes deref/load completely. Supply only the reachable
    # count provider; do not invent an Ordering type or change generated code.
    (project / 'verification/aeneas/data-drain/SandproofDataDrain/FunsExternal.lean').write_text(
        'import SandproofDataDrain.Types\nopen Aeneas Aeneas.Std\n'
        'def alloc.sync.Arc.strong_count {T : Type} (_A : Type)\n'
        '    (view : alloc.sync.Arc T) : Result Usize := .ok view.observedStrongCount\n')
    inventory = json.loads((project / 'verification/aeneas/data-drain/translation.json').read_text())
    if {f['lean_name'] for f in inventory['functions'] if f['is_opaque']} != {'alloc.sync.Arc.strong_count'}:
        raise ValueError('mutation has an unexpected external surface')
    evidence = '''import SandboxModel.Boundaries.DataDrain.Representation
import SandproofDataDrain.Funs
import SandboxModel.ProofAuditCommand
namespace DataDrainNegative
open Aeneas Aeneas.Std SandboxModel.Boundaries.DataDrain
def witness : SandproofDataDrainRust.data_drain.DataLease := ⟨⟨0, 1#usize, true⟩⟩
theorem mutant_returns_true : SandproofDataDrainRust.read_drained witness = .ok true := by
  simp [SandproofDataDrainRust.read_drained, SandproofDataDrainRust.data_drain.DataLease.drained,
    SandproofDataDrainRust.data_drain.DataLease.routes_dropped, alloc.sync.Arc.strong_count, witness]
theorem mutant_not_safe : ¬ RustReadSafe SandproofDataDrainRust.read_drained := by
  intro safe
  have contradiction := safe witness true mutant_returns_true
  simp [ReadSafe, view, witness] at contradiction
#audit_controller_proof DataDrainNegative.mutant_returns_true
#audit_controller_proof DataDrainNegative.mutant_not_safe
end DataDrainNegative
'''
    (project / 'Evidence.lean').write_text(evidence)
    run('mutant-imports', ['lake', 'build', 'SandproofDataDrain.Funs',
                         'SandboxModel.Boundaries.DataDrain.Representation',
                         'SandboxModel.ProofAuditCommand'], cwd=project)
    checked_service = service(project, output)
    run('mutant-kernel-audit', ['lake', 'env', 'lean', 'Evidence.lean'], cwd=project)
    if sha(ROOT / SOURCE) != before:
        raise ValueError('original production source changed during negative check')
    receipt = {'success': True, 'property': 'SandboxModel.Boundaries.DataDrain.RustReadSafe',
               'original_test': {'passed': 1}, 'mutant_test': {'failed': 1, 'exit': 101},
               'mutant_compiled_and_translated': True, 'formal_counterexample_checked': True,
               'production_source_sha256': before, 'production_unchanged': True,
               'mutant_source_sha256': sha(project / SOURCE), 'service': checked_service}
    (output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(receipt, indent=2))
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print('DataLease negative check failed: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
