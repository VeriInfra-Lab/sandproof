"""Actual formal component; full acceptance also requires the registered native case."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import subprocess
import time
from check import check_inputs, ROOT, EXPECTED_AUDITS

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output',type=Path,required=True)
out=parser.parse_args().output.resolve()
if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')!='formal':
    raise ValueError('Formal work runs on a6000 only')
frozen=check_inputs();out.mkdir(mode=0o700,parents=True,exist_ok=False)
start=time.monotonic()
environment=json.loads((ROOT/'verification/experiments/tokio-mutex/environment.json').read_text())
files=environment['model_files']
record=dict(kind='conditional mutex formal component; not whole async Rust or full suite',complete=False,files=files)
def run(command,name):
    with (out/name).open('xb') as log:
        result=subprocess.run(command,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,timeout=900)
    if result.returncode:raise ValueError('Formal command failed: '+name)
try:
    run(['lake','build','SandboxModel.Boundaries.TokioMutex.Audit'],'build.log')
    run(['lake','env','lean','SandboxModel/Boundaries/TokioMutex/Audit.lean'],'audit.log')
    observed=re.findall(r'^Checked ([\w.]+): \d+ standard logical dependencies$',(out/'audit.log').read_text(),re.M)
    if sorted(observed)!=sorted(EXPECTED_AUDITS):raise ValueError('Incomplete audit')
    code='import tools.proof_services as service\nservice.FILES='+repr(tuple(files))+'\nservice.main()\n'
    (ROOT/'artifacts').mkdir(mode=0o700,exist_ok=True)
    run(['python3','-c',code],'actual-files.log')
    text=(out/'actual-files.log').read_text()
    if '4 complete, non-stale physical-file Lean service diagnostics:' not in text:
        raise ValueError('Incomplete physical file checks')
    actual=re.findall(r'^Checked ([\w.]+): \d+ standard logical dependencies$',text,re.M)
    if sorted(actual)!=sorted(EXPECTED_AUDITS):raise ValueError('Actual file audit mismatch')
    if check_inputs()!=frozen:raise ValueError('Changed component during proof')
    # Keep original service messages; names alone are not success evidence.
    for message in text.splitlines():
        if re.fullmatch(r'Checked [\w.]+: \d+ standard logical dependencies',message):
            print(message,flush=True)
    record.update(complete=True,exit_code=0,audits=21,actual_files=4,inputs=frozen)
except BaseException as error:
    record.update(exit_code=1,error=repr(error))
record['seconds']=time.monotonic()-start
(out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps(record));raise SystemExit(record['exit_code'])
