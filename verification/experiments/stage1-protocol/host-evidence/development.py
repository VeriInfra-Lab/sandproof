"""Reproduce the unadopted host-evidence candidate; never issue an acceptance seal."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/host-evidence')
GEN = Path('verification/aeneas/stage1-protocol/host-evidence/SandproofHostEvidence')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


base = module('host_evidence_sources', ROOT/HERE.parent/'schema-leaves/formal.py')
checker = module('host_evidence_checker', ROOT/HERE.parent/'formal.py')
resolver = module('host_evidence_closure', ROOT/'tools/proof_batch.py')
runtime = module('host_evidence_runtime', ROOT/HERE.parent/'formal_runtime.py')
sha = base.sha


def wrapper(project):
    paths = [Path.home(), runtime.tool_root(ROOT), runtime.package_root(ROOT)]
    for key in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
        if os.environ.get(key):
            seed = Path(os.environ[key]).resolve(strict=True)
            if not seed.is_dir():
                raise ValueError('missing fixed runtime: '+key)
            paths.append(seed)
    return ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
        '--tmpfs', '/tmp', '--tmpfs', '/home',
        *[arg for p in dict.fromkeys(paths) for arg in ('--ro-bind', str(p), str(p))],
        '--bind', str(project), str(runtime.VIRTUAL), '--chdir', str(runtime.VIRTUAL),
        '--setenv', 'CARGO_TARGET_DIR', str(runtime.VIRTUAL/'artifacts/extract-target'),
        '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
        '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']


def type_schema(decl):
    """Keep every business field, its type and each variant's actual tag/order."""
    kind = decl['kind']
    def field(value):
        return dict(name=value['name'], ty=value['ty'])
    if isinstance(kind, dict) and 'Struct' in kind:
        return dict(name=decl['item_meta']['name'], struct=[field(f) for f in kind['Struct']])
    if isinstance(kind, dict) and 'Enum' in kind:
        return dict(name=decl['item_meta']['name'], enum=[dict(name=v['name'],
            discriminant=v['discriminant'], fields=[field(f) for f in v['fields']])
            for v in kind['Enum']])
    raise ValueError('business type became opaque or lost its fields')


def admit(raw, translated, inventory, variant):
    if raw.get('has_errors') is not False:
        raise ValueError('incomplete original IR')
    data = raw['translated']
    if data['options'] != inventory['capture_options']:
        raise ValueError('unreviewed capture options')
    if [t['key'] for t in data['target_information']] != ['x86_64-unknown-linux-gnu']:
        raise ValueError('incorrect original target')
    functions = sorted((f['lean_name'], f['is_opaque'], f['lean_file'])
        for f in translated['functions'])
    if functions != [tuple(f) for f in inventory['generated_functions'][variant]]:
        raise ValueError('changed or missing generated method/model closure')
    for schema in inventory['business_schemas']:
        matches = [t for t in data['type_decls'] if t and t['item_meta']['name'] == schema['name']]
        if len(matches) != 1 or type_schema(matches[0]) != schema:
            raise ValueError('incomplete business type: '+str(schema['name']))
    for name in inventory['transparent_functions']:
        matches = [f for f in data['fun_decls'] if f and f['item_meta']['name'] == name]
        if (len(matches) != 1 or matches[0]['item_meta']['opacity'] != 'Transparent'
                or not isinstance(matches[0]['body'], dict) or 'Structured' not in matches[0]['body']):
            raise ValueError('actual method/derived comparison is missing or opaque: '+str(name))


def run(output, variant, prepare_only):
    output.mkdir(parents=True, mode=0o700)
    rec = dict(kind='host-evidence-candidate-development', variant=variant, prepared=False,
        complete=False, source_candidate_adopted=False, original_rust_correspondence=False,
        formal_acceptance=False, ning_acceptance=False, shared_acceptance=False, full_v3=False,
        production_modified=False, commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    started = time.monotonic()

    def command(cwd, folder, label, argv, extra=None):
        log = folder/(label+'.log'); begin = time.monotonic()
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=cwd, env=dict(env, **(extra or {})),
                stdin=subprocess.DEVNULL, stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(argv=argv, cwd=str(cwd), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec, indent=2)+'\n')
        if result.returncode:
            raise ValueError('failed '+str(log))

    try:
        inv = json.loads((ROOT/HERE/'inventory.json').read_text())
        for name, digest in inv['production_inputs'].items():
            if sha(ROOT/name) != digest:
                raise ValueError('original source/provider changed: '+name)
        for name, digest in inv['providers'].items():
            if sha(ROOT/HERE/name) != digest:
                raise ValueError('reviewed proof/model/patch changed: '+name)
        rec['inputs'] = base.snapshot(ROOT)
        rec['input_modes'] = {n:('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') for n in rec['inputs']}
        rec['source_commit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        std = module('host_evidence_existing_library', ROOT/HERE.parent/'inbox-config-validation/library_binding.py')
        rec['fixed_std_sources'] = std.check(ROOT)
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool differs: '+name)
        rec['tools'] = fixed
        project = output/'source'; project.mkdir()
        expected = dict(rec['inputs'])
        for name, digest in rec['inputs'].items():
            dest = project/name; dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, dest)
            if sha(dest) != digest:
                raise ValueError('input changed: '+name)
        (project/'.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir(); (project/'.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        ceiling = dict(GIT_CEILING_DIRECTORIES=str(project.parent))
        # Apply only to the private copy. Neither candidate changes the public tree.
        for label in ('source-guard', 'shared-vector-comparison'):
            patch = project/HERE/(label+'-candidate.patch')
            command(project, output, label+'-check', ['git', 'apply', '--unidiff-zero', '--check', str(patch)], ceiling)
            command(project, output, label+'-apply', ['git', 'apply', '--unidiff-zero', str(patch)], ceiling)
        for name, digest in inv['candidate_inputs'].items():
            if sha(project/name) != digest:
                raise ValueError('candidate patch did not reproduce reviewed bytes: '+name)
            expected[name] = digest
        node = project/'sandbox-node/src/lib.rs'
        if 'pub mod host_evidence;' in node.read_text():
            raise ValueError('already instrumented source')
        node.write_text(node.read_text().rstrip()+'\n\n#[path = "../../verification/experiments/stage1-protocol/host-evidence/roots.rs.in"]\npub mod host_evidence;\n')
        expected['sandbox-node/src/lib.rs'] = sha(node)
        if variant != 'correct':
            mutation = inv['mutations'][variant]; path = project/mutation['source']; text = path.read_text()
            if text.count(mutation['old']) != 1:
                raise ValueError('ambiguous actual Rust mutation')
            path.write_text(text.replace(mutation['old'], mutation['new'], 1))
            expected[mutation['source']] = sha(path)
            rec['mutation'] = mutation
        args = [str(runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
        for name in inv['include']: args += ['--include', name]
        for name in inv['roots']: args += ['--start-from', name]
        args += ['--dest-file', str(runtime.VIRTUAL/'artifacts/host-evidence.llbc'), '--',
            '--manifest-path', inv['manifest'], '--lib', '--locked', '--offline', '--target', fixed['target']]
        command(project, output, 'charon', wrapper(project)+args)
        raw = json.loads((project/'artifacts/host-evidence.llbc').read_text())
        generated = output/'generated'; generated.mkdir()
        command(project, output, 'aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', inv['generated_namespace'], '-subdir', inv['generated_subdir'],
            '-dest', str(generated), str(project/'artifacts/host-evidence.llbc')])
        admit(raw, json.loads((generated/'translation.json').read_text()), inv, variant)
        actual_generated = {str(p.relative_to(generated)):sha(p) for p in generated.rglob('*.lean')}
        if actual_generated != inv['generated_files'][variant]:
            raise ValueError('fresh generated files differ from reviewed original outputs')
        rec['generated'] = actual_generated
        command(project, output, 'admission-regressions', [sys.executable, str(HERE/'admission_tests.py'),
            '--llbc', str(project/'artifacts/host-evidence.llbc'), '--generated', str(generated), '--variant', variant])
        (project/GEN).mkdir(parents=True)
        for name in ('Types.lean', 'Funs.lean'):
            shutil.copy2(generated/inv['generated_subdir']/name, project/GEN/name)
        for name in ('FunsExternal', 'TypesExternal'):
            shutil.copy2(ROOT/HERE/(name+'.lean.in'), project/GEN/(name+'.lean'))
        for name in ('Types', 'Funs', 'FunsExternal', 'TypesExternal'):
            expected[str(GEN/(name+'.lean'))] = sha(project/GEN/(name+'.lean'))
        lake = project/'lakefile.toml'
        lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofHostEvidence"\nsrcDir = "verification/aeneas/stage1-protocol/host-evidence"\n')
        expected['lakefile.toml'] = sha(lake)
        stems = ['HostBootModel', 'HostBootFacts', 'HostEvidenceTypes', 'HostEvidenceModel', 'HostEvidenceValues']
        if variant == 'correct':
            stems += ['HostBootCandidate', 'HostCommandCandidate', 'HostEvidenceConnection',
                'HostEvidenceWitnesses', 'HostEvidenceBoundaries']
        else:
            stems += ['HostEvidenceWrongCommand' if variant == 'skip-command' else 'HostEvidenceWrongResource']
        for stem in stems:
            name = 'SandboxModel/Lifecycle/Protocol/'+stem+'.lean'
            shutil.copy2(ROOT/HERE/(stem+'.lean.in'), project/name); expected[name] = sha(project/name)
        if variant == 'skip-resource':
            # The unchanged helper is checked against THIS mutant's own Funs/cache.
            name = 'SandboxModel/Lifecycle/Protocol/HostProcessFields.lean'
            shutil.copy2(ROOT/HERE/'HostBootCandidate.lean.in', project/name); expected[name] = sha(project/name)
        seeds = inv['checks'][variant]['seeds']
        files = sorted(resolver.closure(project, ['SandboxModel/Lifecycle/Protocol/'+s+'.lean' for s in seeds], active_view='lakefile.toml'))
        audits = [a for n in files for a in re.findall(r'^#audit_controller_proof (\S+)$', (project/n).read_text(), re.M)]
        if (len(files) != inv['checks'][variant]['files'] or len(audits) != inv['checks'][variant]['audits']
                or len(set(audits)) != len(audits)):
            raise ValueError('incomplete reviewed actual-file/audit closure')
        if variant != 'correct' and any(any(s in n for s in ['HostEvidenceConnection', 'HostEvidenceWitnesses',
                'HostEvidenceBoundaries', 'HostCommandCandidate', 'HostBootCandidate']) for n in files):
            raise ValueError('mutant must not import the correct implementation proof')
        rec.update(prepared=True, files={n:sha(project/n) for n in files}, audits=audits)
        if not prepare_only:
            command(project, output, 'build', ['lake', 'build', *['SandboxModel.Lifecycle.Protocol.'+s for s in seeds]])
            physical = output/'actual-files'; physical.mkdir()
            rec['actual'] = checker.actual_files(project, files, physical, env, command)
            if rec['actual']['files'] != rec['files'] or sorted(rec['actual']['audits']) != sorted(audits):
                raise ValueError('original checker closure does not match prepared inputs')
            rec['complete'] = True
        if any(sha(project/n) != h for n,h in expected.items()):
            raise ValueError('private inputs changed during checking')
        if base.snapshot(ROOT) != rec['inputs']:
            raise ValueError('original source changed during checking')
        if any(('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') != mode
                for n,mode in rec['input_modes'].items()):
            raise ValueError('original source executable mode changed during checking')
    except BaseException as error:
        rec.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    rec['seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')
    return rec


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=('correct', 'skip-command', 'skip-resource'), default='correct')
    parser.add_argument('--prepare-only', action='store_true', help='capture and assemble, without claiming proof completion')
    parser.add_argument('--lock', type=Path, required=True, help='the existing shared proof-heavy lock')
    args = parser.parse_args(); os.umask(0o077)
    with args.lock.open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        result = run(args.output.absolute(), args.variant, args.prepare_only)
    print(json.dumps({k:result.get(k) for k in ('prepared', 'complete', 'seconds', 'error')}))
    raise SystemExit(not (result['complete'] or (args.prepare_only and result['prepared'] and 'error' not in result)))
