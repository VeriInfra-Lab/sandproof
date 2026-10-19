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
           'revocation-contract','tunnel','data-drain','sqlite','foundations','network',
           'first-chain-linux','first-chain','storage')


def file_digest(path):
    if path.suffix == '.llbc':
        return llbc_digest(json.loads(path.read_text()))
    return sha(path)


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
                if not line.startswith('import '):
                    continue
                for module in line[7:].split('--', 1)[0].split():
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
        seeds = [f'verification/{manifest}-inputs.sha256']
    # Full Rust closure is deliberately conservative. A Rust change can
    # invalidate extra modules, but can never silently miss an indirect use.
    return dict(schema=1, module=name, command=command, rust=rust_inputs(root),
                rules=rules(root), inputs=closure(root,seeds),
                runtime=json.loads(Path(os.environ['SANDPROOF_FORMAL_RUNTIME']).read_text()),
                native=json.loads((Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])/'complete.json').read_text()))


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
            return dict(module=name,reused=True,receipt=str(entry/'receipt.json'),seconds=0)
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
        receipt=dict(complete=True,exit_code=0,key=key,files=files,evidence=evidence,
                     batch=os.environ.get('SANDPROOF_PROOF_BATCH'),
                     seconds=round(time.monotonic()-started,3))
        write_json(entry/'receipt.json',receipt)
        return dict(module=name,reused=False,receipt=str(entry/'receipt.json'),seconds=receipt['seconds'])


def runtime():
    """Hash actual tool binaries, installed Lean library and all package files.

    Once per batch, again at completion; a child cannot introduce its own
    environment stamp. Kept outside .lake; no compiler or cache is changed.
    """
    result={'tools':{},'libraries':{},'environment':{k:v for k,v in os.environ.items()
        if k.startswith(('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_PROFILE_',
                         'CARGO_BUILD_TARGET','LEAN_','OCAMLRUNPARAM'))}}
    for name,args in [('rustc',['-vV']),('cargo',['-V']),('lean',['--version']),('lake',['--version'])]:
        binary=shutil.which(name)
        if name in ('rustc','cargo'):
            binary=subprocess.check_output(['rustup','which',name],text=True).strip()
        else:
            binary=str(Path(subprocess.check_output(['lean','--print-prefix'],text=True).strip())/'bin'/name)
        result['tools'][name]=dict(sha256=sha(binary),version=subprocess.check_output([binary,*args],text=True))
    tools=Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT',str(ROOT/'.tools')))
    for name in ('aeneas/bin/aeneas','aeneas/charon/bin/charon','aeneas/charon/bin/charon-driver'):
        result['tools'][name]=sha(tools/name)
    # Both the frontend and its full Rust sysroot affect extraction.
    frontend=Path(subprocess.check_output([str(tools/'aeneas/charon/bin/charon'),'toolchain-path'],text=True).strip())
    prefix=Path(subprocess.check_output(['lean','--print-prefix'],text=True).strip())
    cargo_home=Path(os.environ.get('CARGO_HOME',str(Path.home()/'.cargo')))
    result['libraries']['cargo-registry']=tree_digest(cargo_home/'registry/src')
    result['libraries']['cargo-config']={str(p):sha(p) for p in
        (cargo_home/'config',cargo_home/'config.toml') if p.is_file()}
    for label,directory in [('lean-lib',prefix/'lib'),('frontend',frontend),('packages',ROOT/'.lake/packages')]:
        # Follow the already admitted package links; detect linked package file
        # edits too, rather than trusting only their Git commit or version text.
        entries={}
        for parent,dirs,files in os.walk(directory,followlinks=True):
            dirs[:]=[d for d in dirs if d not in ('.git','target','artifacts','_build','node_modules')]
            for name in files:
                path=Path(parent)/name
                if label == 'packages':
                    if '.lake' in path.relative_to(directory).parts or path.suffix not in ('.lean','.toml','.json','.c','.h'):
                        continue
                if path.is_file(): entries[str(path.relative_to(directory))]=sha(path)
        result['libraries'][label]=digest(entries)
    return result


def run_batch(cache, recompute=False):
    started=time.monotonic()
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')!='formal':
        raise ValueError('formal role required; ordinary tests belong on ning')
    native=Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])
    validate_native(native)
    from tools.acceptance_receipts import native_case
    native_case('functional',ROOT,None)
    cache.mkdir(parents=True,exist_ok=True)
    # Serialize the shared checkout as well as cache publication. Different
    # branches need separate worktrees/.lake/build, not one writable directory.
    with locked(cache/'checkout-locks'/digest(str(ROOT.resolve()))):
        profile=runtime();stamp=cache/('runtime-'+digest(profile)+'.json');write_json(stamp,profile)
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
        results.append(step('retained',['bash','scripts/verify-retained-proofs.sh']))
        if runtime()!=profile:
            raise ValueError('formal runtime changed during the batch')
        write_json(cache/'latest-batch.json',dict(complete=True,exit_code=0,results=results,
            seconds=round(time.monotonic()-started,3),runtime_sha256=digest(profile),
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
