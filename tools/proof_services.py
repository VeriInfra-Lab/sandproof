"""Check the actual retained Lean files through the registered service client."""
import hashlib
import json
import re
from pathlib import Path
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
FILES=('SandboxModel.lean','verification/ProofAudit.lean',
       'SandboxModel/Boundaries/JsonRequest/Audit.lean',
       'SandboxModel/Boundaries/Revocation/Audit.lean',
       'SandboxModel/Boundaries/OwnedMessage/Audit.lean',
       'SandboxModel/Boundaries/Notify/Audit.lean',
       'SandboxModel/Boundaries/JoinSet/Audit.lean',
       'SandboxModel/Boundaries/StdMutex/Audit.lean',
       'SandboxModel/Boundaries/Condvar/Core.lean',
       'SandboxModel/Boundaries/Condvar/Protocol.lean',
       'SandboxModel/Boundaries/Condvar/Examples.lean',
       'SandboxModel/Boundaries/Condvar/Audit.lean',
       'SandboxModel/Boundaries/TokioRwLock/Core.lean',
       'SandboxModel/Boundaries/TokioRwLock/Mapping.lean',
       'SandboxModel/Boundaries/TokioRwLock/Examples.lean',
       'SandboxModel/Boundaries/TokioRwLock/Audit.lean',
       'SandboxModel/Boundaries/TokioRwLock.lean',
       'SandboxModel/Boundaries/SchemaDomains/Audit.lean',
       'SandboxModel/Foundations/LoopCallbacks/Audit.lean',
       'SandboxModel/Foundations/CallbackState/Audit.lean',
       'SandboxModel/Boundaries/ChildObservation/Audit.lean',
       'SandboxModel/Boundaries/BoundedWriter/Audit.lean',
       'SandboxModel/Boundaries/SqliteStatements/Audit.lean',
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
    audited_messages=[]
    for request,row in zip(requests,records):
        if (row.get('request')!=request or row.get('stale') is not False
            or row['revision']['file_sha256']!=request['expected_sha256']):
            raise ValueError('changed or stale physical proof file')
        result=row['result']
        for _ in range(3):
            if 'success' in result: break
            result=result.get('structuredContent',result.get('result',{}))
        if (result.get('success') is not True or result.get('partial') is not False
            or result.get('timed_out') is not False or result.get('failed_dependencies')
            or result.get('still_elaborating_lines') not in (None,[])
            or any(i.get('severity') in ('warning','error') for i in result.get('items',[]))):
            raise ValueError('physical proof diagnostics incomplete or rejected')
        expected=set(re.findall(r'^#audit_controller_proof (\S+)$',
                                (ROOT/request['arguments']['file_path']).read_text(),re.M))
        checked=set()
        for item in result.get('items',[]):
            message=str(item.get('message',item.get('data','')))
            match=re.fullmatch(r'Checked ([\w.]+): \d+ standard logical dependencies',message)
            if match:
                checked.add(match[1])
                audited_messages.append(message)
        if expected and checked!=expected:
            raise ValueError('physical service did not report the actual complete dependency audit')
    # Forward only actual messages after every original request was admitted.
    # The batch must see these audits even when Lake reused compiled imports.
    for message in audited_messages:
        print(message,flush=True)
    print(f'{len(FILES)} complete, non-stale physical-file Lean service diagnostics:',out)


if __name__=='__main__': main()
