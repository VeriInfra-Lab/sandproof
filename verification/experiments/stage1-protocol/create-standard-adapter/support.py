"""Current concrete Create methods, one full specification and finite library use."""
from pathlib import Path
import importlib.util
import json
import os
import re
import shutil

ROOT=Path(__file__).resolve().parents[4]
HERE=Path('verification/experiments/stage1-protocol/create-standard-adapter')
SOURCE='controller-service/src/controller/lifecycle_inbox.rs'
V5='controller-service/src/controller/transaction.rs'
GEN=Path('verification/aeneas/stage1-protocol/standard-create/SandproofStandardCreate')
PROOFS=Path('SandboxModel/Lifecycle/Protocol')
PREFIX='sandproof_controller_service::controller::lifecycle_inbox::'


def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path)
    value=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


load=module('standard_create_existing_load',ROOT/HERE.parent/'inbox-load/formal.py')
runtime=module('standard_create_fixed_runtime',ROOT/HERE.parent/'formal_runtime.py')
capacity=module('standard_create_original_wrapper',ROOT/HERE.parent/'create-load-capacity/support.py')
metadata=module('standard_create_bounded_metadata',ROOT/HERE/'llbc_metadata.py')
sha=load.sha
ROOTS=[PREFIX+n for n in ['proof_standard_create_world',
    'proof_standard_create_begin_lifetime_update','proof_standard_create_begin_inbox_save']]
INCLUDE=list(load.INCLUDE)+['core::option::*::as_ref',
    'rusqlite::transaction::Transaction','rusqlite::transaction::*::deref']
OPAQUE=['rusqlite::error::Error']


def inventory():
    data=json.loads((ROOT/HERE/'inventory.json').read_text())
    for group,prefix in [('source_inputs',ROOT),('providers',ROOT),
                         ('templates',ROOT/HERE),('drivers',ROOT/HERE)]:
        for n,h in data[group].items():
            if sha(prefix/n)!=h:
                raise ValueError('reviewed standard Create '+group+' changed: '+n)
    text=(ROOT/SOURCE).read_text()
    body='fn world(&mut self) -> core::World {\n        self.tx.world()\n    }'
    if text.count(body)!=1:
        raise ValueError('unique actual standard World method missing or changed')
    return data


def library_review(data):
    rustup=Path(os.environ.get('RUSTUP_HOME',str(Path.home()/'.rustup')))
    cargo=Path(os.environ.get('CARGO_HOME',str(Path.home()/'.cargo')))
    observed={}
    for role,entry in data['rust_library_sources'].items():
        source=rustup/'toolchains'/entry['toolchain']/'lib/rustlib/src/rust/library'
        for n,h in entry['files'].items():
            p=source/n
            if sha(p)!=h: raise ValueError('fixed '+role+' library source changed: '+n)
            observed[role+':'+n]=dict(path=str(p),sha256=h)
    for n,h in data['rusqlite_sources'].items():
        matches=list((cargo/'registry/src').glob('*/rusqlite-0.40.2/'+n))
        if not matches or any(sha(p)!=h for p in matches):
            raise ValueError('fixed rusqlite source changed: '+n)
        observed['rusqlite:'+n]=dict(paths=[str(p) for p in matches],sha256=h)
    return observed


def mutate(project,mutation):
    path=project/SOURCE
    before=sha(path)
    if mutation is not None:
        text=path.read_text()
        if text.count(mutation['before'])!=1:
            raise ValueError('wrong actual standard World method is not unique')
        path.write_text(text.replace(mutation['before'],mutation['after'],1))
    return dict(source=SOURCE,original_sha256=before,variant_sha256=sha(path),
        mutation=mutation,actual_production_base=True,
        candidate_world_call_preserved_once=True,v5_production_changed=False)


def admit(ir,output):
    small=metadata.summary(ir)
    (output/'llbc-summary.json').write_text(json.dumps(small,indent=2)+'\n')
    options=small['options']
    if small['target_keys']!=['x86_64-unknown-linux-gnu']:
        raise ValueError('wrong actual extraction target')
    if (small['has_errors'] is not False or options['start_from']!=ROOTS
        or options['include']!=INCLUDE or options['opaque']!=OPAQUE
        or any(options[k] for k in ['exclude','skip_borrowck','no_typecheck',
            'monomorphize','remove_unused_clauses'])):
        raise ValueError('incomplete or changed fixed extraction options')
    if len(small['roots'])!=3 or any(x['opacity']!='Transparent' or x['arg_count']!=1
        or len(x['calls'])!=1 for x in small['roots']):
        raise ValueError('actual private forwarding root missing or replaced')
    if [x['leaf'] for x in small['selected_callees']]!=[
            'world','begin_lifetime_update','begin_inbox_save'] or any(
        x['prefix']!=['sandproof_controller_service','controller','lifecycle_inbox']
        or x['opacity']!='Transparent' for x in small['selected_callees']):
        raise ValueError('roots no longer call the actual transparent methods')
    return dict(roots=small['roots'],actual_callees=small['selected_callees'],
        bounded_reader=small['reader_bounds'],summary_sha256=sha(output/'llbc-summary.json'),
        private_forwarders_not_exported_by_aeneas=True)


def generated_carriers(types,data):
    text=types.read_text()
    result={}
    for name,fields in data['complete_structure_fields'].items():
        start=text.index('structure '+name)
        end=text.find('\n/--',start)
        body=text[start:end if end>=0 else len(text)]
        found=re.findall(r'^  (\w+) :',body,re.M)
        if found!=fields:
            raise ValueError('missing or changed complete actual carrier: '+name)
        result[name]=found
    return result


def install(project,variant,data):
    installed={}
    for n in ['TypesExternal','FunsExternal']:
        path=project/GEN/(n+'.lean')
        shutil.copy2(ROOT/HERE/(n+'.lean.in'),path)
        installed[str(path.relative_to(project))]=sha(path)
    for n in data['common_templates']+data['variants'][variant]['templates']:
        path=project/PROOFS/(n+'.lean')
        shutil.copy2(ROOT/HERE/(n+'.lean.in'),path)
        installed[str(path.relative_to(project))]=sha(path)
    # Recheck the exact canonical math in this independent actual generation.
    # No shared Access file is written, and no mathematical proof text changes.
    n='SandboxModel/Access/Cloning.lean'
    text=(ROOT/n).read_text()
    if text.count('import SandproofNetwork.Funs')!=1 or text.count(
            'open Aeneas Aeneas.Std SandproofNetworkRust')!=1:
        raise ValueError('canonical clone provider imports changed')
    text=text.replace('import SandproofNetwork.Funs',
        'import SandproofStandardCreate.Funs\nimport SandboxModel.Lifecycle.Protocol.StandardCreateCloningDelta')
    text=text.replace('open Aeneas Aeneas.Std SandproofNetworkRust',
        'open Aeneas Aeneas.Std SandproofStandardCreateRust')
    (project/n).write_text(text)
    installed[n]=sha(project/n)
    lake=project/'lakefile.toml'
    if 'name = "SandproofStandardCreate"' in lake.read_text():
        raise ValueError('private standard Create view already registered')
    lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofStandardCreate"\n'
        'srcDir = "verification/aeneas/stage1-protocol/standard-create"\n')
    installed['lakefile.toml']=sha(lake)
    return installed
