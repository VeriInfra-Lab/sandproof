"""Reuse the canonical BLOB/Load providers and the unchanged SHA mathematics.

Generated bodies are copied from fixed-tool outputs, never rewritten here.
"""
from pathlib import Path
import importlib.util
import json
import re
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-digest')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
GEN = Path('verification/aeneas/stage1-protocol/standard-digest/SandproofStandardDigest')
ARRAY = Path('verification/aeneas/stage1-protocol/digest-array32/SandproofDigestArray32/Types.lean')
MATH = ['Sha256Bytes', 'Sha256BytesChecks', 'DigestArray32Representation', 'DigestByteBridge']
COMMON = ['StandardDigestModel', 'StandardDigestLibrary', 'StandardDigestValues']
VIEWS = {
    'actual': ['StandardDigestCore', 'StandardDigestBridge', 'StandardDigestWitnesses'],
    'input-empty': ['StandardDigestInputCounter'],
    'nonempty-tail-zero': ['StandardDigestTailCounter'],
}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


blob = module('standard_digest_canonical_blob', ROOT/HERE.parent/'standard-blob/support.py')
preparation = module('standard_digest_source_preparation', ROOT/HERE/'preparation.py')
base, checker, resolver = blob.base, blob.checker, blob.resolver
runtime, reader, registry_module = blob.runtime, blob.reader, blob.registry_module
sha, mode, ir_value_digest = blob.sha, blob.mode, blob.ir_value_digest


def retained_roots(variant):
    return COMMON+VIEWS[variant]+blob.COMMON+(blob.VIEWS['actual'] if variant == 'actual' else [])


def inventory():
    """Admission is tied to actual semantic captures and retained proof census."""
    blob.inventory()
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    if data['source_adopted'] is not True or data['library_provider_adopted'] is not True or data['full_v3']:
        raise ValueError('this entry requires the adopted finite source and explicit library conditions')
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('package_inputs', ROOT/HERE), ('generated', ROOT)]:
        for name, digest in data[group].items():
            path = prefix/name
            if path.is_symlink() or not path.is_file() or sha(path) != digest:
                raise ValueError('reviewed '+group+' changed: '+name)
    owned = {p.stem.removesuffix('.lean') for p in (ROOT/HERE).glob('*.lean.in')}
    if owned != set(COMMON).union(*map(set, VIEWS.values())):
        raise ValueError('an owned correct or wrong proof template is missing')
    audits = {stem: re.findall(r'^#audit_controller_proof (\S+)$',
        (ROOT/HERE/(stem+'.lean.in')).read_text(), re.M) for stem in sorted(owned)}
    if audits != data['owned_audits']:
        raise ValueError('owned audit census differs')
    provenance = json.loads((ROOT/HERE/'reuse-provenance.json').read_text())
    for name, digest in provenance['math_and_generated_data_unchanged'].items():
        if sha(ROOT/HERE/'reuse'/name) != digest:
            raise ValueError('the previously checked SHA mathematics/data changed')
    return data


def materialize(project, variant, generated):
    """Independent value views; no combined concrete standard IO is claimed."""
    changed = {}

    def copy(source, destination):
        target = project/destination
        if source.is_symlink() or not source.is_file():
            raise ValueError('non-regular mathematical/generated provider')
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        changed[str(destination)] = sha(target)

    if variant == 'actual':
        changed.update(blob.materialize(project, 'actual'))
    else:
        for stem in blob.COMMON:
            copy(ROOT/blob.HERE/(stem+'.lean.in'), PROOFS/(stem+'.lean'))
        for stem in ('TypesExternal', 'FunsExternal', 'ReadValueData', 'ReadTypeBindings'):
            copy(ROOT/blob.HERE/'external'/(stem+'.lean.in'), blob.READ_GEN/(stem+'.lean'))
    # Preserve the exact published independent Read/Load/Graphic providers.
    index = json.loads((ROOT/blob.HERE/'generated-index.json').read_text())
    for key, destination in [('standard-actual', blob.READ_GEN),
                              *([('load-canonical', blob.LOAD_GEN),
                                 ('primitive', blob.GRAPHIC_GEN)] if variant == 'actual' else [])]:
        for name in ('Types.lean', 'Funs.lean'):
            entry = index['captures'][key]['published'][destination.name+'/'+name]
            source = ROOT/entry['path']
            if sha(source) != entry['sha256']:
                raise ValueError('original generated provider changed')
            copy(source, destination/name)
    for stem in MATH:
        copy(ROOT/HERE/'reuse'/(stem+'.lean.in'), PROOFS/(stem+'.lean'))
    copy(ROOT/HERE/'reuse'/ARRAY, ARRAY)
    for name in ('Types.lean', 'Funs.lean'):
        copy(generated/GEN.name/name, GEN/name)
    for stem in ('TypesExternal', 'FunsExternal'):
        copy(ROOT/HERE/'external'/(stem+'.lean.in'), GEN/(stem+'.lean'))
    for stem in COMMON+VIEWS[variant]:
        copy(ROOT/HERE/(stem+'.lean.in'), PROOFS/(stem+'.lean'))
    if variant != 'actual':
        forbidden = ['StandardDigestCore', 'StandardDigestBridge', 'StandardDigestWitnesses',
                     'BlobColumnsCore', 'StandardBlobCore', 'StandardBlobWitnesses',
                     'StandardBlobNormalContext', 'InboxLoadStandardBlobDigestDomain']
        if any((project/PROOFS/(stem+'.lean')).exists() for stem in forbidden):
            raise ValueError('wrong environment contains a correct correspondence')
    lake = project/'lakefile.toml'
    text = lake.read_text()
    for name, directory in [('SandproofLoadBlob', blob.READ_GEN.parent),
                            ('SandproofStorage', Path('verification/aeneas/storage')),
                            ('SandproofStandardDigest', GEN.parent),
                            ('SandproofDigestArray32', ARRAY.parent.parent)]:
        if 'name = "'+name+'"' not in text:
            text += '\n[[lean_lib]]\nname = "'+name+'"\nsrcDir = "'+str(directory)+'"\n'
            if name == 'SandproofLoadBlob':
                text += 'leanOptions = { weak.linter.style.nameCheck = false }\n'
    lake.write_text(text)
    changed['lakefile.toml'] = sha(lake)
    return changed
