"""Formal-only module acceptance with checked, conservative result reuse."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.acceptance_receipts import ROOT, digest, locked, load_success, rules, rust_inputs, sha, write_json, tree_digest, validate_native
from tools.check_foundations_extraction import llbc_digest

MODULES = ('json-request','tls-pin','json-identifiers','json-counter','task-contract',
           'revocation-contract','owned-message','notify','joinset','std-mutex','condvar','tokio-rwlock','tunnel','data-drain','sqlite','foundations','network',
           'first-chain-linux','first-chain','storage',
           'schema-domains','loop-callbacks','sqlite-statements','redis-lua')

# SQLite emits its admitted kernel counterexamples as well as positive roots.
# Include those actual audits in the complete inventory.
EXTRA_AUDIT_FILES = ('verification/experiments/sqlite-statements/negative/Counterexample.lean',)

# These generators intentionally reuse a fixed workspace output location.
# Keep immutable raw copies in the success entry, even when the directory
# already existed before this invocation. Other module outputs are tracked
# source files or fresh, uniquely named evidence directories below.
FIXED_EVIDENCE = {'json-request': (
    'artifacts/json-request-binding/encode.llbc',
    'artifacts/json-request-binding/decode.llbc')}


def file_digest(path):
    if path.suffix == '.llbc':
        return llbc_digest(json.loads(path.read_text()))
    return sha(path)


def tracked_inputs(root=ROOT):
    names=subprocess.check_output(['git','ls-files','-z'],cwd=root).decode().split('\0')
    return {n:file_digest(root/n) for n in names if n}


def closure(root, seeds):
    """Resolve every local Lean import, including imports omitted by roots.

    External packages and the installed compiler/stdlib are covered by the
    runtime fingerprint, not guessed from an import list.
    """
    import tomllib
    libraries = {'SandboxModel': root}
    for view in root.glob('lake*.toml'):
        for lib in tomllib.loads(view.read_text()).get('lean_lib', []):
            libraries[lib['name']] = root / lib.get('srcDir', '.')
    result = {}
    queue = list(seeds)
    while queue:
        name = queue.pop()
        if name in result:
            continue
        path = root / name
        result[name] = file_digest(path)
        if path.suffix == '.sha256':
            for line in path.read_text().splitlines():
                match = re.fullmatch(r'[0-9a-f]{64}  (.+)', line)
                if not match or Path(match[1]).is_absolute() or '..' in Path(match[1]).parts:
                    raise ValueError('malformed input manifest: ' + name)
                queue.append(match[1])
        if path.suffix == '.lean':
            for line in path.read_text().splitlines():
                header=re.match(r'^\s*(?:(?:public|private|meta)\s+)?import\s+(.+)$',line.lstrip('\ufeff'))
                if not header:
                    continue
                for module in header[1].split('--', 1)[0].split():
                    if module == 'all':
                        continue
                    prefix = module.split('.')[0]
                    if prefix in libraries:
                        target = libraries[prefix] / (module.replace('.', '/') + '.lean')
                        queue.append(str(target.relative_to(root)))
    return dict(sorted(result.items()))


def module_key(name, command, root=ROOT):
    if name == 'retained':
        seeds = ['SandboxModel.lean','verification/ProofAudit.lean']
        seeds += [str(p.relative_to(root)) for d in ('SandboxModel','verification/aeneas')
                  for p in (root/d).rglob('*.lean')]
        seeds += [str(p.relative_to(root)) for p in (root/'verification').glob('*inputs.sha256')]
    else:
        manifest = 'foundations' if name == 'foundations-generation' else name
        seeds = [f'verification/{manifest}-inputs.sha256','verification/acceptance-inputs.sha256']
    seeds += [p.name for p in root.glob('lake*.toml')]
    seeds += [n for n in ('lean-toolchain','lake-manifest.json','rust-toolchain.toml') if (root/n).is_file()]
    # Full Rust closure is deliberately conservative. A Rust change can
    # invalidate extra modules, but can never silently miss an indirect use.
    return dict(schema=1, module=name, command=command, rust=rust_inputs(root),
                rules=rules(root), inputs=closure(root,seeds),
                runtime=json.loads(Path(os.environ['SANDPROOF_FORMAL_RUNTIME']).read_text()),
                native=native_dependencies(Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])))


def native_dependencies(cache):
    seal=json.loads((cache/'complete.json').read_text())
    commands={}
    for identity,value in seal.get('commands',{}).items():
        key=json.loads((cache/'commands'/identity/'receipt.json').read_text())['key']
        if (key.get('label')!='functional' and key.get('rules')==seal.get('rules')
                and key.get('runtime')==seal.get('runtime')):
            commands[identity]=value
    # The full functional gate is mandatory admission for every batch, but a
    # Markdown-only change to that gate's inputs does not change a Lean module.
    return dict(rules=seal.get('rules'),rust=seal.get('rust'),runtime=seal.get('runtime'),commands=commands)


def evidence_files(directory):
    result = {}
    for parent, dirs, files in os.walk(directory):
        dirs[:] = [n for n in dirs if not ('target' in n or n in ('.git','.lake','.tools','node_modules'))]
        for name in files:
            path = Path(parent)/name
            if path.is_symlink() or path.suffix not in ('.log','.json','.jsonl','.llbc','.lean','.sha256'):
                continue
            result[str(path.resolve())] = sha(path)
    return result


def step(name, command, root=ROOT):
    step_started=time.monotonic()
    cache = Path(os.environ['SANDPROOF_FORMAL_CACHE']).resolve()
    key = module_key(name, command, root)
    entry = cache / 'modules' / name / digest(key)
    with locked(entry):
        prior = load_success(entry,key)
        if prior is not None:
            try:
                if any(sha(path)!=expected for path,expected in prior.get('evidence',{}).items()):
                    prior = None
            except OSError:
                prior = None
        force = os.environ.get('SANDPROOF_PROOF_RECOMPUTE') == '1'
        same_batch = prior is not None and prior.get('batch') == os.environ.get('SANDPROOF_PROOF_BATCH')
        if prior is not None and (not force or same_batch):
            print(f'REUSE {name} {entry}',flush=True)
            sys.stdout.write((entry/'command.log').read_text())
            return dict(module=name,reused=True,receipt=str(entry/'receipt.json'),
                        seconds=round(time.monotonic()-step_started,3),module_run_seconds=0)
        print(f'RUN {name} {entry}',flush=True)
        artifacts = root/'artifacts'; artifacts.mkdir(exist_ok=True)
        before_dirs = {p.resolve() for p in artifacts.iterdir()}
        temp_root = Path(os.environ.get('TMPDIR','/tmp'))
        before_tmp = {p.resolve() for p in temp_root.iterdir()}
        llbc = {p:p.read_bytes() for p in (root/'verification/aeneas/llbc').glob('*.llbc')}
        started = time.monotonic()
        log = entry/'command.log'
        # Remove an old success before execution so a killed rerun stays failed.
        (entry/'receipt.json').unlink(missing_ok=True)
        with log.open('wb') as stream:
            code = subprocess.run(command,cwd=root,stdout=stream,stderr=subprocess.STDOUT).returncode
        sys.stdout.write(log.read_text())
        if code:
            raise subprocess.CalledProcessError(code,command)
        # Preserve raw emissions; normalize only the already-reviewed unordered
        # short_names map. No AST field or ordered sequence is discarded.
        for path,original in llbc.items():
            if path.read_bytes()!=original:
                if llbc_digest(json.loads(original))!=llbc_digest(json.loads(path.read_bytes())):
                    raise ValueError('ordered LLBC changed: '+str(path))
                saved=entry/'raw-generated'/path.name;saved.parent.mkdir(exist_ok=True)
                saved.write_bytes(path.read_bytes());path.write_bytes(original)
        if module_key(name,command,root)!=key:
            raise ValueError('module inputs changed during acceptance: '+name)
        evidence={}
        for directory in [p for p in artifacts.iterdir() if p.resolve() not in before_dirs]:
            evidence.update(evidence_files(directory))
        for directory in [p for p in temp_root.iterdir() if p.resolve() not in before_tmp and p.name.startswith('sandproof-')]:
            evidence.update(evidence_files(directory))
        files={'command.log':sha(log)}
        files.update({str(p.relative_to(entry)):sha(p) for p in (entry/'raw-generated').glob('*')})
        for relative in FIXED_EVIDENCE.get(name, ()):
            path=Path(relative)
            if path.is_absolute() or '..' in path.parts:
                raise ValueError('fixed evidence must be project-relative')
            source=root/path
            if not source.is_file() or source.is_symlink():
                raise ValueError('missing fixed raw evidence: '+relative)
            expected=sha(source)
            target=entry/'fixed-output'/path;target.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(source,target)
            if sha(target)!=expected or sha(source)!=expected:
                raise ValueError('fixed raw evidence changed while preserving it')
            files[str(target.relative_to(entry))]=expected
        receipt=dict(complete=True,exit_code=0,key=key,files=files,evidence=evidence,
                     batch=os.environ.get('SANDPROOF_PROOF_BATCH'),
                     seconds=round(time.monotonic()-started,3))
        write_json(entry/'receipt.json',receipt)
        return dict(module=name,reused=False,receipt=str(entry/'receipt.json'),
                    seconds=round(time.monotonic()-step_started,3),module_run_seconds=receipt['seconds'])


def fixed_runtime_sha(path, cached, new_hashes, allow_metadata=False):
    path=Path(path).resolve(strict=True)
    started=time.time_ns()
    with path.open('rb') as stream:
        stat=os.fstat(stream.fileno())
        identity=[stat.st_dev,stat.st_ino,stat.st_size,stat.st_mtime_ns,stat.st_ctime_ns]
        prior=cached.get('files',{}).get(str(path),{})
        if not isinstance(prior,dict):prior={}
        value=prior.get('sha256')
        # Linux local-filesystem timestamps can share one clock tick. Like a
        # racy Git index entry, a digest taken near the last write must be read
        # again even when size/mtime/ctime all compare equal. A clock rollback
        # also forces a content read. Only old, quiescent fixed tools use this.
        verified=prior.get('verified_started_ns')
        quiescent=(allow_metadata and type(verified) is int and started>=verified
                   and verified-stat.st_ctime_ns>=2_000_000_000
                   and verified-stat.st_mtime_ns>=2_000_000_000)
        if (not quiescent or prior.get('identity')!=identity or not isinstance(value,str)
                or re.fullmatch('[0-9a-f]{64}',value) is None):
            import hashlib
            value=hashlib.file_digest(stream,'sha256').hexdigest()
        after=os.fstat(stream.fileno())
        if identity!=[after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns]:
            raise ValueError('fixed runtime file changed while fingerprinting: '+str(path))
    new_hashes[str(path)]=dict(identity=identity,sha256=value,verified_started_ns=started)
    return value

def runtime(force=False):
    """Hash actual tool binaries, installed Lean library and all package files.

    Once per batch, again at completion; a child cannot introduce its own
    environment stamp. Kept outside .lake; no compiler or cache is changed.
    """
    cache_file=Path(os.environ['SANDPROOF_FORMAL_CACHE'])/'runtime-file-hashes.json'
    try:
        cached=json.loads(cache_file.read_text())
        if not isinstance(cached,dict) or cached.get('schema')!=1 or not isinstance(cached.get('files'),dict):
            cached={'schema':1,'files':{}}
    except (OSError,ValueError,TypeError):cached={'schema':1,'files':{}}
    if force:cached={'schema':1,'files':{}}
    new_hashes={}
    filesystems={}
    def fixed_sha(path):
        path=Path(path).resolve(strict=True)
        device=path.stat().st_dev
        if device not in filesystems:
            probe=subprocess.run(['stat','-f','-c','%T',str(path)],capture_output=True,text=True)
            filesystems[device]=(probe.returncode==0 and probe.stdout.strip() in ('ext2/ext3','ext4'))
        return fixed_runtime_sha(path,cached,new_hashes,allow_metadata=filesystems[device])
    result={'tools':{},'libraries':{},'environment':{k:v for k,v in os.environ.items()
        if k.startswith(('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_PROFILE_',
                         'CARGO_BUILD_TARGET','LEAN_','LAKE_','OCAMLRUNPARAM',
                         'CC','CXX','AR','LD','CFLAGS','CPPFLAGS','PKG_CONFIG','SQLITE3','LIBSQLITE3'))
        or k in ('RUSTC','RUSTDOC','CARGO_BUILD_RUSTC')}}
    for name,args in [('rustc',['-vV']),('cargo',['-V']),('lean',['--version']),('lake',['--version'])]:
        binary=shutil.which(name)
        if name in ('rustc','cargo'):
            binary=subprocess.check_output(['rustup','which',name],text=True).strip()
        else:
            binary=str(Path(subprocess.check_output(['lean','--print-prefix'],text=True).strip())/'bin'/name)
        result['tools'][name]=dict(sha256=fixed_sha(binary),version=subprocess.check_output([binary,*args],text=True))
    tools=Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT',str(ROOT/'.tools')))
    for name in ('aeneas/bin/aeneas','aeneas/charon/bin/charon','aeneas/charon/bin/charon-driver'):
        result['tools'][name]=fixed_sha(tools/name)
    service=ROOT/'.tools/lean-mcp'
    result['tools']['service-python']=fixed_sha(service/'bin/python')
    result['libraries']['service']=digest({str(p.relative_to(service)):fixed_sha(p)
        for p in (service/'lib').rglob('*') if p.is_file() and p.suffix in ('.py','.so','.pth')})
    # Both the frontend and its full Rust sysroot affect extraction.
    frontend=Path(subprocess.check_output([str(tools/'aeneas/charon/bin/charon'),'toolchain-path'],text=True).strip())
    prefix=Path(subprocess.check_output(['lean','--print-prefix'],text=True).strip())
    cargo_home=Path(os.environ.get('CARGO_HOME',str(Path.home()/'.cargo')))
    result['libraries']['cargo-registry']=digest({str(p.relative_to(cargo_home/'registry/src')):fixed_sha(p)
        for p in (cargo_home/'registry/src').rglob('*') if p.is_file()})
    result['libraries']['cargo-config']={str(p):sha(p) for p in
        (cargo_home/'config',cargo_home/'config.toml') if p.is_file()}
    for label,directory in [('lean-lib',prefix/'lib'),('frontend',frontend),('packages',ROOT/'.lake/packages')]:
        tree_started=time.monotonic()
        # Follow the already admitted package links; detect linked package file
        # edits too, rather than trusting only their Git commit or version text.
        entries={}
        for parent,dirs,files in os.walk(directory,followlinks=True):
            dirs[:]=[d for d in dirs if d not in ('.git','target','artifacts','_build','node_modules')]
            if label == 'frontend' and Path(parent)==directory:
                # Compiler, Rust libraries/sources and target configuration are
                # inputs. Installed HTML/man pages are not used by extraction.
                dirs[:]=[d for d in dirs if d!='share']
            if label == 'packages':
                dirs[:]=[d for d in dirs if d!='.lake']
            for name in files:
                path=Path(parent)/name
                if label == 'packages':
                    if '.lake' in path.relative_to(directory).parts or path.suffix not in ('.lean','.toml','.json','.c','.h'):
                        continue
                if path.is_file(): entries[str(path.relative_to(directory))]=fixed_sha(path)
        result['libraries'][label]=digest(entries)
        print(json.dumps(dict(runtime_tree=label,files=len(entries),seconds=round(time.monotonic()-tree_started,3))),flush=True)
    write_json(cache_file,dict(schema=1,files=new_hashes))
    return result


def expected_audits(root=ROOT):
    paths = [root/'verification/ProofAudit.lean']
    paths += [p for directory in ('SandboxModel', 'verification/aeneas')
              for p in (root/directory).rglob('*.lean')]
    paths += [root/name for name in EXTRA_AUDIT_FILES]
    return {name for path in paths
            for name in re.findall(r'^#audit_controller_proof (\S+)$', path.read_text(), re.M)}


def run_batch(cache, recompute=False):
    started=time.monotonic()
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')!='formal':
        raise ValueError('formal role required; ordinary tests belong on ning')
    native=Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])
    validate_native(native)
    from tools.acceptance_receipts import native_case
    native_case('functional',ROOT,None)
    cache.mkdir(parents=True,exist_ok=True)
    os.environ['SANDPROOF_FORMAL_CACHE']=str(cache)
    # Serialize the shared checkout as well as cache publication. Different
    # branches need separate worktrees/.lake/build, not one writable directory.
    with locked(cache/'checkout-locks'/digest(str(ROOT.resolve()))):
        inputs_before=tracked_inputs()
        native_before=sha(native/'complete.json')
        from tools.check_first_chain_extraction import check_tools
        check_tools(ROOT,Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT',str(ROOT/'.tools'))))
        profile=runtime(force=recompute);stamp=cache/('runtime-'+digest(profile)+'.json');write_json(stamp,profile)
        os.environ['SANDPROOF_FORMAL_RUNTIME']=str(stamp)
        os.environ['SANDPROOF_FORMAL_CACHE']=str(cache)
        os.environ['SANDPROOF_PROOF_BATCH']=str(time.time_ns())+'-'+str(os.getpid())
        if recompute: os.environ['SANDPROOF_PROOF_RECOMPUTE']='1'
        results=[]
        # Admission is always current, even when every module result is reused.
        for manifest in sorted((ROOT/'verification').glob('*inputs.sha256')):
            subprocess.run(['shasum','-a','256','-c',str(manifest)],cwd=ROOT,check=True)
        subprocess.run(['bash','scripts/verify-use-inventory.sh'],cwd=ROOT,check=True)
        subprocess.run([sys.executable,'tools/v0_use_inventory.py','check-published',
                        os.environ['SANDPROOF_USE_CAPTURE']],cwd=ROOT,check=True)
        subprocess.run([sys.executable,'tools/check_json_request_binding.py'],cwd=ROOT,check=True)
        for name in MODULES:
            results.append(step(name,['bash',f'scripts/verify-{name}.sh']))
            if tracked_inputs()!=inputs_before:
                raise ValueError('tracked inputs changed between modules')
        results.append(step('retained',['bash','scripts/verify-retained-proofs.sh']))
        expected=expected_audits()
        observed=set()
        for row in results:
            text=(Path(row['receipt']).parent/'command.log').read_text()
            observed.update(re.findall(r'Checked ([\w.]+): \d+ standard logical dependencies',text))
        if expected != observed:
            raise ValueError('dependency audit inventory mismatch: '+repr(dict(
                missing=sorted(expected-observed), additional=sorted(observed-expected))))
        if tracked_inputs()!=inputs_before or sha(native/'complete.json')!=native_before:
            raise ValueError('batch source or native evidence changed')
        validate_native(native)
        if runtime()!=profile:
            raise ValueError('formal runtime changed during the batch')
        write_json(cache/'latest-batch.json',dict(complete=True,exit_code=0,results=results,
            seconds=round(time.monotonic()-started,3),runtime_sha256=digest(profile),
            audits=dict(expected=len(expected),observed=len(observed),missing=sorted(expected-observed),additional=sorted(observed-expected)),
            native_receipt_sha256=sha(native/'complete.json')))
        print(json.dumps(dict(complete=True,results=results,seconds=round(time.monotonic()-started,3))))


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='mode',required=True)
    run=sub.add_parser('run');run.add_argument('--cache',type=Path,required=True);run.add_argument('--recompute',action='store_true')
    task=sub.add_parser('step');task.add_argument('name',choices=['foundations-generation','foundations']);task.add_argument('command',nargs=argparse.REMAINDER)
    args=parser.parse_args(argv)
    if args.mode=='run': run_batch(args.cache.resolve(),args.recompute)
    else:
        command=args.command
        if command and command[0]=='--': command=command[1:]
        if not command: parser.error('step requires a command')
        step(args.name,command)


if __name__=='__main__':
    main()
