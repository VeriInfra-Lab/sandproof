"""Check the actual retained Lean files through the registered service client."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
FILES=('SandboxModel.lean','verification/ProofAudit.lean',
       'SandboxModel/Boundaries/JsonRequest/Audit.lean',
       'SandboxModel/Boundaries/Revocation/Audit.lean',
       'SandboxModel/Boundaries/TlsPin/Audit.lean',
       'verification/aeneas/storage/SandproofStorage/Audit.lean',
       'verification/aeneas/sqlite/SandproofSqlite/Audit.lean',
       'verification/aeneas/tunnel/SandproofTunnel/Audit.lean')


def main():
    out=Path(tempfile.mkdtemp(prefix='physical-service-',dir=ROOT/'artifacts'))
    requests=[dict(tool='lean_diagnostic_messages',arguments={'file_path':n},
        expected_sha256=hashlib.sha256((ROOT/n).read_bytes()).hexdigest()) for n in FILES]
    text=''.join(json.dumps(r)+'\n' for r in requests)+'{"tool":"exit"}\n'
    (out/'requests.jsonl').write_text(text)
    with (out/'service.log').open('w') as log:
        subprocess.run([str(ROOT/'.tools/lean-mcp/bin/python'),'tools/lean_session.py'],
            cwd=ROOT,input=text,text=True,stdout=log,stderr=subprocess.STDOUT,check=True)
    records=[]
    for line in (out/'service.log').read_text().splitlines():
        try: row=json.loads(line)
        except ValueError: continue
        if row.get('request',{}).get('tool')=='lean_diagnostic_messages': records.append(row)
    if len(records)!=len(requests): raise ValueError('missing physical Lean request')
    for request,row in zip(requests,records):
        if (row.get('request')!=request or row.get('stale') is not False
            or row['revision']['file_sha256']!=request['expected_sha256']):
            raise ValueError('changed or stale physical proof file')
        result=row['result']
        for _ in range(3):
            if 'success' in result: break
            result=result.get('structuredContent',result.get('result',{}))
        if (result.get('success') is not True or result.get('partial') is not False
            or result.get('timed_out') or result.get('failed_dependencies')
            or any(i.get('severity') in ('warning','error') for i in result.get('items',[]))):
            raise ValueError('physical proof diagnostics incomplete or rejected')
    print('Eight complete, non-stale physical-file Lean service diagnostics:',out)


if __name__=='__main__': main()
