"""Original-tool diagnostic of the actual OwnedChild entry; proof machine only.

A translation failure is retained as a tool limitation, never a counterexample.
No production source or tool is changed by this diagnostic.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

TOOLS = {'aeneas': '505b6ca35217e7be5c96c3e2f8045edfbdf47291',
         'aeneas/charon': 'b104e24fea7d721b71e6c39fd70f26ff20bc0980'}

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def run(source, output, tool_root):
    if output.exists() or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('fresh evidence directory and dedicated formal target required')
    for name, revision in TOOLS.items():
        if subprocess.check_output(['git', '-C', str(tool_root/name), 'rev-parse', 'HEAD'], text=True).strip() != revision:
            raise ValueError('original tool revision mismatch: '+name)
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = {'kind': 'child-observation-original-entry-diagnostic', 'tools': TOOLS,
              'sources': {name:sha(source/name) for name in ('sandbox-runtime/src/child_process.rs',
                  'sandbox-runtime/src/child_process/reaper.rs', 'sandbox-runtime/src/lib.rs',
                  'sandbox-runtime/Cargo.toml', 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')}}
    llbc = output/'original.llbc'
    charon = tool_root/'aeneas/charon/bin/charon'
    aeneas = tool_root/'aeneas/bin/aeneas'
    record['binary_sha256'] = {'charon':sha(charon), 'aeneas':sha(aeneas)}
    command = [str(charon), 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
        '--targets', 'x86_64-unknown-linux-gnu', '--opaque', 'std::io', '--opaque', 'alloc::io', '--opaque', 'std::process',
        '--opaque', 'std::time', '--opaque', 'std::thread',
        '--opaque', 'crate::child_process::reaper', '--include', 'sandbox_runtime',
        '--start-from', 'crate::child_process::_::stop_and_reap', '--dest-file', str(llbc),
        '--', '--manifest-path', 'sandbox-runtime/Cargo.toml', '--lib', '--locked', '--offline']
    record['charon_command'] = command
    environment = dict(os.environ, CARGO_INCREMENTAL='0', CARGO_NET_OFFLINE='true')
    with (output/'charon.log').open('xb') as log:
        try:
            record['charon_exit_code'] = subprocess.run(command, cwd=source, env=environment,
                stdout=log, stderr=subprocess.STDOUT, timeout=300).returncode
        except subprocess.TimeoutExpired:
            record['charon_timed_out'] = True
    if llbc.exists():
        data = json.loads(llbc.read_text())
        record.update(has_errors=data['has_errors'], llbc_sha256=sha(llbc))
        record['local_functions'] = [dict(name=f['item_meta']['name'], structured=isinstance(f['body'],dict)
            and 'Structured' in f['body']) for f in data['translated']['fun_decls'] if f and f['item_meta']['is_local']]
        if record.get('charon_exit_code')==0 and not data['has_errors']:
            generated=output/'generated'; generated.mkdir()
            command=[str(aeneas), '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                '-split-files', '-emit-json', '-namespace', 'SandproofChildObservationRust',
                '-subdir', 'SandproofChildObservation', '-dest', str(generated), str(llbc)]
            record['aeneas_command']=command
            with (output/'aeneas.log').open('xb') as log:
                try:
                    record['aeneas_exit_code']=subprocess.run(command,cwd=source,env=environment,
                        stdout=log,stderr=subprocess.STDOUT,timeout=300).returncode
                except subprocess.TimeoutExpired:
                    record['aeneas_timed_out']=True
            record['generated']={str(p.relative_to(generated)):sha(p) for p in generated.rglob('*') if p.is_file()}
    record.update(seconds=time.monotonic()-started,
        success=record.get('charon_exit_code')==0 and record.get('has_errors') is False and record.get('aeneas_exit_code')==0)
    if any(sha(source/name)!=digest for name,digest in record['sources'].items()):
        raise ValueError('source changed during preflight')
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps({k:v for k,v in record.items() if k not in ('local_functions','sources')},indent=2))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--tool-root',type=Path,required=True)
    args=parser.parse_args()
    run(args.source.resolve(strict=True),args.output.absolute(),args.tool_root.resolve(strict=True))
