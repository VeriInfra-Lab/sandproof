"""Actual Std SQL delegation, reusing the single complete helper model."""
from pathlib import Path
import importlib.util
import json
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-standard-sql')
SOURCE = 'controller-service/src/controller/lifecycle_inbox.rs'
HELPER_SOURCE = 'controller-service/src/controller/lifecycle_inbox/lifetime_sql.rs'
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
STD = Path('verification/aeneas/stage1-protocol/standard-create/SandproofStandardCreate')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


helper = module('std_sql_original_helper', ROOT/HERE.parent/'create-lifetime-sql/support.py')
standard = module('std_sql_complete_carriers', ROOT/HERE.parent/'create-standard-adapter/support.py')
metadata = module('std_sql_bounded_metadata', ROOT/HERE.parent/'create-standard-adapter/llbc_metadata.py')
load, runtime, capacity, sha = helper.load, helper.runtime, helper.capacity, helper.sha
GEN = helper.GEN
ROOTS = ['sandproof_controller_service::controller::lifecycle_inbox::proof_standard_create_sql']+['sandproof_controller_service::controller::lifecycle_inbox::lifetime_sql::'+name for name in ['proof_original_integer_to_sql','proof_original_text_to_sql']]
INCLUDE = list(dict.fromkeys(helper.INCLUDE+['rusqlite::transaction::Transaction']))
OPAQUE = ['rusqlite::error::Error']


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('templates', ROOT/HERE), ('drivers', ROOT/HERE)]:
        for name, digest in data[group].items():
            if sha(prefix/name) != digest:
                raise ValueError('reviewed Std SQL '+group+' changed: '+name)
    text = (ROOT/SOURCE).read_text()
    if text.count('lifetime_sql::execute(self.connection, statement, deadline, sandbox)') != 1:
        raise ValueError('unique actual Std SQL delegation changed')
    if helper.inventory() != data['helper_inventory']:
        raise ValueError('complete helper/provider inventory changed')
    return data


def library_review(data):
    return helper.library_review(data['helper_inventory'])


def mutate(project, mutation):
    path = project/SOURCE
    before = sha(path)
    if mutation is not None:
        text = path.read_text()
        if text.count(mutation['before']) != 1:
            raise ValueError('actual Std SQL wire mutation is not unique')
        path.write_text(text.replace(mutation['before'], mutation['after'], 1))
    return dict(source=SOURCE, original_sha256=before, variant_sha256=sha(path),
        mutation=mutation, actual_production_base=True,
        helper_changed=False, private_business_replacement=False)


def admit(ir, output):
    small = metadata.summary(ir)
    (output/'llbc-summary.json').write_text(json.dumps(small, indent=2)+'\n')
    options = small['options']
    if (small['target_keys'] != ['x86_64-unknown-linux-gnu'] or small['has_errors'] is not False
            or options['start_from'] != ROOTS or options['include'] != INCLUDE
            or options['opaque'] != OPAQUE or any(options[k] for k in [
                'exclude','skip_borrowck','no_typecheck','monomorphize','remove_unused_clauses'])):
        raise ValueError('incomplete or changed fixed Std SQL extraction')
    if (len(small['roots']) != 1 or small['roots'][0]['opacity'] != 'Transparent'
            or small['roots'][0]['arg_count'] != 4 or len(small['roots'][0]['calls']) != 1):
        raise ValueError('private original four-argument forwarding root changed')
    if (len(small['selected_callees']) != 1 or small['selected_callees'][0]['leaf'] != 'execute_lifetime_update'
            or small['selected_callees'][0]['opacity'] != 'Transparent'
            or small['selected_callees'][0]['prefix'] != [
                'sandproof_controller_service','controller','lifecycle_inbox']):
        raise ValueError('root does not call the actual transparent Std delegate')
    return dict(roots=small['roots'], actual_callees=small['selected_callees'],
        bounded_reader=small['reader_bounds'], summary_sha256=sha(output/'llbc-summary.json'))


def generated_carriers(types, data):
    return standard.generated_carriers(types, data)


def install(project, variant, data):
    # The actual helper body/math stays unchanged even in a wrong Std wire view.
    installed = helper.install(project, 'actual', data['helper_inventory'])
    path = project/GEN/'TypesExternal.lean'
    shutil.copy2(ROOT/HERE/'TypesExternal.lean.in', path)
    installed[str(path.relative_to(project))] = sha(path)
    path = project/STD/'TypesExternal.lean'
    path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT/HERE.parent/'create-standard-adapter/TypesExternal.lean.in', path)
    installed[str(path.relative_to(project))] = sha(path)
    for stem in data['common_templates']+data['variants'][variant]['templates']:
        path = project/PROOFS/(stem+'.lean')
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), path)
        installed[str(path.relative_to(project))] = sha(path)
    lake = project/'lakefile.toml'
    if 'name = "SandproofStandardCreate"' in lake.read_text():
        raise ValueError('Std SQL carrier view unexpectedly already registered')
    lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofStandardCreate"\n'
        'srcDir = "verification/aeneas/stage1-protocol/standard-create"\n')
    installed['lakefile.toml'] = sha(lake)
    if variant != 'actual' and (project/PROOFS/'StandardSqlDelegate.lean').exists():
        raise ValueError('wrong Std wire installed correct Std theorem')
    return installed
