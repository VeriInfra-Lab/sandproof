"""One actual reply implementation and the existing complete receipt relation."""
from pathlib import Path
import importlib.util
import json
import re
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/record-reply')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
GEN = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
PUBLISHED = Path('verification/aeneas/stage1-protocol/record-reply')
MODEL = 'sandbox-internal/src/server/lifecycle.rs'
CASES = ('actual', 'skip-match', 'clear-terminal')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('record_reply_original_snapshot', ROOT/HERE.parent/'schema-leaves/formal.py')
checker = module('record_reply_original_files', ROOT/HERE.parent/'formal.py')
resolver = module('record_reply_original_closure', ROOT/'tools/proof_batch.py')
runtime = module('record_reply_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
reader = module('record_reply_original_readback', ROOT/HERE.parent/'registered.py')
registry_module = module('record_reply_original_registry', ROOT/HERE.parent/'envelope-writer/native.py')
registry_module.HERE = Path('.')
sha = base.sha


def mode(path):
    return '100755' if path.stat().st_mode & 0o111 else '100644'


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    if data.get('schema') != 1 or data.get('production_adopted') is not True:
        raise ValueError('the actual adopted reply helper is required')
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('templates', ROOT/HERE), ('drivers', ROOT/HERE)]:
        for name, digest in data[group].items():
            path = prefix/name
            if path.is_symlink() or not path.is_file() or sha(path) != digest:
                raise ValueError('reply '+group+' changed: '+name)
    if tuple(data['variants']) != CASES:
        raise ValueError('missing or extra actual reply source view')
    expected = set(data['common_proofs']) | {name for row in data['variants'].values()
        for name in row['proofs']}
    present = {p.name.removesuffix('.lean.in') for p in (ROOT/HERE).glob('*.lean.in')}
    if present != expected:
        raise ValueError('missing or unregistered reply proof template')
    index = json.loads((ROOT/HERE/'generated-index.json').read_text())
    if index['actual_source_sha256'] != sha(ROOT/MODEL):
        raise ValueError('actual reply source differs from reviewed generated index')
    for variant, row in index['variants'].items():
        for name, digest in row['generated_sha256'].items():
            if sha(ROOT/PUBLISHED/variant/'SandproofStage1Request'/name) != digest:
                raise ValueError('published original tool output changed')
    for name, row in data['root_namespace_adapters'].items():
        if sha(ROOT/name) != row['original_sha256']:
            raise ValueError('existing shared proof changed: '+name)
    return data


def mutate(project, mutation):
    path = project/MODEL
    before = sha(path)
    if mutation is not None:
        if mutation['source'] != MODEL:
            raise ValueError('mutation must affect the actual reply helper')
        text = path.read_text()
        if text.count(mutation['before']) != 1:
            raise ValueError('actual reply mutation is not unique')
        path.write_text(text.replace(mutation['before'], mutation['after'], 1))
    return dict(source=MODEL, original_sha256=before, variant_sha256=sha(path),
        mutation=mutation, actual_production_base=True, private_business_replacement=False)


def native_prepare(project, variant, data):
    plan = data['native_variants'][variant]
    derivation = mutate(project, plan['mutation'])
    if variant == 'restored':
        wrong = mutate(project, data['variants']['clear-terminal']['mutation'])
        path = project/MODEL
        path.write_bytes((ROOT/MODEL).read_bytes())
        if sha(path) != derivation['original_sha256']:
            raise ValueError('restored reply source is not the exact original')
        derivation['restoration'] = dict(wrong_sha256=wrong['variant_sha256'],
            restored_sha256=sha(path), exact_original_restored=True)
    return derivation, {MODEL: sha(project/MODEL)}


def install(project, variant, data, generated):
    installed = {}
    index = json.loads((ROOT/HERE/'generated-index.json').read_text())['variants'][variant]
    functions = json.loads((generated/'translation.json').read_text())['functions']
    observed = [{key: row[key] for key in ('lean_name', 'rust_name', 'is_opaque', 'lean_file')}
        for row in functions]
    if observed != index['functions']:
        raise ValueError('complete actual translated function inventory differs')
    for name, digest in index['generated_sha256'].items():
        source = generated/'SandproofStage1Request'/name
        if sha(source) != digest:
            raise ValueError('fresh actual tool output differs from published output: '+name)
        target = project/GEN/name
        shutil.copy2(source, target)
        installed[str(target.relative_to(project))] = digest
    # Preserve the original four fixed A6 external value definitions verbatim.
    library = str(GEN/'FunsExternal.lean')
    if sha(project/library) != data['providers'][library]:
        raise ValueError('original external value models changed')
    for name, row in data['root_namespace_adapters'].items():
        path = project/name
        if sha(path) != row['original_sha256']:
            raise ValueError('shared original proof differs before namespace adaptation')
        text = path.read_text()
        position = text.index('open Aeneas')
        path.write_text(text[:position]+data['root_namespace_open']+text[position:])
        if sha(path) != row['adapted_sha256']:
            raise ValueError('namespace-only proof adaptation changed')
        installed[name] = sha(path)
    stems = data['common_proofs']+data['variants'][variant]['proofs']
    for stem in stems:
        target = project/PROOFS/(stem+'.lean')
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), target)
        installed[str(target.relative_to(project))] = sha(target)
    if variant != 'actual' and any((project/PROOFS/(name+'.lean')).exists()
            for name in ('RecordReplyCore', 'RecordReplyWitnesses')):
        raise ValueError('wrong view contains the correct reply theorem')
    return installed


def proof_inventory(project, seeds):
    files = sorted(resolver.closure(project,
        [str(PROOFS/(name+'.lean')) for name in seeds], active_view='lakefile.toml'))
    audits = [name for filename in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (project/filename).read_text(), re.M)]
    if len(audits) != len(set(audits)):
        raise ValueError('duplicate reply audit names')
    return files, audits
