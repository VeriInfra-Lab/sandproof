"""Original Lean service in isolated core, actual-caller and wrong-code views.
Only the formal node runs this. Source/model identity is checked across views.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from tools.check_child_observation import (ASSETS, MODELS, LIBRARY, VERSIONS,
    check_sources, expected_audits, check_record, sha)

BASE=['SandboxModel/ProofAuditCommand.lean','tools/lean_session.py',
      'scripts/lean-mcp-server.sh','lean-toolchain','lake-manifest.json']
COMMON=['Events.lean','Interface.lean','Model.lean','Library.lean','Fixtures.lean']
POSITIVE=['Refinement.lean','Guarantees.lean','Examples.lean','Audit.lean']
VIEW_LIBRARY=LIBRARY/'SandproofChildObservation'
NEGATIVE=Path('SandboxModel/ChildObservationNegative.lean')

def copy(root,view,name,target=None):
    p=view/(target or name);p.parent.mkdir(parents=True,exist_ok=True)
    shutil.copy2(root/name,p)

def prepare_view(root,view,variant):
    view.mkdir(mode=0o700)
    for name in BASE:copy(root,view,Path(name))
    files=[MODELS/n for n in COMMON]
    if variant!='kill-success-release':
        files += [MODELS/n for n in POSITIVE]
        files.append(Path('SandboxModel/Boundaries/ChildObservation.lean'))
        if variant=='caller':files += [MODELS/n for n in ['Concrete.lean','ConcreteAudit.lean']]
    for name in files:copy(root,view,name)
    if variant=='kill-success-release':
        for name in ['Types.lean','Funs.lean','FunsExternal.lean']:
            copy(root,view,ASSETS/'negative'/name,VIEW_LIBRARY/name)
        copy(root,view,ASSETS/'negative/Counterexample.lean',NEGATIVE)
        files.append(NEGATIVE);target='SandboxModel.ChildObservationNegative'
    else:
        original=LIBRARY/'concrete/SandproofChildObservation' if variant=='caller' else VIEW_LIBRARY
        for p in sorted((root/original).glob('*.lean')):copy(root,view,p.relative_to(root),VIEW_LIBRARY/p.name)
        target='SandboxModel.Boundaries.ChildObservation.'+('ConcreteAudit' if variant=='caller' else 'Audit')
    files += [p.relative_to(view) for p in sorted((view/VIEW_LIBRARY).glob('*.lean'))]
    shutil.copy2(root/'lake-child-observation.toml',view/'lakefile.toml')
    p=view/'lakefile.toml';p.write_text(p.read_text().replace('SandboxModel.Boundaries.ChildObservation.Audit',target))
    (view/'.lake').mkdir();(view/'.lake/packages').symlink_to((root/'.lake/packages').resolve(strict=True),target_is_directory=True)
    (view/'.tools').symlink_to((root/'.tools').resolve(strict=True),target_is_directory=True)
    (view/'artifacts').mkdir()
    inputs={str(p.relative_to(view)):sha(p) for p in view.rglob('*') if p.is_file() and not p.is_symlink()}
    return target,files,inputs

def run(root,output):
    frozen=check_sources(root)
    if output.exists() or output.is_symlink():raise ValueError('fresh evidence directory required')
    output.mkdir(parents=True,mode=0o700);start=time.monotonic();record={'kind':'child-observation-component-not-full-V0','views':{}}
    for variant in ['correct','caller','kill-success-release']:
        view=output/variant;target,files,inputs=prepare_view(root,view,variant)
        with (view/'artifacts/build.log').open('xb') as log:
            code=subprocess.run(['lake','build',target],cwd=view,env=dict(os.environ,LEAN_NUM_THREADS='1'),stdout=log,stderr=subprocess.STDOUT,timeout=360).returncode
        if code:raise ValueError('original Lean build failed: '+str(view))
        requests=[dict(tool='lean_diagnostic_messages',arguments={'file_path':str(n)},expected_sha256=sha(view/n)) for n in files]
        requests.append({'tool':'exit'});data=''.join(json.dumps(r)+'\n' for r in requests)
        (view/'artifacts/requests.jsonl').write_text(data)
        with (view/'artifacts/service.log').open('xb') as log:
            code=subprocess.run([str(view/'.tools/lean-mcp/bin/python'),'tools/lean_session.py'],cwd=view,
                env=dict(os.environ,LEAN_NUM_THREADS='1'),input=data,text=True,stdout=log,stderr=subprocess.STDOUT,timeout=420).returncode
        if code:raise ValueError('original Lean client failed')
        rows=[json.loads(s) for s in (view/'artifacts/service.log').read_text().splitlines() if s.startswith('{')]
        rows=[r for r in rows if r.get('request',{}).get('tool')=='lean_diagnostic_messages']
        if len(rows)!=len(files):raise ValueError('physical file diagnostics missing/extra')
        audit=NEGATIVE if variant=='kill-success-release' else MODELS/('ConcreteAudit.lean' if variant=='caller' else 'Audit.lean')
        count=0
        for name,row in zip(files,rows):
            expected=expected_audits(root,variant) if name==audit else (expected_audits(root,'correct') if name==MODELS/'Audit.lean' else set())
            count+=check_record(view,row,expected,name,VERSIONS)
            for item in row['result']['result']['items']:
                if item.get('message','').startswith('Checked '):print(item['message'],flush=True)
        if any(sha(view/n)!=d for n,d in inputs.items()):raise ValueError('proof inputs changed during checks')
        record['views'][variant]={'inputs':inputs,'physical_files':len(files),'physical_audits':count,'build_exit_code':code,'service_sha256':sha(view/'artifacts/service.log')}
        (output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
    for name in COMMON:
        if len({sha(output/v/MODELS/name) for v in record['views']})!=1:raise ValueError('positive/negative contract/model differs')
    record.update(success=True,seconds=time.monotonic()-start)
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n');return record

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--root',type=Path,default=ROOT);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();print(json.dumps(run(args.root.resolve(strict=True),args.output.absolute()),indent=2))
