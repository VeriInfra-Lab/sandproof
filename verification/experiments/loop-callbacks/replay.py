"""Rebuild original and wrong Rust query bodies with the unchanged tools on a6000.

This is the formal component. scripts/verify-loop-callbacks.sh additionally
requires the sealed ning component receipt through the existing admission API.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import time

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from tools.check_loop_callbacks_contract import (check,check_extraction,LLBC,DEST,
    llbc_digest)
from tools.check_loop_callbacks_components import mutate
from tools.check_first_chain_extraction import check_tools


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command,root,env,log):
    with log.open('xb') as stream:
        code=subprocess.run([str(x) for x in command],cwd=root,env=env,
            stdout=stream,stderr=subprocess.STDOUT,timeout=900).returncode
    if code:
        raise ValueError('Formal command failed (not a semantic refutation): '+str(log))
    return dict(command=[str(x) for x in command],exit_code=code,log_sha256=sha(log))


def frozen_head(root,contract):
    names=contract['inputs']+['verification/loop-callbacks-inputs.sha256']
    for name in names:
        content=subprocess.check_output(['git','show','HEAD:'+name],cwd=root)
        if content!=(root/name).read_bytes():
            raise ValueError('Commit the complete frozen query inputs before replay: '+name)
    return subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip()


def prepare(root,archive,destination):
    destination.mkdir()
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        tree.extractall(destination,filter='data')
    # This is a private source export. Fixed tools/packages are read-only inputs;
    # writable .lake/build is distinct for correct and each wrong generated body.
    (destination/'.tools').symlink_to((root/'.tools').resolve(),target_is_directory=True)
    (destination/'.lake').mkdir()
    (destination/'.lake/packages').symlink_to((root/'.lake/packages').resolve(),target_is_directory=True)
    (destination/'lakefile.toml').write_bytes((root/'lake-loop-callbacks.toml').read_bytes())
    return destination


def library_sources(root,tool_root):
    frozen=json.loads((root/'verification/foundations-environment.json').read_text())
    charon=tool_root/'aeneas/charon/bin/charon'
    frontend=Path(subprocess.check_output([str(charon),'toolchain-path'],text=True).strip())/'bin/rustc'
    result={}
    for role,compiler in [('production','rustc'),('extraction',str(frontend))]:
        sysroot=Path(subprocess.check_output([compiler,'--print','sysroot'],text=True).strip())
        library=sysroot/'lib/rustlib/src/rust/library'
        observed={name:sha(library/name) for name in frozen['library_sources'][role]}
        if observed!=frozen['library_sources'][role]:
            raise ValueError('Original standard-library source changed: '+role)
        result[role]=observed
    return result


def service(project,files,expected_roots,out,env):
    requests=[dict(tool='lean_diagnostic_messages',arguments=dict(file_path=name,timeout_s=120),
        expected_sha256=sha(project/name)) for name in files]
    text=''.join(json.dumps(row)+'\n' for row in requests)+'{"tool":"exit"}\n'
    (out/'requests.jsonl').write_text(text)
    log=out/'service.log'
    with log.open('xb') as stream:
        process=subprocess.Popen([str(project/'.tools/lean-mcp/bin/python'),'tools/lean_session.py'],
            cwd=project,env=env,stdin=subprocess.PIPE,stdout=stream,stderr=subprocess.STDOUT,text=True)
        process.communicate(text,timeout=900)
    if process.returncode:
        raise ValueError('Original Lean service client failed: '+str(log))
    rows=[]
    for line in log.read_text().splitlines():
        try:row=json.loads(line)
        except ValueError:continue
        if row.get('request',{}).get('tool')=='lean_diagnostic_messages':rows.append(row)
    if len(rows)!=len(requests):
        raise ValueError('Missing actual-file diagnostic result')
    checked=set()
    for request,row in zip(requests,rows):
        if (row.get('request')!=request or row.get('stale') is not False
                or row['revision']['file_sha256']!=request['expected_sha256']):
            raise ValueError('Changed/stale actual proof source')
        result=row['result']
        for _ in range(4):
            if 'success' in result:break
            result=result.get('structuredContent',result.get('result',{}))
        diagnostics=result.get('items',[])
        if (result.get('success') is not True or result.get('partial') is not False
                or result.get('timed_out') is not False or result.get('failed_dependencies')
                or result.get('still_elaborating_lines') not in (None,[])
                or any(d.get('severity') in ('error','warning') for d in diagnostics)):
            raise ValueError('Incomplete/rejected physical proof diagnostics: '+str(log))
        actual={m[1] for d in diagnostics if (m:=re.fullmatch(
            r'Checked (\S+): \d+ standard logical dependencies',d.get('message','')))}
        source_expected=set(re.findall(r'^#audit_controller_proof (\S+)$',
            (project/request['arguments']['file_path']).read_text(),re.M))
        if actual!=source_expected:
            raise ValueError('Missing/extra actual-file audit roots')
        checked.update(actual)
    if checked!=set(expected_roots):
        raise ValueError('Physical service audit differs from frozen contract roots')
    return dict(actual_files=len(files),audit_roots=sorted(checked),
        service_log_sha256=sha(log),requests_sha256=sha(out/'requests.jsonl'))


def reproduce(root,tool_root,output):
    root=root.resolve();tool_root=tool_root.resolve();output=output.resolve()
    contract=check(root);commit=frozen_head(root,contract)
    check_tools(root,tool_root)
    if subprocess.check_output(['git','-C',str(root/'.lake/packages/aeneas'),
            'status','--porcelain','--untracked-files=no'],text=True):
        raise ValueError('Original Lean backend has tracked source modifications')
    libraries=library_sources(root,tool_root)
    output.mkdir(parents=True,exist_ok=False)
    started=time.monotonic()
    archive=subprocess.check_output(['git','archive','HEAD'],cwd=root)
    (output/'source.tar').write_bytes(archive)
    tracked=subprocess.check_output(['git','ls-files','-z'],cwd=root,text=True).split('\0')
    rust_inputs={name:sha(root/name) for name in tracked if name.endswith('.rs')}
    env=dict(os.environ,SANDPROOF_PROOF_TOOL_ROOT=str(tool_root))
    record=dict(kind='original-Rust-shared-query-formal-component',commit=commit,
        source_archive_sha256=sha(output/'source.tar'),library_sources=libraries,variants={})
    for variant in ['correct','skip-first','invert-presence']:
        begin=time.monotonic();out=output/variant;out.mkdir()
        project=prepare(root,archive,out/'source')
        path=project/'controller-core/src/store.rs'
        text,mutation=mutate(path.read_text(),variant)
        path.write_text(text)
        if variant!='correct' and sha(path)!=contract['wrong_variants'][variant]['mutated_source_sha256']:
            raise ValueError('Wrong Rust is not the frozen single mutation')
        for name,expected in rust_inputs.items():
            if name!='controller-core/src/store.rs' and sha(project/name)!=expected:
                raise ValueError('Original source missing/changed in private export: '+name)
        (out/'mutation.json').write_text(json.dumps(mutation,indent=2)+'\n')
        one=dict(original_store_sha256=sha(root/'controller-core/src/store.rs'),
            actual_store_sha256=sha(path),commands=[])
        one['commands'].append(run(['bash','scripts/regenerate-loop-callbacks-lean.sh'],
            project,env,out/'generate.log'))
        llbc=json.loads((project/LLBC).read_text())
        inventory=json.loads((project/DEST/'translation.json').read_text())
        selected=contract if variant=='correct' else contract | {
            'reviewed_opaque_signatures':contract['wrong_variants'][variant]['reviewed_opaque_signatures']}
        check_extraction(project,llbc,inventory,selected)
        expected_digest=selected['llbc_digest'] if variant=='correct' else contract['wrong_variants'][variant]['llbc_digest']
        if llbc_digest(llbc)!=expected_digest:
            raise ValueError('Complete regenerated LLBC differs: '+variant)
        actual={str(p.relative_to(project)):sha(p) for p in (project/DEST).rglob('*') if p.is_file()}
        expected=({name:sha(root/name) for name in actual} if variant=='correct' else
            {name:value for name,value in contract['wrong_variants'][variant]['generated'].items() if name!=LLBC})
        if actual!=expected:
            raise ValueError('Complete actual generated Lean/inventory differs: '+variant)
        if variant=='correct':
            audit='SandboxModel/Foundations/LoopCallbacks/Audit.lean'
            one['commands'].append(run(['lake','build','SandboxModel.Foundations.LoopCallbacks.Audit',
                'SandboxModel.Foundations.LoopCallbacks'],project,env,out/'build.log'))
            files=['SandboxModel/Foundations/LoopCallbacks.lean']+[
                'SandboxModel/Foundations/LoopCallbacks/'+module+'.lean'
                for module in ['Contracts','Loop','Refinement','Examples','Audit']]
            expected_roots=contract['theorems']
        else:
            info=contract['wrong_variants'][variant]
            audit='SandboxModel/Foundations/LoopCallbacks/Counterexample.lean'
            (project/audit).write_bytes((root/info['template']).read_bytes())
            one['commands'].append(run(['lake','build','SandboxModel.Foundations.LoopCallbacks.Counterexample'],
                project,env,out/'build.log'))
            files=[audit];expected_roots=info['theorems']
        one['commands'].append(run(['lake','env','lean',audit],project,env,out/'kernel-audit.log'))
        one['service']=service(project,files,expected_roots,out,env)
        one.update(success=True,seconds=round(time.monotonic()-begin,3),llbc_digest=llbc_digest(llbc),
            generated=actual,actual_llbc_sha256=sha(project/LLBC))
        record['variants'][variant]=one
        (out/'result.json').write_text(json.dumps(one,indent=2)+'\n')
    if check(root)!=contract or frozen_head(root,contract)!=commit:
        raise ValueError('Module inputs changed during formal replay')
    record.update(complete=True,exit_code=0,seconds=round(time.monotonic()-started,3))
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps(dict(complete=True,kind=record['kind'],commit=commit,
        correct_audits=len(contract['theorems']),wrong_variants=2,seconds=record['seconds'])))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--tool-root',type=Path,default=Path(os.environ.get(
        'SANDPROOF_PROOF_TOOL_ROOT',str(ROOT/'.tools'))))
    args=parser.parse_args()
    reproduce(ROOT,args.tool_root,args.output)
