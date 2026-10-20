"""Preserve actual shared-lock backend limitations; never semantic negatives."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_first_chain_extraction import check_tools


def sources():
    names = subprocess.check_output(['git','ls-files','-z'],cwd=ROOT).decode().split('\0')
    return {n: hashlib.sha256((ROOT/n).read_bytes()).hexdigest() for n in names if n and
            (n in ('Cargo.toml','Cargo.lock','rust-toolchain.toml') or
             n.split('/')[0].startswith(('sandbox-','controller-')))}


def preflight(output):
    tools=Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT',str(ROOT/'.tools')))
    check_tools(ROOT,tools)
    if 'CARGO_TARGET_DIR' not in os.environ:
        raise ValueError('Use the registered formal compiler target')
    output.mkdir(parents=True,exist_ok=False)
    before=sources();start=time.monotonic();records=[]
    for name,manifest,crate,pattern,expected in (
        ('pty','sandbox-guest-runtime/Cargo.toml','sandproof_guest_runtime',
         'crate::pty::_::descriptor','Dynamic trait types are not supported'),
        ('meter','sandbox-guest-service/Cargo.toml','sandproof_guest_service',
         'crate::task_accounting::_::lock',"Can't end abstraction")):
        directory=output/name;directory.mkdir();llbc=directory/'original.llbc'
        command=[str(tools/'aeneas/charon/bin/charon'),'cargo','--preset=aeneas',
            '--error-on-warnings','--sysroot','default','--opaque','std::sync',
            '--opaque','tokio','--opaque','rustix','--include',crate,'--start-from',pattern,
            '--dest-file',str(llbc),'--','--manifest-path',manifest,'--lib','--locked','--offline']
        with (directory/'charon.log').open('xb') as log:
            code=subprocess.run(command,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,timeout=900).returncode
        if code or not llbc.is_file():
            raise ValueError('Shared-lock frontend changed; inspect original evidence: '+str(directory))
        tree=json.loads(llbc.read_text())
        bodies=[f for f in tree['translated']['fun_decls'] if f and f['item_meta']['is_local']
                and isinstance(f['body'],dict) and 'Structured' in f['body']]
        if tree.get('has_errors') is not False or not bodies:
            raise ValueError('No actual transparent shared-lock root')
        generated=directory/'generated';generated.mkdir()
        backend=[str(tools/'aeneas/bin/aeneas'),'-backend','lean','-abort-on-error',
            '-warnings-as-errors','-split-files','-emit-json','-namespace','StdMutexPreflightRust',
            '-subdir','StdMutexPreflight','-dest',str(generated),str(llbc)]
        with (directory/'aeneas.log').open('xb') as log:
            failed=subprocess.run(backend,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT,timeout=900).returncode
        text=(directory/'aeneas.log').read_text()
        # Fixed-tool unsupported behavior is source/preflight evidence only.
        # An improved or different result requires review, not a semantic pass.
        if failed!=2 or expected not in text:
            raise ValueError('Shared-lock backend differs from reviewed limitation: '+str(directory))
        records.append(dict(name=name,charon_command=command,charon_exit=code,
            aeneas_command=backend,aeneas_exit=failed,expected_limitation=expected,
            llbc_sha256=hashlib.sha256(llbc.read_bytes()).hexdigest()))
    result=dict(kind='translation-limitations-not-semantic-counterexamples',probes=records,
                sources=before,sources_unchanged=sources()==before,seconds=round(time.monotonic()-start,3))
    (output/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    if not result['sources_unchanged']:
        raise ValueError('Production sources changed during original shared-lock preflight')
    print(json.dumps(dict(kind=result['kind'],seconds=result['seconds'],evidence=str(output))))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    preflight(parser.parse_args().output.resolve())
