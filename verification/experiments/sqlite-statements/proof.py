"""Original Lean service checks in independent, small positive/negative views.

Formal machine only. Fixed package/tool trees are read-only links; all writable
builds/logs stay below the caller's evidence directory. No Rust tests run here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_sqlite_statements import (AUDIT, ASSETS, LIBRARY, MODELS,
    NEGATIVE_PROOF, NEGATIVE_VIEW_PROOF, ENVIRONMENT, check_pair, check_service,
    check_record, expected_audits, read_json, sha)


BASE_FILES = ['SandboxModel/ProofAuditCommand.lean', 'tools/lean_session.py',
    'scripts/lean-mcp-server.sh', 'lean-toolchain', 'lake-manifest.json',
    'verification/aeneas/sqlite/SandproofSqlite/TypesExternal.lean',
    'verification/aeneas/foundations/SandproofFoundations/TypesExternal.lean',
    'verification/aeneas/foundations/SandproofFoundations/IdentifierLibrary.lean']


def copy(root, view, name):
    target = view / name
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(root / name, target)


def prepare_view(root, view, negative):
    view.mkdir(mode=0o700)
    for name in BASE_FILES:
        copy(root, view, name)
    names = ['Bindings.lean', 'Contract.lean', 'Integer.lean'] if negative else [p.name for p in sorted((root / MODELS).glob('*.lean'))]
    for name in names:
        copy(root, view, MODELS / name)
    if not negative:
        copy(root, view, 'SandboxModel/Boundaries/SqliteStatements.lean')
    for name in ['Types.lean', 'Funs.lean', 'TypesExternal.lean', 'FunsExternal.lean']:
        relative = LIBRARY / 'SandproofSqliteStatements' / name
        copy(root, view, relative)
    if negative:
        shutil.copy2(root / ASSETS / 'negative/Funs.lean', view / LIBRARY / 'SandproofSqliteStatements/Funs.lean')
        copy(root, view, NEGATIVE_PROOF)
        shutil.copy2(root / NEGATIVE_PROOF, view / NEGATIVE_VIEW_PROOF)
    shutil.copy2(root / 'lake-sqlite-statements.toml', view / 'lakefile.toml')
    if negative:
        lakefile = view / 'lakefile.toml'
        lakefile.write_text(lakefile.read_text().replace(
            'defaultTargets = ["SandboxModel.Boundaries.SqliteStatements.Audit"]',
            'defaultTargets = ["SandboxModel.SqliteStatementsNegative"]'))
    (view / '.lake').mkdir()
    (view / '.lake/packages').symlink_to((root / '.lake/packages').resolve(strict=True), target_is_directory=True)
    (view / '.tools').symlink_to((root / '.tools').resolve(strict=True), target_is_directory=True)
    (view / 'artifacts').mkdir()
    return {str(p.relative_to(view)): sha(p) for p in view.rglob('*') if p.is_file() and not p.is_symlink()}


def command(args, view, log, environment, timeout):
    started = time.monotonic()
    with log.open('xb') as stream:
        code = subprocess.run(args, cwd=view, env=environment, stdout=stream,
                              stderr=subprocess.STDOUT, timeout=timeout).returncode
    record = dict(command=args, exit_code=code, seconds=time.monotonic()-started,
                  log_sha256=sha(log))
    if code:
        raise ValueError('original Lean command failed; preserve ' + str(log))
    return record


def run(root, output):
    if output.exists() or output.is_symlink():
        raise ValueError('evidence output exists; inspect the original task')
    check_pair(root, read_json(root / 'verification/aeneas/llbc/sandproof_sqlite_statements.llbc'),
               read_json(root / 'verification/aeneas/llbc/sandproof_sqlite_statements_wrong_count.llbc'))
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='sqlite-statements-formal-module-not-full-acceptance', views={})
    environment = dict(os.environ, LEAN_NUM_THREADS='1')
    for tag, negative in [('correct', False), ('accept-zero-changes', True)]:
        view = output / tag
        inputs = prepare_view(root, view, negative)
        target = 'SandboxModel.SqliteStatementsNegative' if negative else 'SandboxModel.Boundaries.SqliteStatements.Audit'
        build = command(['lake', 'build', target], view, view / 'artifacts/build.log', environment, 300)
        if negative:
            files = [NEGATIVE_VIEW_PROOF]
        else:
            files = sorted(MODELS / name for name in ['Model.lean', 'Integer.lean', 'Bindings.lean',
                'Contract.lean', 'Refinement.lean', 'Bridge.lean', 'Audit.lean'])
            files += [Path('SandboxModel/Boundaries/SqliteStatements.lean')]
            files += [LIBRARY / 'SandproofSqliteStatements' / name for name in ['TypesExternal.lean', 'FunsExternal.lean']]
        requests = [dict(tool='lean_diagnostic_messages', arguments={'file_path': str(name)},
                         expected_sha256=sha(view / name)) for name in files]
        requests.append(dict(tool='exit'))
        data = ''.join(json.dumps(x)+'\n' for x in requests)
        (view / 'artifacts/requests.jsonl').write_text(data)
        client_started = time.monotonic()
        with (view / 'artifacts/service.log').open('xb') as log:
            child = subprocess.Popen([str(view / '.tools/lean-mcp/bin/python'), 'tools/lean_session.py'],
                cwd=view, env=environment, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, text=True)
            child.communicate(data, timeout=360)
        if child.returncode:
            raise ValueError('original Lean client failed: ' + str(view))
        count = check_service(root, view, view / 'artifacts/service.log', negative)
        # The unchanged original client preserves full MCP envelopes in its
        # JSONL and prints their structured content to stdout. Admit the
        # original structured stdout records; keep both raw forms as evidence.
        rows = [json.loads(s) for s in (view / 'artifacts/service.log').read_text().splitlines() if s.startswith('{')]
        rows = [row for row in rows if row.get('request', {}).get('tool') == 'lean_diagnostic_messages']
        if len(rows) != len(files):
            raise ValueError('missing/extra actual physical diagnostics')
        versions = read_json(root / ENVIRONMENT)['service_versions']
        for name, row in zip(files, rows):
            expected = expected_audits(root, negative) if name in (AUDIT, NEGATIVE_VIEW_PROOF) else set()
            check_record(view, row, expected, name, versions)
        if any(sha(view / n) != digest for n, digest in inputs.items()):
            raise ValueError('formal view sources/config changed during checks')
        # Emit actual, already admitted service messages so the existing full
        # batch's cross-module audit collector can see these roots too.
        for row in rows:
            for item in row['result']['result']['items']:
                if item.get('message', '').startswith('Checked '):
                    print(item['message'], flush=True)
        record['views'][tag] = dict(inputs=inputs, build=build, actual_diagnostics=len(files),
            actual_audits=count, client_seconds=time.monotonic()-client_started)
        (output / 'progress.json').write_text(json.dumps(record,indent=2)+'\n')
    if sha(output / 'correct' / MODELS / 'Contract.lean') != sha(output / 'accept-zero-changes' / MODELS / 'Contract.lean'):
        raise ValueError('positive and negative did not use the same independent contract')
    record.update(success=True,seconds=time.monotonic()-started)
    (output / 'result.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(run(args.root.resolve(strict=True), args.output.absolute()), indent=2))
