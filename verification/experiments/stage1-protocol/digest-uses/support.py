"""One current Rust digest implementation and one complete protocol value contract."""
from pathlib import Path
import importlib.util
import json
import re
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/digest-uses')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
GEN = Path('verification/aeneas/stage1-protocol/digest-uses/SandproofDigestUses')
MODEL = 'controller-service/src/controller/lifecycle_inbox/source_work.rs'
TEST_ROOT = 'controller-service/src/controller/lifecycle_inbox.rs'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('fixed_sha_original_snapshot', ROOT/HERE.parent/'schema-leaves/formal.py')
load = module('fixed_sha_original_checker', ROOT/HERE.parent/'inbox-load/formal.py')
runtime = module('fixed_sha_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
capture = module('fixed_sha_actual_capture', ROOT/HERE/'capture.py')
binding = module('fixed_sha_finite_binding', ROOT/HERE/'binding.py')
reader = module('fixed_sha_original_readback', ROOT/HERE.parent/'registered.py')
sha = base.sha


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group, prefix in [('source_inputs',ROOT),('providers',ROOT),
                          ('templates',ROOT/HERE),('drivers',ROOT/HERE)]:
        for name, digest in data[group].items():
            if sha(prefix/name) != digest:
                raise ValueError('fixed digest '+group+' changed: '+name)
    if sha(ROOT/HERE/'capture.py') != data['driver_sha256']:
        raise ValueError('capture driver changed')
    leaf = (ROOT/'controller-service/src/controller/lifecycle_inbox/sha256.rs').read_text()
    if leaf != data['actual_leaf_source']:
        raise ValueError('fixed leaf gained a changed call, branch or object access')
    if (ROOT/MODEL).read_text().count('ContentDigest::from_sha256(super::sha256::digest(bytes))') != 1:
        raise ValueError('actual source digest call changed')
    return data


def mutate(project, mutation):
    path = project/MODEL
    before = sha(path)
    if mutation:
        assert mutation['source'] == MODEL
        text = path.read_text()
        if text.count(mutation['before']) != 1:
            raise ValueError('actual digest mutation is not unique')
        path.write_text(text.replace(mutation['before'],mutation['after'],1))
    return dict(source=MODEL,original_sha256=before,variant_sha256=sha(path),
        mutation=mutation,actual_production_base=True,private_business_replacement=False)


def native_prepare(project, variant, data):
    mutation = data['native_variants'][variant]['mutation']
    derivation = mutate(project,mutation)
    if variant == 'restored':
        wrong = mutate(project,data['modes']['tail-zero']['mutation'])
        path = project/MODEL
        original = (ROOT/MODEL).read_bytes()
        path.write_bytes(original)
        assert sha(path) == derivation['original_sha256']
        derivation['restoration'] = dict(wrong_sha256=wrong['variant_sha256'],
            restored_sha256=sha(path),exact_original_restored=True)
    changes = {MODEL:sha(project/MODEL)}
    codec = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
    for target, fragment, marker in [
        (codec,'native-codec-root.rs.in','proof_digest_instance_save'),
    ]:
        path = project/target
        if marker in path.read_text():
            raise ValueError('native thin call was already instrumented')
        path.write_bytes(path.read_bytes()+(ROOT/HERE/fragment).read_bytes())
        changes[target] = sha(path)
    return derivation,changes


def install(project, variant, data, generated):
    installed = {}
    for name in ['Types.lean','Funs.lean']:
        target = project/GEN/name
        target.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(generated/'SandproofDigestUses'/name,target)
        installed[str(target.relative_to(project))] = sha(target)
    for name in ['TypesExternal.lean','FunsExternal.lean']:
        target = project/GEN/name
        shutil.copy2(ROOT/HERE/(name+'.in'),target)
        installed[str(target.relative_to(project))] = sha(target)
    # These are the exact existing generated data providers, not alternative
    # protocol/World definitions. Preserve their original provenance and bytes.
    for name, entry in data['canonical_generated_data'].items():
        source = ROOT/HERE/entry['provider']
        assert sha(source) == entry['sha256']
        target = project/name
        target.parent.mkdir(parents=True,exist_ok=True)
        if target.exists() and sha(target) != entry['sha256']:
            raise ValueError('canonical generated provider conflicts: '+name)
        shutil.copy2(source,target)
        installed[name] = sha(target)
    name = 'verification/aeneas/stage1-protocol/standard-create/SandproofStandardCreate/TypesExternal.lean'
    target = project/name
    provider = ROOT/HERE.parent/'create-standard-adapter/TypesExternal.lean.in'
    shutil.copy2(provider,target)
    installed[name] = sha(target)
    stems = data['common_proofs']+data['variants'][variant]['proofs']
    for stem in stems:
        target = project/PROOFS/(stem+'.lean')
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'),target)
        installed[str(target.relative_to(project))] = sha(target)
    if variant != 'actual':
        for name in ['DigestUses','DigestUsesWitnesses']:
            if (project/PROOFS/(name+'.lean')).exists():
                raise ValueError('wrong view contains the correct caller theorem')
    lake = project/'lakefile.toml'
    text = lake.read_text()
    for name,folder in [('SandproofStandardCreate','standard-create'),
        ('SandproofDigestArray32','digest-array32'),('SandproofDigestUses','digest-uses')]:
        if 'name = "'+name+'"' in text:
            raise ValueError('private digest carrier view unexpectedly already registered: '+name)
        text += '\n[[lean_lib]]\nname = "'+name+'"\nsrcDir = "verification/aeneas/stage1-protocol/'+folder+'"\n'
    lake.write_text(text)
    installed['lakefile.toml'] = sha(lake)
    return installed


def generated_inventory(generated, recipe):
    value = json.loads((generated/'translation.json').read_text())
    fields = ('lean_name','rust_name','is_opaque','lean_file')
    observed = [{key:f[key] for key in fields} for f in value['functions']]
    if observed != recipe['generated_functions']:
        raise ValueError('actual complete function inventory differs')
    return value


def proof_inventory(project, seeds):
    files = sorted(load.resolver.closure(project,
        [str(PROOFS/(name+'.lean')) for name in seeds],active_view='lakefile.toml'))
    audits = [n for f in files for n in re.findall(r'^#audit_controller_proof (\S+)$',
        (project/f).read_text(),re.M)]
    if len(audits) != len(set(audits)):
        raise ValueError('duplicate digest audit names')
    return files,audits
