"""Check the local proof-warning cleanup; never issue public acceptance."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

HERE = Path(__file__).resolve().parent
FOREIGN = {
    'SandproofOriginalSourceRows/FunsExternal.lean',
    'SandproofUnloadStartupJoint/FunsExternal.lean',
    'SandproofUnloadOptionsRead/FunsExternal.lean',
    'SandproofUnloadContinuedReady/FunsExternal.lean',
}
SUPPORT = 'SandproofUnloadBaseSnapshot.firstEntryOrdered'
CLEANED = {
    'InboxParentCarriers.lean', 'NativeSnapshotFields.lean',
    'SandproofUnloadBaseSnapshot/Provider.lean', 'ActualStartupParentCompose.lean',
    'StartupParentBound.lean', 'StartupReadAgreement.lean', 'WorldParentConclusion.lean',
    'BaseSecondInbox.lean', 'RefoundWorldParent.lean', 'ContinuedDriverCaller.lean',
}
CODE = '''import UnloadParentLink
import Lean
open Lean Elab Command

def canonicalContractType : Expr → Expr
  | .app f a => .app (canonicalContractType f) (canonicalContractType a)
  | .lam _ t b k => .lam .anonymous (canonicalContractType t) (canonicalContractType b) k
  | .forallE _ t b k => .forallE .anonymous (canonicalContractType t) (canonicalContractType b) k
  | .letE _ t v b n => .letE .anonymous (canonicalContractType t)
      (canonicalContractType v) (canonicalContractType b) n
  | .mdata _ b => canonicalContractType b
  | .proj n i b => .proj n i (canonicalContractType b)
  | other => other

run_cmd do
  let env ← getEnv
  let names : Array Name := CONTRACT_NAMES
  let mut rows : Array Json := #[]
  for declName in names do
    let some declInfo := env.find? declName | throwError "Missing registered declaration {declName}"
    rows := rows.push <| Json.mkObj [
      ("name", toJson declName.toString), ("theorem", toJson declInfo.isTheorem),
      ("type", toJson (reprStr (canonicalContractType declInfo.type)))]
  logInfo (Json.mkObj [("contract_signatures", Json.arr rows)]).compress
'''


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources(project):
    plan = json.loads((project / 'plan.json').read_bytes())
    if len(plan['modules']) != 279 or plan['roots'] != ['UnloadParentLink']:
        raise ValueError('unexpected proof closure')
    for name in plan['modules']:
        if not (project / name).resolve(strict=True).is_relative_to(project):
            raise ValueError('proof source escapes project')
    actual = {name: sha(project / name) for name in plan['modules']}
    if actual != {name: entry['sha256'] for name, entry in plan['modules'].items()}:
        raise ValueError('actual source differs from the exact materialization plan')
    return actual


def declarations():
    names = json.loads((HERE / 'registration-candidate.json').read_bytes())['audited_declarations']
    if (len(names) != 86 or len(set(names)) != 86
            or any(not re.fullmatch(r'[A-Za-z_][A-Za-z_0-9]*(?:\.[A-Za-z_][A-Za-z_0-9]*)*', n) for n in names)):
        raise ValueError('changed audited declaration inventory')
    return names


def signature_record(path):
    records = []
    for line in path.read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get('request', {}).get('tool') == 'lean_run_code':
            records.append(row)
    if len(records) != 1:
        raise ValueError('missing exact signature response')
    row, = records
    result = row['result']
    if (row.get('stale') is not False
            or row['revision']['file_sha256'] != row['request']['expected_sha256']
            or result.get('success') is not True or result.get('timed_out') is not False):
        raise ValueError('signature probe did not complete on the exact source')
    entries = []
    for item in result.get('diagnostics', []):
        if item.get('severity') == 'error':
            raise ValueError('signature probe error')
        message = str(item.get('message', ''))
        if message.startswith('{"contract_signatures":'):
            entries.extend(json.loads(message)['contract_signatures'])
    return row, entries


def snapshot(project, output, lock):
    before = sources(project)
    names = declarations() + [SUPPORT]
    code = CODE.replace('CONTRACT_NAMES', '#[' + ', '.join('`' + n for n in names) + ']')
    output.mkdir(mode=0o700)
    tag = hashlib.sha256(str(output).encode()).hexdigest()[:16]
    snippet = project / 'artifacts' / ('warning-contracts-' + tag + '.lean')
    with snippet.open('x') as stream:
        stream.write(code)
    request = dict(tool='lean_run_code', code_file=str(snippet.relative_to(project)),
                   expected_sha256=sha(snippet))
    payload = json.dumps(request) + '\n{"tool":"exit"}\n'
    (output / 'requests.jsonl').write_text(payload)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    with lock.open('r+') as guard:
        fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with (output / 'service.log').open('x') as log:
            subprocess.run([str(project / '.tools/lean-mcp/bin/python'), 'tools/lean_session.py'],
                                 cwd=project, input=payload, text=True, stdout=log,
                                 stderr=subprocess.STDOUT, check=True,
                                 env=dict(os.environ, LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536'))
    row, entries = signature_record(output / 'service.log')
    if row['request'] != request:
        raise ValueError('missing exact signature response')
    if row['revision']['file_sha256'] != sha(snippet):
        raise ValueError('signature probe did not complete on the exact source')
    if len(entries) != len(names) or {x['name'] for x in entries} != set(names):
        raise ValueError('incomplete or duplicated declaration signatures')
    if before != sources(project):
        raise ValueError('proof source changed during the signature probe')
    report = dict(development_only=True, public_acceptance=False, complete=True,
                  entries=sorted(entries, key=lambda x: x['name']), source_sha256=before,
                  service_log_sha256=sha(output / 'service.log'), request=request,
                  revision=row['revision'])
    (output / 'signatures.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(dict(complete=True, signatures=len(entries), public_acceptance=False)))


def read_snapshot(path):
    data = json.loads(path.read_bytes())
    if (not isinstance(data, dict) or data.get('complete') is not True
            or data['service_log_sha256'] != sha(path.parent / 'service.log')):
        raise ValueError('signature evidence is incomplete or damaged')
    row, entries = signature_record(path.parent / 'service.log')
    if (sorted(entries, key=lambda x: x['name']) != data['entries']
            or data['request'] != row['request'] or data['revision'] != row['revision']):
        raise ValueError('signature report disagrees with the original service response')
    return data


def compare(project, diagnostics, before, after, output):
    diagnostic = json.loads((diagnostics / 'result.json').read_bytes())
    warnings = diagnostic['warnings']
    prefix = "The declaration 'ArrayT1.Insts.RusqliteParamsParams.__bind_in' contains '__'"
    if (len(warnings) != 4 or {x['file'] for x in warnings} != FOREIGN
            or any(not x['diagnostic']['message'].startswith(prefix) for x in warnings)):
        raise ValueError('unresolved or unexpected warnings; four original foreign names required')
    actual = sources(project)
    if (diagnostic.get('complete') is not True or diagnostic['errors'] or diagnostic['rejected']
            or diagnostic['own_warnings'] or diagnostic['source_sha256'] != actual
            or diagnostic['service_log_sha256'] != sha(diagnostics / 'service.log')):
        raise ValueError('incomplete or changed physical-file diagnostics')
    old_snapshot, new_snapshot = read_snapshot(before), read_snapshot(after)
    if new_snapshot['source_sha256'] != actual or old_snapshot['source_sha256'].keys() != actual.keys():
        raise ValueError('signature evidence does not match the physical source closure')
    changed = {name for name, value in actual.items() if old_snapshot['source_sha256'][name] != value}
    if changed != CLEANED:
        raise ValueError('source delta differs from the ten handwritten proof adapters')
    left, right = old_snapshot['entries'], new_snapshot['entries']
    old, new = {x['name']: x for x in left}, {x['name']: x for x in right}
    if len(old) != len(left) or len(new) != len(right) or not set(declarations()) <= old.keys():
        raise ValueError('missing or duplicated registered contracts')
    if not old.keys() <= new.keys() or any(new[name] != value for name, value in old.items()):
        raise ValueError('a declaration type changed beyond bound names/metadata')
    output.mkdir(mode=0o700)
    report = dict(development_only=True, public_acceptance=False, public_zero_warning=False,
                  complete=True, physical_files=len(actual), remaining_foreign_warnings=4,
                  compared_declarations=len(old), registered_declarations=len(declarations()),
                  before_sha256=sha(before), after_sha256=sha(after),
                  diagnostics_sha256=sha(diagnostics / 'result.json'))
    (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('snapshot', 'compare'))
    parser.add_argument('--project', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path)
    parser.add_argument('--diagnostics', type=Path)
    parser.add_argument('--before', type=Path)
    parser.add_argument('--after', type=Path)
    args = parser.parse_args()
    if args.mode == 'snapshot':
        if args.lock is None:
            parser.error('snapshot requires the existing shared proof lock')
        snapshot(args.project.resolve(strict=True), args.output.resolve(), args.lock.resolve(strict=True))
    else:
        if any(x is None for x in (args.diagnostics, args.before, args.after)):
            parser.error('compare requires diagnostics and both signature records')
        compare(args.project.resolve(strict=True), args.diagnostics.resolve(strict=True),
                args.before.resolve(strict=True), args.after.resolve(strict=True), args.output.resolve())
