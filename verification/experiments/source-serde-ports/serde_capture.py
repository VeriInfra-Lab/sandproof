"""Fixed actual Serde typed-call admission and actual original/wrong Lean captures."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil

from serde_inputs import HERE, MODES, PROFILES, NAMESPACE, ROOT, SOURCE_FILES, configuration, retention, row_provider, stage
from binding import CALLS, name, required_call, typed_calls


def admit(profile, ir, generated):
    raw=json.loads(ir.read_bytes())
    if profile=='required-only': evidence=required_call(raw)
    else: evidence=typed_calls(raw, ['decode_source_payload'] if profile=='decode-only' else CALLS)
    if profile != 'call-admission':
        mapping=json.loads((generated/'translation.json').read_bytes())['functions']
        root=evidence['root'] if profile=='required-only' else evidence['decode_source_payload']['root']
        actual=[entry for entry in mapping if entry['def_id']==root['def_id']]
        if len(actual)!=1 or actual[0]['is_opaque'] is not False:
            raise ValueError('the actual selected generated Rust function must be transparent')
    return evidence


def capture(args):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')!='formal': raise ValueError('proof-machine role required')
    if args.mode!='correct' and args.profile!='required-only': raise ValueError('only the actual required_option variant is selected')
    if min(shutil.disk_usage(path).free for path in (args.output.parent,ROOT))<80*1024**3:
        raise ValueError('80 GiB of free capture and workspace capacity is required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    fixed=json.loads((ROOT/'verification/foundations-environment.json').read_bytes())
    charon,aeneas=args.tools/'charon/bin/charon',args.tools/'bin/aeneas'
    for name,path in (('charon',charon),('aeneas',aeneas)):
        if retention.sha(path)!=fixed[name]['binary_sha256']: raise ValueError('registered fixed tool differs: '+name)
    args.output.mkdir(mode=0o700)
    row=row_provider(args.output)
    from probe_prepare_iterators import run
    cfg=configuration(args.profile)
    report=dict(complete=False,profile=args.profile,mode=args.mode,source=retention.SOURCE,public_acceptance=False,
                lean_generated=False,commands=[],configuration=cfg)
    try:
        source=retention.snapshot(args.output)
        stage(row,source,args.mode)
        report['rules']={name:retention.sha(HERE/name) for name in ('serde_inputs.py','serde_capture.py','binding.py')}
        report['rust']={str(path):retention.sha(source/path) for path in SOURCE_FILES}
        selectors=[part for flag,key in (('--start-from','roots'),('--include','include'),('--opaque','opaque')) for value in cfg[key] for part in (flag,value)]
        ir,generated=args.output/'actual.llbc',args.output/'generated'
        env=dict(os.environ,CARGO_NET_OFFLINE='true',CARGO_BUILD_JOBS='4',CARGO_INCREMENTAL='0',CARGO_TARGET_DIR=str(args.output/'target'))
        for name in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS'):env.pop(name,None)
        commands=[('charon',[charon,'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default',*selectors,
                   '--dest-file',ir,'--','--manifest-path','controller-service/Cargo.toml','--lib','--locked','--offline','--target',fixed['target']],240)]
        if args.profile!='call-admission':
            commands.append(('aeneas',[aeneas,'-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json',
                         '-namespace',NAMESPACE,'-subdir',NAMESPACE,'-dest',generated,ir],120))
        with args.lock.open('r+') as guard:
            fcntl.flock(guard,fcntl.LOCK_EX|fcntl.LOCK_NB)
            for label,argv,limit in commands:
                receipt=run(label,argv,limit,source,args.output,env);report['commands'].append(receipt)
                if receipt['exit_code'] or receipt['bounded_stop']:raise ValueError(label+' did not complete')
        report['ir']=retention.sha(ir)
        report['admission']=admit(args.profile,ir,generated)
        if args.profile!='call-admission':
            report['generated']={str(path.relative_to(generated)):retention.sha(path) for path in sorted(generated.rglob('*')) if path.is_file()}
            report['lean_generated']=True
        report['complete']=True
    finally:
        retention.write_json(args.output/'result.json',report)
    print(args.profile,args.mode,'completed', 'fixed Charon/Aeneas' if report['lean_generated'] else 'B4 exact typed call admission only')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile',choices=PROFILES,required=True);parser.add_argument('--mode',choices=MODES,required=True)
    for name in ('output','tools','lock'):parser.add_argument('--'+name,type=lambda value:Path(value).resolve(),required=True)
    capture(parser.parse_args())
