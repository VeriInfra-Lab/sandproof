"""One data-preserving binding for the actual fixed two-argument ID format."""
from pathlib import Path
import importlib.util
import json
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-fixed-identity')
SOURCE = 'controller-service/src/controller/identity.rs'
GEN = Path('verification/aeneas/stage1-protocol/create-fixed-identity/SandproofStage1CreateAttachment')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
ROOTS = ['sandproof_controller_service::controller::identity::proof_attachment_identity']


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


load = module('fixed_identity_original_load_tools', ROOT/HERE.parent/'inbox-load/formal.py')
runtime = module('fixed_identity_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
capacity = module('fixed_identity_virtual_wrapper', ROOT/HERE.parent/'create-load-capacity/support.py')
binding = module('fixed_identity_original_call_binding', ROOT/HERE/'binding.py')
sha = load.sha
INCLUDE = list(load.INCLUDE) + ['core::hint::must_use']
OPAQUE = ['rusqlite::error::Error']


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('templates', ROOT/HERE), ('drivers', ROOT/HERE)]:
        for name, digest in data[group].items():
            if sha(prefix/name) != digest:
                raise ValueError('reviewed fixed identity '+group+' changed: '+name)
    return data


def library_review(data):
    fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
    reviewed = {}
    for kind, toolchain in [('production', '1.97.0'), ('extraction', fixed['extraction_frontend'])]:
        sysroot = Path(subprocess.check_output(
            ['rustup', 'run', toolchain, 'rustc', '--print', 'sysroot'], text=True).strip())
        folder = sysroot/'lib/rustlib/src/rust/library'
        values = {name: sha(folder/name) for name in data['standard_library_sources'][kind]}
        if values != data['standard_library_sources'][kind]:
            raise ValueError('reviewed formatting library changed: '+kind)
        reviewed[kind] = dict(toolchain=toolchain, sources=values)
    return reviewed


def function_range(text):
    start = text.index('pub(super) fn sandbox_id(')
    opening = text.index('{', start)
    depth = 0
    for index in range(opening, len(text)):
        depth += (text[index] == '{') - (text[index] == '}')
        if depth == 0:
            return start, index+1
    raise ValueError('unclosed actual identity function')


def mutate(project, mutation):
    path = project/SOURCE
    before = sha(path)
    if mutation is not None:
        text = path.read_text()
        start, end = function_range(text)
        body = text[start:end]
        if body.count(mutation['before']) != 1:
            raise ValueError('actual identity mutation is not unique')
        path.write_text(text[:start]+body.replace(mutation['before'], mutation['after'], 1)+text[end:])
    return dict(source=SOURCE, original_sha256=before, variant_sha256=sha(path),
        mutation=mutation, actual_production_base=True, private_business_replacement=False)


def install(project, variant, data):
    installed = {}
    for stem in ['FixedFormatBinding', 'TypesExternal', 'FunsExternal']:
        path = project/GEN/(stem+'.lean')
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), path)
        installed[str(path.relative_to(project))] = sha(path)
    for stem in data['common_templates']+data['variants'][variant]['templates']:
        path = project/PROOFS/(stem+'.lean')
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), path)
        installed[str(path.relative_to(project))] = sha(path)
    # Only the independent view is extended. The shared root and Lake file
    # remain single-writer inputs for the coordinator.
    lake = project/'lakefile.toml'
    if 'name = "SandproofStage1CreateAttachment"' in lake.read_text():
        raise ValueError('identity view unexpectedly already registered')
    lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofStage1CreateAttachment"\n'
        'srcDir = "verification/aeneas/stage1-protocol/create-fixed-identity"\n')
    installed['lakefile.toml'] = sha(lake)
    return installed
