"""Strict original-tool extraction of this lane's actually called UPDATE core.

Formal machine only. Both actual roots share one Charon compilation. Wrong
Rust is a separate source snapshot and must extract before formal refutation.
A frontend/backend failure is never counted as a semantic counterexample.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

from preflight import TOOLS, production_inputs

HERE=Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def extract(source, output, tool_root, variant):
    if output.exists() or output.is_symlink():
        raise ValueError('extraction output already exists; inspect the original task')
    if not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('use the coordinated stable formal compiler target')
    profile=json.loads((HERE/'candidate.json').read_text())
    for name,digest in profile['candidate_files'].items():
        expected=digest
        if variant=='accept-zero-changes' and name.endswith('/statement_core.rs'):
            text=(HERE/'statement-core.rs').read_text()
            if text.count('if changed != 1 {')!=1:
                raise ValueError('ambiguous frozen negative source')
            expected=hashlib.sha256(text.replace('if changed != 1 {','if changed > 1 {').encode()).hexdigest()
        if sha(source/name)!=expected:
            raise ValueError('candidate source is not the registered actual-call variant: '+name)
    for name,digest in profile['source_files'].items():
        if name not in profile['candidate_files'] and sha(source/name)!=digest:
            raise ValueError('original source/config changed: '+name)
    for name,digest in TOOLS.items():
        actual=subprocess.check_output(['git','-C',str(tool_root/name),'rev-parse','HEAD'],text=True).strip()
        if actual!=digest:
            raise ValueError('unreviewed original tool: '+name)
    charon=tool_root/'aeneas/charon/bin/charon'
    aeneas=tool_root/'aeneas/bin/aeneas'
    before=production_inputs(source)
    output.mkdir(parents=True,mode=0o700)
    started=time.monotonic()
    record=dict(kind='actual-call-sqlite-statement-original-tool-extraction',variant=variant,
        base=profile['base'],source=str(source),sources=before,roots=profile['roots'],
        target=profile['target'],tools=TOOLS,
        tool_binaries_sha256={'charon':sha(charon),'aeneas':sha(aeneas)})
    llbc=output/'original.llbc'
    command=[str(charon),'cargo','--preset=aeneas','--error-on-warnings',
        '--sysroot','default','--targets',profile['target'],
        '--opaque','rusqlite','--opaque','serde_json','--opaque','std::io',
        '--include','sandproof_controller_service','--include','sandproof_protocol',
        '--start-from',','.join(profile['roots']),'--dest-file',str(llbc),'--',
        '--manifest-path','controller-service/Cargo.toml','--lib','--locked','--offline']
    record['charon_command']=command
    environment=dict(os.environ,CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true')
    with (output/'charon.log').open('xb') as log:
        try:
            record['charon_exit_code']=subprocess.run(command,cwd=source,env=environment,
                stdout=log,stderr=subprocess.STDOUT,timeout=300).returncode
        except subprocess.TimeoutExpired:
            record['charon_timed_out']=True
    if llbc.exists():
        data=json.loads(llbc.read_text())
        record.update(has_errors=data['has_errors'],llbc_sha256=sha(llbc))
        record['local_functions']=[dict(name=f['item_meta']['name'],
            structured=isinstance(f['body'],dict) and 'Structured' in f['body'])
            for f in data['translated']['fun_decls'] if f and f['item_meta']['is_local']]
        if record.get('charon_exit_code')==0 and data['has_errors'] is False:
            generated=output/'generated';generated.mkdir()
            command=[str(aeneas),'-backend','lean','-abort-on-error','-warnings-as-errors',
                '-split-files','-emit-json','-namespace','SandproofSqliteStatementsRust',
                '-subdir','SandproofSqliteStatements','-dest',str(generated),str(llbc)]
            record['aeneas_command']=command
            with (output/'aeneas.log').open('xb') as log:
                try:
                    record['aeneas_exit_code']=subprocess.run(command,cwd=source,env=environment,
                        stdout=log,stderr=subprocess.STDOUT,timeout=300).returncode
                except subprocess.TimeoutExpired:
                    record['aeneas_timed_out']=True
            record['generated']={str(p.relative_to(generated)):sha(p)
                for p in generated.rglob('*') if p.is_file()}
    record['sources_unchanged']=production_inputs(source)==before
    record['success']=(record.get('charon_exit_code')==0 and record.get('has_errors') is False
        and record.get('aeneas_exit_code')==0 and record['sources_unchanged'])
    record['seconds']=time.monotonic()-started
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps({k:v for k,v in record.items() if k not in ('sources','local_functions')},indent=2))
    return record


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--tool-root',type=Path,required=True)
    parser.add_argument('--variant',choices=('correct','accept-zero-changes'),default='correct')
    args=parser.parse_args()
    result=extract(args.source.resolve(strict=True),args.output.absolute(),
                   args.tool_root.resolve(strict=True),args.variant)
    raise SystemExit(0 if result['success'] else 1)
