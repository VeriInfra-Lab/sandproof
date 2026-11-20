"""One actual representation for existing load, validation, selection and install."""
import importlib.util
import json
from pathlib import Path
import re
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-load-capacity')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SOURCE = 'controller-service/src/controller/lifecycle_inbox.rs'
PREFIX = 'sandproof_controller_service::controller::lifecycle_inbox::'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


load = module('capacity_original_load', ROOT/HERE.parent/'inbox-load/formal.py')
runtime = module('capacity_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
binding = module('capacity_full_root_binding', ROOT/HERE/'binding.py')
error_binding = module('capacity_full_error_binding', ROOT/HERE.parent/'envelope-writer/binding.py')
sha = load.sha
ROOTS = list(load.ROOTS) + [PREFIX+'proof_create_attachment_install', PREFIX+'proof_create_attachment_selection']
GEN, GRAPHIC = load.GENERATED, load.GRAPHIC
FORBIDDEN_WRONG = {'InboxCompositionProofs', 'InboxLoadComposition', 'InboxLoadWitnesses',
                   'CreateLoadOwnerBound', 'CreateLoadExamples', 'InboxLoadSuccess'}


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('templates', ROOT/HERE), ('drivers', ROOT/HERE)]:
        for name, digest in data[group].items():
            if sha(prefix/name) != digest:
                raise ValueError('reviewed '+group+' changed: '+name)
    load.check_inventory(ROOT)
    load.library_binding.check(ROOT)
    return data


def mutate(project, mutation):
    path = project/MODEL
    before = sha(path)
    if mutation is not None:
        text = path.read_text()
        start = text.index(mutation['function_marker'])
        brace = text.index('{', start)
        depth, end = 0, None
        for index in range(brace, len(text)):
            depth += int(text[index] == '{')-int(text[index] == '}')
            if depth == 0:
                end = index+1
                break
        if end is None:
            raise ValueError('unclosed original validator')
        body = text[start:end]
        if body.count(mutation['before']) != 1:
            raise ValueError('owner-limit mutation is not unique in original validator')
        path.write_text(text[:start]+body.replace(mutation['before'], mutation['after'], 1)+text[end:])
    return dict(source=MODEL, original_sha256=before, variant_sha256=sha(path), mutation=mutation,
                actual_production_base=True, private_business_replacement=False)


def prepare(project, variant, data):
    derivation = mutate(project, data['variants'][variant]['mutation'])
    views = load.native.preparation.views
    roots = [
        (MODEL, 'inbox-validation/generic-root.rs.in'),
        ('controller-service/src/capture.rs', 'inbox-validation/primitive-roots.rs.in'),
        ('verification/experiments/stage1-protocol/inbox-record/lib.rs', 'terminal-binding/roots.rs.in'),
        ('controller-service/src/controller/lifecycle_inbox/codec.rs', 'inbox-load/generic-root.rs.in'),
        (SOURCE, 'create-install/root.rs.in')]
    changed = {MODEL: sha(project/MODEL)}
    for name, provider in roots:
        views.append(project, name, (ROOT/HERE.parent/provider).read_text())
        changed[name] = sha(project/name)
    renewal = json.loads((ROOT/HERE.parent/'inbox-renewal-session/fragments.json').read_text())['renewal_roots']
    target = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'
    views.append(project, target, renewal)
    changed[target] = sha(project/target)
    return derivation, changed


def wrapper(project):
    paths = [Path.home(), runtime.tool_root(ROOT), runtime.package_root(ROOT)]
    import os
    for key in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
        if os.environ.get(key):
            paths.append(Path(os.environ[key]).resolve(strict=True))
    return ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home',
            *[part for path in dict.fromkeys(paths) for part in ('--ro-bind', str(path), str(path))],
            '--bind', str(project), str(runtime.VIRTUAL), '--chdir', str(runtime.VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(runtime.VIRTUAL/'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']


def install(project, variant, data):
    installed = load.library_binding.install(ROOT, project)
    lake = project/'lakefile.toml'
    if 'name = "SandproofStage1Graphic"' not in lake.read_text():
        lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofStage1Graphic"\nsrcDir = "verification/aeneas/stage1-protocol/graphic"\n')
    installed['lakefile.toml'] = sha(lake)
    old = load.check_inventory(ROOT)
    for name in old['views']['correct']['files']:
        if not name.startswith(str(PROOFS)+'/'):
            continue
        if variant != 'actual' and Path(name).stem in FORBIDDEN_WRONG:
            continue
        provider = old['providers'][Path(name).stem]['path']
        shutil.copy2(ROOT/provider, project/name)
        installed[name] = sha(project/name)
    prior = json.loads((ROOT/HERE.parent/'create-install/inventory.json').read_text())
    deduplicated = {}
    for name, provider in prior['common_proofs'].items():
        if name in installed and sha(ROOT/provider) != sha(project/name):
            source, existing = (ROOT/provider).read_text(), (project/name).read_text()
            if name == str(GEN/'TypesExternal.lean'):
                if not set(re.findall(r'^abbrev .+$', existing, re.M)) <= set(re.findall(r'^abbrev .+$', source, re.M)):
                    raise ValueError('original error alias changed')
                if source.count('abbrev sandproof_protocol.ContractError := SandproofFoundationsRust.sandproof_protocol.ContractError') != 1:
                    raise ValueError('complete original error alias missing')
                # Full original enum is independently extracted and compared before install.
            elif name == str(GEN/'FunsExternal.lean'):
                symbols = re.findall(r'^def (\S+)', source, re.M)
                if any(load.library_binding.fragment(source, n) != load.library_binding.fragment(existing, n) for n in symbols):
                    raise ValueError('existing library wrappers differ')
                deduplicated[name] = dict(original_full_load_provider_retained=True, required_symbols=symbols)
                continue
            else:
                raise ValueError('different proof provider: '+name)
        shutil.copy2(ROOT/provider, project/name)
        installed[name] = sha(project/name)
    for stem in prior['common_templates']+prior['variants']['actual']['templates']:
        name = str(PROOFS/(stem+'.lean'))
        shutil.copy2(ROOT/HERE.parent/'create-install'/(stem+'.lean.in'), project/name)
        installed[name] = sha(project/name)
    for stem, plan in data['store_format_adaptations'].items():
        name = str(PROOFS/(stem+'.lean'))
        source = project/name
        if sha(source) != plan['before_sha256']:
            raise ValueError('store-version adaptation has a different original: '+stem)
        text = source.read_text()
        text, count = re.subn(r'\b23\b', '24', text)
        if count != plan['occurrences']:
            raise ValueError('store-version adaptation range changed: '+stem)
        source.write_text(text)
        if sha(source) != plan['after_sha256']:
            raise ValueError('reviewed current store model differs: '+stem)
        installed[name] = sha(source)
    for stem in data['common_templates']+data['variants'][variant]['templates']:
        name = str(PROOFS/(stem+'.lean'))
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), project/name)
        installed[name] = sha(project/name)
    if variant != 'actual' and any((project/PROOFS/(n+'.lean')).exists() for n in FORBIDDEN_WRONG):
        raise ValueError('wrong view contains correct whole-validator/load correspondence')
    return installed, deduplicated
