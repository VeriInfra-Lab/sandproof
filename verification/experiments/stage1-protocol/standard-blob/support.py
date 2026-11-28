"""One read contract, unchanged protocol/Load providers, exact private views."""
from pathlib import Path
import hashlib
import importlib.util
import json
import re
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-blob')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
READ_GEN = Path('verification/aeneas/stage1-protocol/load-blob/SandproofLoadBlob')
LOAD_GEN = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
GRAPHIC_GEN = Path('verification/aeneas/stage1-protocol/graphic/SandproofStage1Graphic')
COMMON = ['BlobColumnsModel', 'BlobColumnsLibrary', 'BlobColumnsValues',
          'StandardBlobModel', 'StandardBlobValues', 'StandardBlobLibrary', 'StandardBlobObserve']
VIEWS = {
    'actual': ['BlobColumnsCore', 'StandardBlobCore', 'StandardBlobWitnesses',
               'StandardBlobDomainSupport', 'StandardBlobDomain', 'StandardBlobNormalContext',
               'StandardBlobReplyBridge', 'InboxLoadBlobDigestDomain', 'InboxLoadStandardBlobDigestDomain'],
    'repeat-first': ['StandardBlobResult', 'NegativeBlobRepeat', 'NegativeStandardBlobRepeat'],
    'second-error-first': ['StandardBlobResult', 'NegativeBlobOrder', 'NegativeStandardBlobOrder'],
}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


base = module('standard_blob_source_snapshot', ROOT/HERE.parent/'schema-leaves/formal.py')
checker = module('standard_blob_real_file_checker', ROOT/HERE.parent/'formal.py')
resolver = module('standard_blob_original_closure', ROOT/'tools/proof_batch.py')
runtime = module('standard_blob_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
reader = module('standard_blob_original_readback', ROOT/HERE.parent/'registered.py')
registry_module = module('standard_blob_locked_registry', ROOT/HERE.parent/'envelope-writer/native.py')
registry_module.HERE = Path('.')
preparation = module('standard_blob_source_preparation', ROOT/HERE/'preparation.py')
sha = base.sha


def mode(path):
    return '100755' if path.stat().st_mode & 0o111 else '100644'


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    if data['production_adopted'] is not True or data['finite_library_provider_adopted'] is not True:
        raise ValueError('the actual source and finite library interfaces must be explicitly adopted')
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('templates', ROOT/HERE), ('drivers', ROOT/HERE),
                          ('generated', ROOT)]:
        for name, digest in data[group].items():
            path = prefix/name
            if path.is_symlink() or not path.is_file() or sha(path) != digest:
                raise ValueError('reviewed '+group+' changed: '+name)
    owned = {p.stem.removesuffix('.lean') for p in (ROOT/HERE).glob('*.lean.in')}
    if owned != set(COMMON).union(*map(set, VIEWS.values())):
        raise ValueError('missing or unregistered retained proof template')
    expected = {stem: re.findall(r'^#audit_controller_proof (\S+)$',
        (ROOT/HERE/(stem+'.lean.in')).read_text(), re.M) for stem in sorted(owned)}
    if expected != data['owned_audits']:
        raise ValueError('owned proof dependency audit inventory changed')
    return data


def ir_value_digest(raw):
    """The original unique-key short_names table is emitted in hash-map order.

    Retain every original byte/hash; compare its full JSON value using only
    that table's key order. No declaration, body or other array is reordered.
    """
    rows = raw['translated']['short_names']
    if not isinstance(rows, list) or any(set(row) != {'key', 'value'} for row in rows):
        raise ValueError('unexpected original short-name table shape')
    keys = [json.dumps(row['key'], sort_keys=True) for row in rows]
    if len(keys) != len(set(keys)):
        raise ValueError('duplicate original short-name key')
    translated = dict(raw['translated'], short_names=sorted(rows,
        key=lambda row: json.dumps(row['key'], sort_keys=True)))
    value = dict(raw, translated=translated)
    return hashlib.sha256(json.dumps(value, sort_keys=True,
        separators=(',', ':')).encode()).hexdigest()


def check_capture(row, key):
    """Compare fresh original extraction with the retained, unedited output."""
    index = json.loads((ROOT/HERE/'generated-index.json').read_text())
    expected = index['captures'][key]
    if row['ir_value_sha256'] != expected['ir_value_sha256']:
        raise ValueError('current complete original IR value differs from published capture: '+key)
    target = Path(row['generated_directory'])
    for relative, entry in expected['published'].items():
        original, published = target/relative, ROOT/entry['path']
        if (original.is_symlink() or published.is_symlink()
                or sha(original) != entry['sha256'] or sha(published) != entry['sha256']
                or original.read_bytes() != published.read_bytes()):
            raise ValueError('fresh generated body differs from retained output: '+key+':'+relative)
    return dict(complete=True, key=key, ir_sha256=row['ir_sha256'],
                original_published_ir_sha256=expected['ir_sha256'],
                full_ir_value_sha256=row['ir_value_sha256'],
                only_ordered_table='translated.short_names (unique keys)',
                generated_index_sha256=sha(ROOT/HERE/'generated-index.json'),
                published=expected['published'])


def materialize(project, variant):
    changed = {}
    if variant == 'actual':
        providers = json.loads((ROOT/HERE/'materialization.json').read_text())['providers']
        for name, entry in providers.items():
            provider = ROOT/entry['provider']
            if sha(provider) != entry['provider_sha256']:
                raise ValueError('existing exact provider changed: '+entry['provider'])
            value = provider.read_bytes()
            if entry['transformation'] == 'only-existing-library-namespace':
                value = value.replace(b'SandproofStage1OwnerMapping', b'SandproofStage1Request')
            elif entry['transformation'] == 'reviewed-store23-to24-literals':
                value, count = re.subn(rb'\b23\b', b'24', value)
                if count != entry['literal_occurrences']:
                    raise ValueError('existing reviewed Store24 adaptation changed')
            elif entry['transformation'] != 'identity':
                raise ValueError('unreviewed existing provider transformation')
            if hashlib.sha256(value).hexdigest() != entry['sha256']:
                raise ValueError('existing mathematical provider differs: '+name)
            target = project/name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(value)
            changed[name] = sha(target)
        target = project/LOAD_GEN/'FunsExternal.lean'
        shutil.copy2(ROOT/HERE/'external/LoadFunsExternal.lean.in', target)
        changed[str(target.relative_to(project))] = sha(target)
    for name in ('TypesExternal', 'FunsExternal', 'ReadValueData', 'ReadTypeBindings'):
        target = project/READ_GEN/(name+'.lean')
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT/HERE/'external'/(name+'.lean.in'), target)
        changed[str(target.relative_to(project))] = sha(target)
    for stem in COMMON+VIEWS[variant]:
        target = project/PROOFS/(stem+'.lean')
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), target)
        changed[str(target.relative_to(project))] = sha(target)
    if variant != 'actual':
        forbidden = ('BlobColumnsCore', 'StandardBlobCore', 'StandardBlobWitnesses',
                     'StandardBlobNormalContext', 'StandardBlobReplyBridge',
                     'InboxLoadStandardBlobDigestDomain')
        if any((project/PROOFS/(name+'.lean')).exists() for name in forbidden):
            raise ValueError('wrong source view contains correct correspondence')
    lake = project/'lakefile.toml'
    text = lake.read_text()
    for name, directory in [('SandproofLoadBlob', READ_GEN.parent),
                            ('SandproofStorage', Path('verification/aeneas/storage')),
                            ('SandproofStage1Graphic', GRAPHIC_GEN.parent)]:
        if 'name = "'+name+'"' not in text:
            text += '\n[[lean_lib]]\nname = "'+name+'"\nsrcDir = "'+str(directory)+'"\n'
            if name == 'SandproofLoadBlob':
                # The fixed generated __bind_in identifier needs this existing
                # naming-linter setting; all kernel/dependency checks remain on.
                text += 'leanOptions = { weak.linter.style.nameCheck = false }\n'
    lake.write_text(text)
    changed['lakefile.toml'] = sha(lake)
    return changed
