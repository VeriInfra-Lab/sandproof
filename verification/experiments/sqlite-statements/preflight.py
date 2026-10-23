"""Original-tool preflight of the actual SQLite statement callers.

Formal machine only. A frontend/backend limitation is diagnostic evidence,
never a semantic counterexample. Every trial has a fresh evidence directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
TOOLS = {'aeneas': '505b6ca35217e7be5c96c3e2f8045edfbdf47291',
         'aeneas/charon': 'b104e24fea7d721b71e6c39fd70f26ff20bc0980'}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def production_inputs(root):
    return {str(p.relative_to(root)): sha(p)
            for p in sorted(root.rglob('*'))
            if p.is_file() and not p.is_symlink()
            and p.relative_to(root).parts[0] != 'artifacts'
            and not any(x in ('.git', '.tools', '.lake', 'target') for x in p.relative_to(root).parts)
            and (p.suffix == '.rs' or p.name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'))}


def preflight(output, source, tool_root):
    output.mkdir(parents=True, exist_ok=False)
    if 'CARGO_TARGET_DIR' not in os.environ:
        raise ValueError('Use the coordinated stable formal compiler target')
    for directory, expected in TOOLS.items():
        actual = subprocess.check_output(['git', '-C', str(tool_root/directory),
            'rev-parse', 'HEAD'], text=True).strip()
        if actual != expected:
            raise ValueError('Unreviewed original tool revision: '+directory)
    before = production_inputs(source)
    environment = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_NET_OFFLINE='true')
    charon = tool_root/'aeneas/charon/bin/charon'
    aeneas = tool_root/'aeneas/bin/aeneas'
    record = dict(kind='original-program-translation-preflight-not-semantic-counterexample',
                  sources=before, tools=TOOLS, target='x86_64-unknown-linux-gnu', trials=[])
    started = time.monotonic()
    for tag, root in [('conditional-write', 'crate::journal::write_accepted_with_ids'),
                      ('sandbox-read', 'crate::journal::_::sandbox')]:
        trial = output/tag
        trial.mkdir()
        llbc = trial/'original.llbc'
        command = [str(charon), 'cargo', '--preset=aeneas', '--error-on-warnings',
            '--sysroot', 'default', '--targets', 'x86_64-unknown-linux-gnu',
            '--opaque', 'rusqlite', '--opaque', 'serde_json',
            '--opaque', 'std::io', '--include', 'sandproof_controller_service',
            '--start-from', root, '--dest-file', str(llbc), '--',
            '--manifest-path', 'controller-service/Cargo.toml', '--lib', '--locked', '--offline']
        row = dict(tag=tag, root=root, charon_command=command)
        tick = time.monotonic()
        with (trial/'charon.log').open('xb') as log:
            try:
                row['charon_exit_code'] = subprocess.run(command, cwd=source,
                    env=environment, stdout=log, stderr=subprocess.STDOUT, timeout=300).returncode
            except subprocess.TimeoutExpired:
                row['charon_timed_out'] = True
        row['charon_seconds'] = round(time.monotonic()-tick, 3)
        if llbc.exists():
            translated = json.loads(llbc.read_text())
            row['has_errors'] = translated.get('has_errors')
            row['llbc_sha256'] = sha(llbc)
            local = [f for f in translated['translated']['fun_decls'] if f and f['item_meta']['is_local']]
            row['original_local_roots'] = [dict(name=f['item_meta']['name'],
                structured=isinstance(f['body'],dict) and 'Structured' in f['body']) for f in local]
            if row.get('charon_exit_code') == 0 and row['has_errors'] is False:
                destination = trial/'generated'
                destination.mkdir()
                command = [str(aeneas), '-backend', 'lean', '-abort-on-error',
                    '-warnings-as-errors', '-split-files', '-emit-json',
                    '-namespace', 'SqliteStatementsPreflightRust', '-subdir', 'SqliteStatementsPreflight',
                    '-dest', str(destination), str(llbc)]
                row['aeneas_command'] = command
                tick = time.monotonic()
                with (trial/'aeneas.log').open('xb') as log:
                    try:
                        row['aeneas_exit_code'] = subprocess.run(command,cwd=source,
                            env=environment,stdout=log,stderr=subprocess.STDOUT,timeout=300).returncode
                    except subprocess.TimeoutExpired:
                        row['aeneas_timed_out'] = True
                row['aeneas_seconds'] = round(time.monotonic()-tick,3)
        record['trials'].append(row)
        record['sources_unchanged'] = production_inputs(source) == before
        (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
        if not record['sources_unchanged']:
            raise ValueError('Original production source changed during preflight')
    record.update(complete=True, seconds=round(time.monotonic()-started,3))
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps({k:v for k,v in record.items() if k!='sources'},indent=2))


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--source',type=Path,default=ROOT)
    parser.add_argument('--tool-root',type=Path,required=True)
    args=parser.parse_args()
    preflight(args.output.resolve(),args.source.resolve(),args.tool_root.resolve())
