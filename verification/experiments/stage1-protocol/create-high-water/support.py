"""One actual Inbox/World representation and the unchanged full protocol contracts."""
import importlib.util
import json
from pathlib import Path
import re
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-high-water')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
SOURCE = 'controller-service/src/controller/lifecycle_inbox.rs'
PREFIX = 'sandproof_controller_service::controller::lifecycle_inbox::'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


capacity = module('high_water_current_capacity', ROOT/HERE.parent/'create-load-capacity/support.py')
owner = module('high_water_current_owner', ROOT/HERE.parent/'create-owner-check/support.py')
load, runtime, binding, error_binding, sha = capacity.load, capacity.runtime, capacity.binding, capacity.error_binding, capacity.sha
ROOTS = list(capacity.ROOTS)+list(owner.ROOTS)
GEN, GRAPHIC, wrapper = capacity.GEN, capacity.GRAPHIC, capacity.wrapper


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('templates', ROOT/HERE), ('drivers', ROOT/HERE)]:
        for name, digest in data[group].items():
            if sha(prefix/name) != digest:
                raise ValueError('reviewed high-water '+group+' changed: '+name)
    capacity.inventory()
    owner.inventory()
    return data


def fixed_library_review(data):
    reviewed = owner.legacy.fixed_library_review(owner.inventory())
    public = {kind: {k: v for k, v in record.items() if k != 'path'}
              for kind, record in reviewed.items()}
    if public != data['fixed_library_review']:
        raise ValueError('original Ord/max source or finite dictionary adapter changed')
    return reviewed


def mutate(project, mutation):
    path = project/SOURCE
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
            raise ValueError('unclosed actual owner/install function')
        body = text[start:end]
        if body.count(mutation['before']) != 1:
            raise ValueError('mutation is not unique in the actual function')
        path.write_text(text[:start]+body.replace(mutation['before'], mutation['after'], 1)+text[end:])
    return dict(source=SOURCE, original_sha256=before, variant_sha256=sha(path), mutation=mutation,
                actual_production_base=True, private_business_replacement=False)


def prepare(project, variant, data):
    derivation = mutate(project, data['variants'][variant]['mutation'])
    _, changed = capacity.prepare(project, 'actual', capacity.inventory())
    load.native.preparation.views.append(project, SOURCE, (ROOT/HERE.parent/'create-owner-check/root.rs.in').read_text())
    changed[SOURCE] = sha(project/SOURCE)
    return derivation, changed


def renamed(text):
    return text.replace('SandproofStage1OwnerMapping', 'SandproofStage1Request')


def install(project, variant, data):
    installed, retained = capacity.install(project, 'actual', capacity.inventory())
    owner_data = owner.inventory()
    adaptations = {}
    for name, provider in owner_data['common_proofs'].items():
        source = ROOT/provider
        target_name = name.replace('/owner-mapping/SandproofStage1OwnerMapping/', '/request/SandproofStage1Request/')
        target = project/target_name
        after = renamed(source.read_text())
        if name.startswith('verification/') and Path(name).name in ('FunsExternal.lean', 'SharedValueBindings.lean', 'TypesExternal.lean'):
            existing = target.read_text()
            if Path(name).name == 'TypesExternal.lean':
                if not set(re.findall(r'^abbrev .+$', after, re.M)) <= set(re.findall(r'^abbrev .+$', existing, re.M)):
                    raise ValueError('original carrier aliases do not embed')
            else:
                for symbol in re.findall(r'^def (\S+)', after, re.M):
                    fragment = load.library_binding.fragment
                    if fragment(after.replace('\nend ', '\n\nend '), symbol) != fragment(existing.replace('\nend ', '\n\nend '), symbol):
                        raise ValueError('existing original library symbol differs: '+symbol)
            retained[target_name] = dict(provider=provider, full_load_provider_kept=True, sha256=sha(target))
            continue
        if Path(name).name == 'OrdDefaultBinding.lean':
            generated = (project/GEN/'Funs.lean').read_text()
            if re.search(r'^def core\.cmp\.Ord\.max\.default\b', generated, re.M):
                raise ValueError('actual generated default max now exists; review the adapter')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(after)
        installed[target_name] = sha(target)
        adaptations[target_name] = dict(provider=provider, original_sha256=sha(source),
            adapted_sha256=sha(target), only_library_namespace_renamed=True)
    for stem in owner_data['common_templates']+owner_data['variants']['actual']['templates']:
        source = ROOT/HERE.parent/'create-owner-check'/(stem+'.lean.in')
        name = str(PROOFS/(stem+'.lean'))
        target = project/name
        target.write_text(renamed(source.read_text()))
        installed[name] = sha(target)
        adaptations[name] = dict(provider=str(source.relative_to(ROOT)), original_sha256=sha(source),
            adapted_sha256=sha(target), only_library_namespace_renamed=True)
    external = project/GEN/'FunsExternal.lean'
    text = external.read_text()
    if 'import SandproofStage1Request.OrdDefaultBinding\n' in text:
        raise ValueError('duplicate default binding import')
    external.write_text('import SandproofStage1Request.OrdDefaultBinding\n'+text)
    installed[str(external.relative_to(project))] = sha(external)
    for stem in data['common_templates']+data['variants'][variant]['templates']:
        target = project/PROOFS/(stem+'.lean')
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), target)
        installed[str(target.relative_to(project))] = sha(target)
    if variant == 'wrong-seed':
        provider = ROOT/HERE.parent/'create-owner-successor/HighWaterCounter.lean.in'
        target = project/PROOFS/'CreateHighWaterSeedObserved.lean'
        target.write_text(renamed(provider.read_text()))
        installed[str(target.relative_to(project))] = sha(target)
        adaptations[str(target.relative_to(project))] = dict(provider=str(provider.relative_to(ROOT)),
            original_sha256=sha(provider), adapted_sha256=sha(target), only_library_namespace_renamed=True)
    seeds = [str(PROOFS/(n+'.lean')) for n in data['variants'][variant]['seeds']]
    # Every retained generated file is checked, including the separate primitive view.
    seeds += [str(directory/name) for directory in (GEN, GRAPHIC)
              for name in ('Types.lean','TypesExternal.lean','Funs.lean','FunsExternal.lean')]
    files = set(load.resolver.closure(project, seeds, active_view='lakefile.toml'))
    removed = []
    for name in list(installed):
        if name.endswith('.lean') and name not in files:
            # Only retire freshly installed copies outside this view; original providers remain.
            (project/name).unlink()
            removed.append(name)
            del installed[name]
    forbidden = data['variants'][variant]['forbidden_correct_correspondences']
    if any((project/PROOFS/(n+'.lean')).exists() for n in forbidden):
        raise ValueError('wrong view retains a false correct correspondence')
    return installed, dict(retained=retained, namespace_only_adaptations=adaptations,
                          freshly_copied_unused_providers=removed)
