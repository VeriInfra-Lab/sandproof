"""Compile/extract actual swallow-poison Rust and refute the same GetMutContract."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import time

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from tools.check_std_mutex_contract import check, walk
from tools.check_first_chain_extraction import check_tools

BEFORE='        self.0.get_mut()\n'
AFTER='''        match self.0.get_mut() {
            Ok(value) => Ok(value),
            Err(error) => Ok(error.into_inner()),
        }
'''


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command,root,log):
    with log.open('xb') as stream:
        code=subprocess.run(command,cwd=root,stdout=stream,stderr=subprocess.STDOUT,timeout=900).returncode
    if code:
        raise ValueError('Failed build/extraction is not a semantic counterexample: '+str(log))


def borrowed_binding(tree):
    """Admit only this actual PoisonError<&mut T> specialization, not owned T."""
    functions=[f for f in tree['translated']['fun_decls'] if f]
    root=[f for f in functions if f['item_meta']['is_local']]
    if len(root)!=1 or not isinstance(root[0]['body'],dict) or 'Structured' not in root[0]['body']:
        raise ValueError('Wrong variant must retain its actual transparent Rust body')
    recovery=[f for f in functions if
        [n['Ident'][0] for n in f['item_meta']['name'] if 'Ident' in n]==
        ['std','sync','poison','into_inner']]
    if len(recovery)!=1 or recovery[0]['body']!='Opaque':
        raise ValueError('Missing original opaque poison-recovery function')
    calls=[n['Call']['call'] for n in walk(root[0]['body']) if isinstance(n,dict) and set(n)=={'Call'}]
    bindings=[c['func']['Regular'] for c in calls if
              c['func'].get('Regular',{}).get('kind')=={'Fun':recovery[0]['def_id']}]
    if len(calls)!=2 or len(bindings)!=1:
        raise ValueError('Unreviewed extra call or poison-recovery use')
    types=bindings[0]['generics']['types']
    if len(types)!=1 or set(types[0])!={'Value'}:
        raise ValueError('Review the actual borrowed recovery type before admitting it')
    value=types[0]['Value'][1]
    if set(value)!={'Ref'} or value['Ref'][2]!='Mut':
        raise ValueError('Restricted borrowed poison model cannot admit an owned payload')
    return dict(root_def_id=root[0]['def_id'],recovery_def_id=recovery[0]['def_id'],
                actual_borrowed_call=bindings[0],scope='PoisonError<&mut T> only')


def reproduce(output):
    contract=check();tools=Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT',str(ROOT/'.tools')))
    check_tools(ROOT,tools)
    if 'CARGO_TARGET_DIR' not in os.environ:
        raise ValueError('Reuse registered formal compiler target')
    output.mkdir(parents=True,exist_ok=False);start=time.monotonic()
    root=output/'source';root.mkdir(mode=0o700)
    archive=subprocess.check_output(['git','archive','HEAD'],cwd=ROOT)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        tree.extractall(root,filter='data')
    path=root/'controller-service/src/application/diagnostics.rs'
    if path.read_bytes()!=(ROOT/path.relative_to(root)).read_bytes() or path.read_text().count(BEFORE)!=1:
        raise ValueError('Wrong source must be exactly the frozen original plus one mutation')
    original=sha(path);path.write_text(path.read_text().replace(BEFORE,AFTER))
    mutated=sha(path);llbc=output/'wrong-original.llbc'
    command=[str(tools/'aeneas/charon/bin/charon'),'cargo','--preset=aeneas','--error-on-warnings',
        '--sysroot','default','--opaque','std::sync','--opaque','tokio','--opaque','rustix',
        '--include','sandproof_controller_service','--start-from','crate::application::diagnostics::_::get_mut',
        '--dest-file',str(llbc),'--','--manifest-path','controller-service/Cargo.toml','--lib',
        '--features','contention-diagnostics','--locked','--offline']
    run(command,root,output/'charon.log')
    tree=json.loads(llbc.read_text())
    if tree.get('has_errors') is not False:
        raise ValueError('Wrong Rust extraction must succeed before its semantic refutation')
    binding=borrowed_binding(tree)
    generated=root/'verification/aeneas/std-mutex-wrong';generated.mkdir()
    backend=[str(tools/'aeneas/bin/aeneas'),'-backend','lean','-abort-on-error','-warnings-as-errors',
        '-split-files','-emit-json','-namespace','StdMutexWrongRust','-subdir','StdMutexWrong',
        '-dest',str(generated),str(llbc)]
    run(backend,root,output/'aeneas.log')
    own=generated/'StdMutexWrong'
    emitted={n:sha(own/n) for n in ('Types.lean','Funs.lean')}
    for name in ('TypesExternal','FunsExternal'):
        shutil.copyfile(ROOT/f'verification/experiments/std-mutex/negative/{name}.lean.in',own/(name+'.lean'))
    proof='SandboxModel/Boundaries/StdMutex/Negative.lean'
    shutil.copyfile(ROOT/'verification/experiments/std-mutex/negative/Negative.lean.in',root/proof)
    with (root/'lakefile.toml').open('a') as stream:
        stream.write('\n[[lean_lib]]\nname = "StdMutexWrong"\nsrcDir = "verification/aeneas/std-mutex-wrong"\n')
    (root/'.tools').symlink_to(ROOT/'.tools',target_is_directory=True)
    (root/'.lake').mkdir();(root/'.lake/packages').symlink_to(ROOT/'.lake/packages',target_is_directory=True)
    run(['lake','build','SandboxModel.Boundaries.StdMutex.Negative'],root,output/'module.log')
    request=dict(tool='lean_diagnostic_messages',arguments={'file_path':proof},expected_sha256=sha(root/proof))
    requests=json.dumps(request)+'\n{"tool":"exit"}\n'
    (output/'requests.jsonl').write_text(requests)
    with (output/'service.log').open('xb') as log:
        code=subprocess.run([str(root/'.tools/lean-mcp/bin/python'),'tools/lean_session.py'],cwd=root,
            input=requests,text=True,stdout=log,stderr=subprocess.STDOUT,timeout=900).returncode
    if code:
        raise ValueError('Original Lean service client failed')
    rows=[]
    for line in (output/'service.log').read_text().splitlines():
        try:row=json.loads(line)
        except ValueError:continue
        if row.get('request',{}).get('tool')=='lean_diagnostic_messages':rows.append(row)
    if len(rows)!=1 or rows[0].get('request')!=request or rows[0].get('stale') is not False or rows[0]['revision']['file_sha256']!=request['expected_sha256']:
        raise ValueError('Missing or stale actual negative-file diagnostic')
    result=rows[0]['result']
    for _ in range(3):
        if 'success' in result:break
        result=result.get('structuredContent',result.get('result',{}))
    if (result.get('success') is not True or result.get('partial') is not False or
        result.get('timed_out') is not False or result.get('failed_dependencies') or
        result.get('still_elaborating_lines') not in (None,[]) or
        any(d.get('severity') in ('warning','error') for d in result.get('items',[]))):
        raise ValueError('Actual negative-file proof incomplete or rejected')
    expected=set(re.findall(r'^#audit_controller_proof (\S+)$',(root/proof).read_text(),re.M))
    checked={m[1] for d in result['items'] if (m:=re.fullmatch(
        r'Checked (\S+): \d+ standard logical dependencies',d.get('message','')))}
    if len(expected)!=1 or checked!=expected:
        raise ValueError('Actual semantic refutation audit missing or extra')
    if {n:sha(own/n) for n in emitted}!=emitted or sha(path)!=mutated:
        raise ValueError('Original wrong source or generated bodies changed during proof')
    receipt=dict(complete=True,kind='actual-wrong-rust-refuted-by-same-get-mut-contract',
        contract_id=contract['contract_id'],source=str(root),original_sha256=original,wrong_sha256=mutated,
        mutation=dict(before=BEFORE,after=AFTER),charon_command=command,aeneas_command=backend,
        generated=emitted,borrowed_binding=binding,audited=list(sorted(expected)),
        service_sha256=sha(output/'service.log'),seconds=round(time.monotonic()-start,3))
    (output/'result.json').write_text(json.dumps(receipt,indent=2)+'\n')
    print(json.dumps(dict(complete=True,seconds=receipt['seconds'],evidence=str(output))))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    reproduce(parser.parse_args().output.resolve())
