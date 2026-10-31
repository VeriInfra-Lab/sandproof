"""Original/candidate/mutant development checks; no shared native seal is issued."""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import shutil
import subprocess
import tarfile
import time
import tomllib
import traceback

ROOT=Path(__file__).resolve().parents[4]
HERE=Path('verification/experiments/stage1-protocol/envelope-writer')
spec=importlib.util.spec_from_file_location('writer_native_formal',ROOT/HERE/'formal.py')
formal=importlib.util.module_from_spec(spec);spec.loader.exec_module(formal)


def registry():
    """Locked archive bytes and the actual unpacked compiler inputs, not names."""
    cargo=Path(os.environ.get('CARGO_HOME',str(Path.home()/'.cargo')))
    lock=tomllib.loads((ROOT/HERE/'Cargo.lock').read_text());archives={};files={}
    for package in lock['package']:
        if 'checksum' not in package: continue
        stem=package['name']+'-'+package['version']
        matches=list((cargo/'registry/cache').glob('*/'+stem+'.crate'))
        if not matches: raise ValueError('missing locked crate: '+stem)
        for archive in matches:
            if formal.sha(archive)!=package['checksum']: raise ValueError('changed locked crate: '+stem)
            archives[str(archive)]=package['checksum']
            source=cargo/'registry/src'/archive.parent.name/stem
            if not source.is_dir(): raise ValueError('missing actual unpacked crate: '+stem)
            expected=set()
            with tarfile.open(archive,'r:gz') as tar:
                for member in tar:
                    if member.isdir(): continue
                    if not member.isfile(): raise ValueError('unreviewed crate archive member')
                    relative=Path(member.name).relative_to(stem)
                    if relative.is_absolute() or '..' in relative.parts: raise ValueError('unsafe crate path')
                    path=source/relative
                    if path.is_symlink() or not path.is_file(): raise ValueError('missing regular crate source: '+str(path))
                    original=tar.extractfile(member).read()
                    import hashlib
                    wanted=hashlib.sha256(original).hexdigest()
                    if formal.sha(path)!=wanted: raise ValueError('actual crate differs from lock archive: '+str(path))
                    expected.add(str(relative));files[str(path)]=wanted
            extras={str(p.relative_to(source)) for p in source.rglob('*') if p.is_file()}-expected
            if extras- {'.cargo-checksum.json','.cargo-ok'}:
                raise ValueError('unregistered compiler input in crate: '+stem+': '+repr(sorted(extras)))
            for name in extras: files[str(source/name)]=formal.sha(source/name)
    return dict(archives=archives,files=files)


def run(output):
    output.mkdir(parents=True,mode=0o700);start=time.monotonic()
    record=dict(kind='writer-native-development',complete=False,full_acceptance=False,commands=[])
    try:
        formal.inventory(ROOT);inputs=formal.snapshot(ROOT);libraries=registry()
        record.update(inputs=inputs,registry=libraries,source_commit=subprocess.check_output(
            ['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip())
        record['tools']={}
        for name,args in [('cargo',['-V']),('rustc',['-Vv']),('clippy-driver',['--version'])]:
            binary=Path(subprocess.check_output(['rustup','which',name],text=True).strip())
            record['tools'][name]=dict(path=str(binary),sha256=formal.sha(binary),
                version=subprocess.check_output([str(binary),*args],text=True))
        for variant in ['original','correct','swallow-error']:
            project=output/variant/'source';project.mkdir(parents=True,mode=0o700)
            expected=dict(inputs)
            for name,digest in inputs.items():
                path=project/name;path.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/name,path)
                if formal.sha(path)!=digest: raise ValueError('source changed while copying')
            if variant!='original':
                transform=formal.preparer.prepare(project,variant)
                for row in transform['files']:expected[row['path']]=row['after_sha256']
            env=dict(os.environ,CARGO_TARGET_DIR=str(output/variant/'target'),CARGO_BUILD_JOBS='4',CARGO_NET_OFFLINE='true')
            feature=[] if variant=='original' else ['--features','candidate']
            for label,extra in [('test',['--lib','--message-format=json','--','--nocapture']),
                                ('clippy',['--all-targets','--','-D','warnings'])]:
                argv=['cargo',label,'--manifest-path',str(HERE/'Cargo.toml'),'--locked','--offline',*feature,*extra]
                log=output/variant/(label+'.log')
                with log.open('xb') as stream:
                    done=subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,
                        stdout=stream,stderr=subprocess.STDOUT,timeout=1800)
                text=log.read_text();row=dict(variant=variant,argv=argv,exit_code=done.returncode,log_sha256=formal.sha(log))
                if label=='test':
                    artifacts=[]
                    for line in text.splitlines():
                        try: message=json.loads(line)
                        except ValueError: continue
                        if message.get('reason')=='compiler-artifact':artifacts.append(message)
                    own=[a for a in artifacts if a['target']['name'] in ('sandproof_controller_service','sandproof_envelope_writer_check')]
                    if len(own)!=2 or any(a['fresh'] for a in own): raise ValueError('own actual libraries/test not freshly compiled')
                    row['artifacts']=[dict(message=a,files={n:formal.sha(Path(n)) for n in a['filenames'] if Path(n).is_file()}) for a in own]
                    if variant=='swallow-error':
                        if (done.returncode!=101 or '2 passed; 1 failed; 0 ignored;' not in text
                                or 'same complete encoding contract lost the original serializer error at 0: Ok([])' not in text
                                or 'standard_full_envelopes_stop_at_every_allocation_failure ... FAILED' not in text):
                            raise ValueError('mutant did not reach the registered original assertion')
                        row['property_rejected']=True
                    else:
                        count=1 if variant=='original' else 3
                        if done.returncode or f'{count} passed; 0 failed; 0 ignored;' not in text:
                            raise ValueError('correct native assertions failed')
                        if variant=='correct' and 'at 3561 exact allocation failure points' not in text:
                            raise ValueError('missing registered real failure-point coverage')
                elif done.returncode: raise ValueError('native clippy failed: '+variant)
                record['commands'].append(row)
            if any(formal.sha(project/n)!=h for n,h in expected.items()):raise ValueError('checked candidate changed')
        if inputs!=formal.snapshot(ROOT) or libraries!=registry():raise ValueError('source/library inputs changed during checking')
        record['complete']=True
    except BaseException as error:record.update(error=repr(error),traceback=traceback.format_exc())
    record['seconds']=time.monotonic()-start
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',required=True,type=Path)
    args=parser.parse_args();os.umask(0o077);result=run(args.output.absolute())
    print(json.dumps({k:result.get(k) for k in ['complete','seconds','error','full_acceptance']}))
    raise SystemExit(0 if result['complete'] else 1)
