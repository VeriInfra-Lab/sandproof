"""Exact four-input current lifetime UPDATE adapter and complete value views."""
from pathlib import Path
import importlib.util
import json
import os
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-lifetime-sql')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/lifetime_sql.rs'
PARENT = 'controller-service/src/controller/lifecycle_inbox.rs'
GEN = Path('verification/aeneas/stage1-protocol/create-lifetime-sql/SandproofLifecycleSql')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
ROOTS = ['sandproof_controller_service::controller::lifecycle_inbox::lifetime_sql::'+name
         for name in ['proof_execute', 'proof_original_integer_to_sql', 'proof_original_text_to_sql']]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


load = module('lifetime_sql_existing_load', ROOT/HERE.parent/'inbox-load/formal.py')
runtime = module('lifetime_sql_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
capacity = module('lifetime_sql_original_wrapper', ROOT/HERE.parent/'create-load-capacity/support.py')
binding = module('lifetime_sql_actual_call_binding', ROOT/HERE/'binding.py')
common = module('lifetime_sql_exact_common_definitions', ROOT/HERE/'materialize.py')
sha = load.sha
INCLUDE = list(load.INCLUDE)+[
    'rusqlite::types::to_sql::ToSqlOutput', 'rusqlite::types::value_ref::ValueRef',
    'rusqlite::types::value::Value', 'rusqlite::types::to_sql::*::to_sql',
    'rusqlite::types::value_ref::*::from', 'rusqlite::params::*::__bind_in',
    'rusqlite::types::to_sql::*::from', 'rusqlite::types::value::*::from']
OPAQUE = ['rusqlite::error::Error']


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('templates', ROOT/HERE), ('drivers', ROOT/HERE)]:
        for name, digest in data[group].items():
            if sha(prefix/name) != digest:
                raise ValueError('reviewed lifetime SQL '+group+' changed: '+name)
    if common.hashes(ROOT) != data['exact_common_fragments']:
        raise ValueError('shared definition fragments changed')
    parent = (ROOT/PARENT).read_text()
    call = 'lifetime_sql::execute(self.connection, statement, deadline, sandbox)'
    if parent.count(call) != 1 or parent.count('mod lifetime_sql;') != 1:
        raise ValueError('unique actual production delegation missing')
    if 'Ok(self.connection.execute(statement, params![deadline, sandbox])?)' in parent:
        raise ValueError('old alternate production call remains')
    return data


def library_review(data):
    cargo = Path(os.environ.get('CARGO_HOME', str(Path.home()/'.cargo')))
    sources = {}
    for name, digest in data['rusqlite_sources'].items():
        matches = list((cargo/'registry/src').glob('*/rusqlite-0.40.2/'+name))
        if not matches or any(sha(p) != digest for p in matches):
            raise ValueError('fixed SQLite usepoint source changed: '+name)
        sources[name] = dict(sha256=digest, paths=[str(p) for p in matches])
    return sources


def mutate(project, mutation):
    path = project/SOURCE
    before = sha(path)
    if mutation is not None:
        text = path.read_text()
        if text.count(mutation['before']) != 1:
            raise ValueError('actual SQL mutation is not unique')
        path.write_text(text.replace(mutation['before'], mutation['after'], 1))
    return dict(source=SOURCE, original_sha256=before, variant_sha256=sha(path),
        mutation=mutation, actual_production_base=True, private_business_replacement=False)


def install(project, variant, data):
    installed = {}
    for stem in ['TypesExternal', 'FunsExternal']:
        path = project/GEN/(stem+'.lean')
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), path)
        installed[str(path.relative_to(project))] = sha(path)
    installed.update(common.install(ROOT, project, GEN, data['exact_common_fragments']))
    for stem in data['common_templates']+data['variants'][variant]['templates']:
        path = project/PROOFS/(stem+'.lean')
        shutil.copy2(ROOT/HERE/(stem+'.lean.in'), path)
        installed[str(path.relative_to(project))] = sha(path)
    lake = project/'lakefile.toml'
    if 'name = "SandproofLifecycleSql"' in lake.read_text():
        raise ValueError('lifetime SQL view already registered')
    # Only exempt the fixed Rust __bind_in symbol's generated library naming
    # style. Kernel, unused-tactic/simp and dependency checks remain enabled.
    lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofLifecycleSql"\n'
        'srcDir = "verification/aeneas/stage1-protocol/create-lifetime-sql"\n'
        'leanOptions = { weak.linter.style.nameCheck = false }\n')
    installed['lakefile.toml'] = sha(lake)
    return installed
