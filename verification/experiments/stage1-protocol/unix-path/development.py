"""Reproduce transparent Unix path values and their ready-field connection; no acceptance seals."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/unix-path')
READY = HERE.parent/'ready-candidate'
PATH_GEN = Path('verification/aeneas/stage1-protocol/unix-path/SandproofUnixPath')
READY_GEN = Path('verification/aeneas/stage1-protocol/ready-candidate/SandproofReadyCandidate')
SNAPSHOT = 'controller-service/src/controller/lifecycle_inbox/readiness/snapshot.rs'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec); spec.loader.exec_module(value)
    return value


base = module('unix_path_original_source', ROOT/HERE.parent/'schema-leaves/formal.py')
runtime = module('unix_path_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
checker = module('unix_path_original_checker', ROOT/HERE.parent/'formal.py')
resolver = module('unix_path_original_closure', ROOT/'tools/proof_batch.py')
sha = base.sha


def wrapper(project):
    paths = [Path.home(), runtime.tool_root(ROOT), runtime.package_root(ROOT)]
    for key in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
        if os.environ.get(key): paths.append(Path(os.environ[key]).resolve(strict=True))
    return ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
        '--tmpfs', '/tmp', '--tmpfs', '/home',
        *[a for p in dict.fromkeys(paths) for a in ('--ro-bind', str(p), str(p))],
        '--bind', str(project), str(runtime.VIRTUAL), '--chdir', str(runtime.VIRTUAL),
        '--setenv', 'CARGO_TARGET_DIR', str(runtime.VIRTUAL/'artifacts/extract-target'),
        '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
        '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']


def run(output, variant):
    output.mkdir(parents=True, mode=0o700); started = time.monotonic()
    rec = dict(kind='transparent-unix-path-independent-development', variant=variant,
        complete=False, commands=[], new_path_model_assumption=False, production_modified=False,
        formal_acceptance=False, ning_acceptance=False, shared_acceptance=False, full_v3=False,
        btree_carrier_proposal_still_unadopted=(variant != 'path'))
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')

    def command(cwd, folder, label, argv):
        log = folder/(label+'.log'); begin = time.monotonic()
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(argv=argv, cwd=str(cwd), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec, indent=2)+'\n')
        if result.returncode: raise ValueError('failed '+str(log))

    try:
        inv = json.loads((ROOT/HERE/'inventory.json').read_text())
        for name, digest in inv['source_inputs'].items():
            if sha(ROOT/name) != digest: raise ValueError('original input differs: '+name)
        for name, digest in inv['templates'].items():
            if sha(ROOT/HERE/name) != digest: raise ValueError('proof/root differs: '+name)
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool differs: '+name)
        sources = {}
        for kind, selector in [('production', []), ('extraction', ['+'+fixed['extraction_frontend']])]:
            sysroot = Path(subprocess.check_output(['rustc', *selector, '--print', 'sysroot'], text=True).strip())
            sources[kind] = {}
            for name, digest in inv['std_sources'][kind].items():
                path = sysroot/'lib/rustlib/src/rust/library'/name
                if sha(path) != digest: raise ValueError('fixed std source differs: '+kind+'/'+name)
                sources[kind][name] = dict(path=str(path), sha256=digest)
        rec['fixed_std_sources'] = sources; rec['tools'] = fixed
        rec['inputs'] = base.snapshot(ROOT)
        rec['input_modes'] = {n:('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') for n in rec['inputs']}
        rec['source_commit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        project = output/'source'; project.mkdir(); expected = dict(rec['inputs'])
        for name, digest in rec['inputs'].items():
            dest = project/name; dest.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(ROOT/name, dest)
            if sha(dest) != digest: raise ValueError('input changed during copy: '+name)
        (project/'.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir(); (project/'.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)

        def capture(label, recipe, namespace, subdir):
            folder = output/label; folder.mkdir(); ir = project/'artifacts'/(label+'.llbc')
            args = [str(runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                '--error-on-warnings', '--sysroot', 'default']
            for name in recipe['include']: args += ['--include', name]
            for name in recipe['roots']: args += ['--start-from', name]
            args += ['--dest-file', str(runtime.VIRTUAL/'artifacts'/(label+'.llbc')), '--',
                '--manifest-path', recipe['manifest'], '--lib', '--locked', '--offline', '--target', fixed['target']]
            command(project, folder, 'charon', wrapper(project)+args)
            raw = json.loads(ir.read_text()); options = raw['translated']['options']
            if (raw.get('has_errors') is not False or options['start_from'] != recipe['roots']
                    or options['include'] != recipe['include'] or options['opaque'] or options['exclude']
                    or options['skip_borrowck'] or options['no_typecheck'] or options['monomorphize']
                    or options['remove_unused_clauses']):
                raise ValueError('unreviewed capture: '+label)
            generated = folder/'generated'; generated.mkdir()
            command(project, folder, 'aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
                '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', namespace,
                '-subdir', subdir, '-dest', str(generated), str(ir)])
            functions = json.loads((generated/'translation.json').read_text())['functions']
            key = 'path' if label == 'path' else variant
            if sorted((f['lean_name'], f['is_opaque'], f['lean_file']) for f in functions) != [tuple(f) for f in inv['generated_functions'][key]]:
                raise ValueError('changed generated method/model closure: '+label)
            if {str(p.relative_to(generated)):sha(p) for p in generated.rglob('*.lean')} != inv['generated_files'][key]:
                raise ValueError('fresh raw generation differs: '+label)
            return generated

        generated = capture('path', inv['path_capture'], 'SandproofUnixPathRust', 'SandproofUnixPath')
        (project/PATH_GEN).mkdir(parents=True)
        for name in ('Types.lean', 'Funs.lean'):
            shutil.copy2(generated/'SandproofUnixPath'/name, project/PATH_GEN/name)
            expected[str(PATH_GEN/name)] = sha(project/PATH_GEN/name)
        proof = project/'SandboxModel/Lifecycle/Protocol'
        shutil.copy2(ROOT/HERE/'UnixPathValues.lean.in', proof/'UnixPathValues.lean')
        expected[str((proof/'UnixPathValues.lean').relative_to(project))] = sha(proof/'UnixPathValues.lean')
        lake = project/'lakefile.toml'
        lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofUnixPath"\nsrcDir = "verification/aeneas/stage1-protocol/unix-path"\n')
        seeds = ['UnixPathValues']
        if variant != 'path':
            providers = {}
            for name, digest in inv['ready_providers'].items():
                value = subprocess.check_output(['git', 'show', inv['ready_provider_commit']+':'+str(READY/name)], cwd=ROOT)
                import hashlib
                if hashlib.sha256(value).hexdigest() != digest: raise ValueError('pinned mathematical provider differs: '+name)
                providers[name] = value
            rec['ready_providers'] = dict(commit=inv['ready_provider_commit'], files=inv['ready_providers'])
            source = project/SNAPSHOT; text = source.read_text()
            if 'proof_ready_candidate_can_probe' in text: raise ValueError('already instrumented source')
            if variant == 'ignore-credential':
                mutation = inv['mutation']
                if text.count(mutation['old']) != 1: raise ValueError('ambiguous actual mutation')
                text = text.replace(mutation['old'], mutation['new'], 1); rec['mutation'] = mutation
            source.write_text(text.rstrip()+'\n'+providers['roots.rs.in'].decode())
            command(project, output, 'ready-rustfmt', ['rustfmt', '--edition', '2024', '--config', 'skip_children=true', SNAPSHOT])
            expected[SNAPSHOT] = sha(source)
            generated = capture('ready', inv['ready_capture'], 'SandproofReadyCandidateRust', 'SandproofReadyCandidate')
            (project/READY_GEN).mkdir(parents=True)
            for name in ('Types.lean', 'Funs.lean'):
                shutil.copy2(generated/'SandproofReadyCandidate'/name, project/READY_GEN/name)
                expected[str(READY_GEN/name)] = sha(project/READY_GEN/name)
            (project/READY_GEN/'FunsExternal.lean').write_bytes(providers['FunsExternal.lean.in'])
            shutil.copy2(ROOT/HERE/'ReadyTypesExternal.candidate.lean.in', project/READY_GEN/'TypesExternal.lean')
            for name in ('FunsExternal.lean', 'TypesExternal.lean'): expected[str(READY_GEN/name)] = sha(project/READY_GEN/name)
            stems = ['ReadyCandidateModel', 'ReadyCandidateValues']
            stems += ['ReadyCandidateProofs', 'ReadyCandidateWitnesses'] if variant == 'correct' else ['ReadyCandidateWrongCredential']
            for stem in stems:
                name = 'SandboxModel/Lifecycle/Protocol/'+stem+'.lean'
                (project/name).write_bytes(providers[stem+'.lean.in']); expected[name] = sha(project/name)
            shutil.copy2(ROOT/HERE/'ReadyPathConnection.lean.in', proof/'ReadyPathConnection.lean')
            expected[str((proof/'ReadyPathConnection.lean').relative_to(project))] = sha(proof/'ReadyPathConnection.lean')
            lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofReadyCandidate"\nsrcDir = "verification/aeneas/stage1-protocol/ready-candidate"\n\n[[lean_lib]]\nname = "SandproofStorage"\nsrcDir = "verification/aeneas/storage"\n')
            seeds = ['ReadyCandidateWitnesses' if variant == 'correct' else 'ReadyCandidateWrongCredential', 'ReadyPathConnection']
        expected['lakefile.toml'] = sha(lake)
        files = sorted(resolver.closure(project, ['SandboxModel/Lifecycle/Protocol/'+s+'.lean' for s in seeds], active_view='lakefile.toml'))
        audits = [a for n in files for a in re.findall(r'^#audit_controller_proof (\S+)$', (project/n).read_text(), re.M)]
        if len(files) != inv['checks'][variant]['files'] or len(audits) != inv['checks'][variant]['audits'] or len(set(audits)) != len(audits):
            raise ValueError('incomplete actual-file/audit closure')
        if variant == 'ignore-credential' and any('ReadyCandidateProofs' in n or 'ReadyCandidateWitnesses' in n for n in files):
            raise ValueError('wrong view imports correct readiness contract')
        rec.update(files={n:sha(project/n) for n in files}, audits=audits)
        command(project, output, 'build', ['lake', 'build', *['SandboxModel.Lifecycle.Protocol.'+s for s in seeds]])
        physical = output/'actual-files'; physical.mkdir()
        rec['actual'] = checker.actual_files(project, files, physical, env, command)
        if rec['actual']['files'] != rec['files'] or sorted(rec['actual']['audits']) != sorted(audits):
            raise ValueError('strict checker closure differs')
        if any(sha(project/n) != h for n,h in expected.items()) or base.snapshot(ROOT) != rec['inputs']:
            raise ValueError('source changed during checks')
        if any(('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') != m for n,m in rec['input_modes'].items()):
            raise ValueError('original executable mode changed')
        if any(sha(Path(row['path'])) != row['sha256'] for group in sources.values() for row in group.values()):
            raise ValueError('fixed std source changed during checks')
        rec['complete'] = True
    except BaseException as error:
        rec.update(error=repr(error), traceback=traceback.format_exc())
    rec['seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')
    return rec


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant', choices=('path', 'correct', 'ignore-credential'), default='path')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True, help='existing shared proof-heavy lock')
    args = parser.parse_args(); os.umask(0o077)
    with args.lock.open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX); result = run(args.output.absolute(), args.variant)
    print(json.dumps({k:result.get(k) for k in ('complete', 'seconds', 'error')}))
    raise SystemExit(not result['complete'])
