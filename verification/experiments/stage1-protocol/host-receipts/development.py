"""Reproduce the independent HostObservation receipt view; never issue an acceptance seal."""
import argparse
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
HERE = Path('verification/experiments/stage1-protocol/host-receipts')
PARENT = HERE.parent/'current-receipts'
GEN = Path('verification/aeneas/stage1-protocol/current-receipt/SandproofReceipt')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


base = module('host_receipt_sources', ROOT/HERE.parent/'schema-leaves/formal.py')
checker = module('host_receipt_checker', ROOT/HERE.parent/'formal.py')
resolver = module('host_receipt_closure', ROOT/'tools/proof_batch.py')
runtime = module('host_receipt_runtime', ROOT/HERE.parent/'formal_runtime.py')
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


def admit(raw, translated, inventory, variant):
    if raw.get('has_errors') is not False:
        raise ValueError('incomplete original IR')
    data = raw['translated']; opt = data['options']
    if (opt['start_from'] != inventory['roots'] or opt['include'] != inventory['include']
            or opt['opaque'] or opt['exclude'] or opt['skip_borrowck'] or opt['no_typecheck']
            or opt['monomorphize'] or opt['remove_unused_clauses']):
        raise ValueError('unreviewed capture options')
    if {t['key'] for t in data['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('incorrect original target')
    functions = translated['functions']
    if sorted((f['lean_name'], f['is_opaque'], f['lean_file']) for f in functions) != [tuple(v) for v in inventory['generated_functions'][variant]]:
        raise ValueError('changed or missing generated method/model closure')
    types = [t for t in data['type_decls'] if t]
    def find(parts):
        wanted = [{'Ident': [s, 0]} for s in parts]
        found = [t for t in types if t['item_meta']['name'] == wanted]
        if len(found) != 1:
            raise ValueError('missing unique complete type: '+str(parts))
        return found[0]['kind']
    host = find(['sandproof_node', 'linux', 'HostObservation'])
    if [f['name'] for f in host['Struct']] != inventory['actual_host_fields']:
        raise ValueError('incomplete original HostObservation')
    result = find(['sandproof_node', 'linux', 'ObservedResult'])
    if [v['name'] for v in result['Enum']] != inventory['actual_result_variants']:
        raise ValueError('incomplete original observation outcomes')
    effect = find(['sandbox_controller_core', 'model', 'EffectId'])
    if [f['name'] for f in effect['Struct']] != inventory['actual_effect_fields']:
        raise ValueError('incomplete original EffectId')
    for schema in inventory['struct_schemas']:
        if [f['name'] for f in find(schema['path'])['Struct']] != schema['fields']:
            raise ValueError('original field order differs: '+str(schema['path']))
    for schema in inventory['enum_schemas']:
        actual = [dict(name=v['name'], discriminant=v['discriminant'],
            fields=[f['name'] for f in v['fields']]) for v in find(schema['path'])['Enum']]
        if actual != schema['variants']:
            raise ValueError('original variant fields/tags differ: '+str(schema['path']))
    business = [f for f in data['fun_decls'] if f and f['item_meta']['name'][:2] ==
        [{'Ident': ['sandproof_node', 0]}, {'Ident': ['linux', 0]}]
        and f['item_meta']['name'][-1] == {'Ident': ['resource', 0]}]
    if (len(business) != 1 or business[0]['item_meta']['opacity'] != 'Transparent'
            or 'Structured' not in business[0]['body']):
        raise ValueError('actual production resource getter is absent/opaque')


def run(output, variant, prepare_only):
    output.mkdir(parents=True, mode=0o700)
    rec = dict(kind='host-receipt-independent-development', variant=variant, prepared=False,
        complete=False, formal_acceptance=False, ning_acceptance=False, full_v3=False,
        shared_model_candidate_adopted=False, production_modified=False, commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    start = time.monotonic()
    def command(cwd, folder, label, argv, extra=None):
        log = folder/(label+'.log'); begin = time.monotonic()
        with log.open('xb') as stream:
            p = subprocess.run(argv, cwd=cwd, env=dict(env, **(extra or {})), stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(argv=argv, cwd=str(cwd), exit_code=p.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec, indent=2)+'\n')
        if p.returncode:
            raise ValueError('failed '+str(log))
    try:
        inv = json.loads((ROOT/HERE/'inventory.json').read_text())
        parent = json.loads((ROOT/PARENT/'inventory.json').read_text())
        for data in (inv, parent):
            for name, digest in data['production_inputs'].items():
                if sha(ROOT/name) != digest:
                    raise ValueError('original source/provider changed: '+name)
        for name, desc in inv['providers'].items():
            if sha(ROOT/HERE/name) != desc['sha256']:
                raise ValueError('proof template changed: '+name)
        for name, desc in parent['providers'].items():
            if sha(ROOT/PARENT/name) != desc['sha256']:
                raise ValueError('parent proof/model changed: '+name)
        if sha(ROOT/HERE/'roots.rs.in') != inv['root_sha256']:
            raise ValueError('original forwarding roots changed')
        rec['inputs'] = base.snapshot(ROOT)
        rec['source_commit'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        std = module('host_receipt_existing_library', ROOT/HERE.parent/'inbox-unload-resolution/library_binding.py')
        rec['fixed_std_sources'] = std.check(ROOT)
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool differs: '+name)
        rec['tools'] = fixed
        project = output/'source'; project.mkdir()
        expected = dict(rec['inputs'])
        for name, digest in rec['inputs'].items():
            p = project/name; p.parent.mkdir(parents=True, exist_ok=True); shutil.copy2(ROOT/name, p)
            if sha(p) != digest: raise ValueError('input changed: '+name)
        (project/'.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir(); (project/'.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        node = project/'sandbox-node/src/lib.rs'
        if 'pub mod receipts;' in node.read_text() or 'pub mod host_receipts;' in node.read_text():
            raise ValueError('already instrumented source')
        node.write_text(node.read_text().rstrip()+'\n\n#[path = "../../verification/extract-foundations/src/receipts.rs"]\npub mod receipts;\n#[path = "../../verification/experiments/stage1-protocol/host-receipts/roots.rs.in"]\npub mod host_receipts;\n')
        expected['sandbox-node/src/lib.rs'] = sha(node)
        if variant == 'owner-only':
            p = project/'sandbox-protocol/src/lib.rs'; text = p.read_text()
            old = 'if self.identity != action.identity {'
            if text.count(old) != 1: raise ValueError('ambiguous original mutation')
            p.write_text(text.replace(old, 'if self.identity.owner != action.identity.owner {', 1))
            expected['sandbox-protocol/src/lib.rs'] = sha(p)
        args = [str(runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
        for name in inv['include']: args += ['--include', name]
        for name in inv['roots']: args += ['--start-from', name]
        args += ['--dest-file', str(runtime.VIRTUAL/'artifacts/host-receipt.llbc'), '--',
            '--manifest-path', inv['manifest'], '--lib', '--locked', '--offline', '--target', fixed['target']]
        command(project, output, 'charon', wrapper(project)+args)
        raw = json.loads((project/'artifacts/host-receipt.llbc').read_text())
        generated = output/'generated'; generated.mkdir()
        command(project, output, 'aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json', '-namespace',
            'SandproofReceiptRust', '-subdir', 'SandproofReceipt', '-dest', str(generated), str(project/'artifacts/host-receipt.llbc')])
        admit(raw, json.loads((generated/'translation.json').read_text()), inv, variant)
        if list(generated.rglob('TypesExternal_Template.lean')): raise ValueError('unreviewed external carrier')
        command(project, output, 'admission-regressions', [sys.executable, str(HERE/'admission_tests.py'),
            '--llbc', str(project/'artifacts/host-receipt.llbc'), '--generated', str(generated),
            '--variant', variant])
        # This patch is deliberately unapplied to the public shared tree.
        ceiling = dict(GIT_CEILING_DIRECTORIES=str(project.parent))
        patch = project/PARENT/'shared-model-candidate.patch'
        command(project, output, 'model-patch-check', ['git', 'apply', '--unidiff-zero', '--check', str(patch)], ceiling)
        command(project, output, 'model-patch', ['git', 'apply', '--unidiff-zero', str(patch)], ceiling)
        for name in ('OutcomeModel', 'Evidence', 'ReceiptModel', 'Receipts'):
            p = 'SandboxModel/Foundations/'+name+'.lean'
            expected[p] = sha(project/p)
        (project/GEN).mkdir(parents=True)
        for name in ('Types.lean', 'Funs.lean'):
            shutil.copy2(generated/'SandproofReceipt'/name, project/GEN/name)
        shutil.copy2(ROOT/PARENT/'FunsExternal.lean.in', project/GEN/'FunsExternal.lean')
        lake = project/'lakefile.toml'
        lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofReceipt"\nsrcDir = "verification/aeneas/stage1-protocol/current-receipt"\n')
        expected['lakefile.toml'] = sha(lake)
        for name in ('Types.lean', 'Funs.lean', 'FunsExternal.lean'):
            expected[str(GEN/name)] = sha(project/GEN/name)
        stems = ['CurrentReceiptDefinitions', 'CurrentReceiptValues']
        if variant == 'correct': stems += ['CurrentReceiptConnection', 'CurrentReceiptWitnesses']
        for stem in stems:
            shutil.copy2(ROOT/PARENT/(stem+'.lean.in'), project/'SandboxModel/Lifecycle/Protocol'/(stem+'.lean'))
            name = 'SandboxModel/Lifecycle/Protocol/'+stem+'.lean'; expected[name] = sha(project/name)
        host = ['HostReceiptDefinitions', 'HostReceiptGetter', 'HostReceiptValues']
        host += ['HostReceiptConnection', 'HostReceiptWitnesses'] if variant == 'correct' else ['HostReceiptWrongOwner']
        for stem in host:
            shutil.copy2(ROOT/HERE/(stem+'.lean.in'), project/'SandboxModel/Lifecycle/Protocol'/(stem+'.lean'))
            name = 'SandboxModel/Lifecycle/Protocol/'+stem+'.lean'; expected[name] = sha(project/name)
        seed = 'HostReceiptWitnesses' if variant == 'correct' else 'HostReceiptWrongOwner'
        files = sorted(resolver.closure(project, ['SandboxModel/Lifecycle/Protocol/'+seed+'.lean'], active_view='lakefile.toml'))
        audits = [a for n in files for a in re.findall(r'^#audit_controller_proof (\S+)$', (project/n).read_text(), re.M)]
        if len(audits) != inv['correct_audits' if variant == 'correct' else 'wrong_audits']:
            raise ValueError('incomplete reviewed audit closure')
        rec.update(prepared=True, files={n:sha(project/n) for n in files}, audits=audits)
        if not prepare_only:
            command(project, output, 'build', ['lake', 'build', 'SandboxModel.Lifecycle.Protocol.'+seed])
            physical = output/'actual-files'; physical.mkdir()
            rec['actual'] = checker.actual_files(project, files, physical, env, command)
            rec['complete'] = True
        if any(sha(project/n) != h for n,h in expected.items()):
            raise ValueError('private original/prepared inputs changed during checking')
        if base.snapshot(ROOT) != rec['inputs']:
            raise ValueError('original source changed during checking')
    except BaseException as error:
        rec.update(error=repr(error), traceback=traceback.format_exc())
    rec['seconds'] = time.monotonic()-start
    (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')
    return rec


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=('correct', 'owner-only'), default='correct')
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args(); os.umask(0o077)
    result = run(args.output.absolute(), args.variant, args.prepare_only)
    print(json.dumps({k:result.get(k) for k in ('prepared', 'complete', 'seconds', 'error')}))
    raise SystemExit(not (result['complete'] or (args.prepare_only and result['prepared'] and 'error' not in result)))
