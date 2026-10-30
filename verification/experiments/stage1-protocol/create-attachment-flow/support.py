"""Actual Create root, existing complete models, and explicit finite providers."""
from pathlib import Path
import hashlib
import importlib.util
import json
import re
import shutil

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-attachment-flow')
SOURCE = 'controller-service/src/controller/lifecycle_inbox.rs'
CODEC = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
GEN = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
GRAPHIC = Path('verification/aeneas/stage1-protocol/graphic/SandproofStage1Graphic')
PATH_GEN = Path('verification/aeneas/stage1-protocol/unix-path/SandproofUnixPath')
PREFIX = 'sandproof_controller_service::controller::lifecycle_inbox::'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


load = module('create_flow_original_load_tools', ROOT/HERE.parent/'inbox-load/formal.py')
runtime = module('create_flow_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
capacity = module('create_flow_actual_object_binding', ROOT/HERE.parent/'create-load-capacity/support.py')
owner = module('create_flow_original_owner_bindings', ROOT/HERE.parent/'create-owner-check/support.py')
error_binding = module('create_flow_full_original_error', ROOT/HERE.parent/'envelope-writer/binding.py')
binding = module('create_flow_full_parent_forwarding', ROOT/HERE/'binding.py')
path_binding = module('create_flow_complete_path_map', ROOT/HERE/'path_binding.py')
sha = load.sha
INCLUDE = list(load.INCLUDE)+['core::convert::num::*::try_from','core::num::error',
    'core::result::*::map_err','std::path::PathBuf','std::ffi::os_str::OsString','std::sys::os_str::bytes::Buf']
ROOTS = list(load.ROOTS)+[PREFIX+'proof_create_attachment_install',PREFIX+'proof_create_attachment_selection']+list(owner.ROOTS)+[
    PREFIX+'proof_attach_create_using',PREFIX+'codec::proof_inbox_save_using',
    PREFIX+'proof_create_attachment_deadline','crate::proof_deadline_cast']


def inventory():
    data=json.loads((ROOT/HERE/'inventory.json').read_text())
    for group,prefix in [('source_inputs',ROOT),('providers',ROOT),('templates',ROOT/HERE),('drivers',ROOT/HERE)]:
        for name,digest in data[group].items():
            if sha(prefix/name)!=digest:
                raise ValueError('reviewed Create '+group+' changed: '+name)
    for name,record in data['external_function_provenance']['functions'].items():
        provider=ROOT/record['provider']
        if sha(provider)!=record['provider_sha256']:
            raise ValueError('existing external provider changed: '+name)
    return data


def function_range(text,marker):
    start=text.index(marker);opening=text.index('{',start);depth=0
    for index in range(opening,len(text)):
        depth+=(text[index]=='{')-(text[index]=='}')
        if depth==0:return start,index+1
    raise ValueError('unclosed actual function')


def mutate(project,mutation):
    path=project/SOURCE;before=sha(path)
    if mutation is not None:
        text=path.read_text();start,end=function_range(text,'pub(super) fn attach_create_using<')
        body=text[start:end]
        if body.count(mutation['before'])!=1:
            raise ValueError('actual Create mutation is not unique')
        path.write_text(text[:start]+body.replace(mutation['before'],mutation['after'],1)+text[end:])
    return dict(source=SOURCE,original_sha256=before,variant_sha256=sha(path),mutation=mutation,
        actual_production_base=True,private_business_replacement=False)


def prepare(project,variant,data):
    derivation=mutate(project,data['variants'][variant]['mutation']);changed={SOURCE:sha(project/SOURCE)}
    append=load.native.preparation.views.append
    additions=[
        ('verification/experiments/stage1-protocol/inbox-record/lib.rs','create-deadline/stdlib-root.rs.in'),
        ('controller-service/src/controller/lifecycle_inbox/model.rs','inbox-validation/generic-root.rs.in'),
        ('controller-service/src/capture.rs','inbox-validation/primitive-roots.rs.in'),
        ('verification/experiments/stage1-protocol/inbox-record/lib.rs','terminal-binding/roots.rs.in'),
        (CODEC,'inbox-load/generic-root.rs.in'),(CODEC,'inbox-save/generic-root.rs.in'),
        (SOURCE,'create-install/root.rs.in'),(SOURCE,'create-owner-check/root.rs.in'),
        (SOURCE,'create-deadline/root.rs.in'),(SOURCE,'create-attachment-flow/root.rs.in')]
    for name,provider in additions:
        append(project,name,(ROOT/HERE.parent/provider).read_text());changed[name]=sha(project/name)
    name='verification/experiments/stage1-protocol/inbox-record/lib.rs'
    append(project,name,json.loads((ROOT/HERE.parent/'inbox-renewal-session/fragments.json').read_text())['renewal_roots']);changed[name]=sha(project/name)
    # Materialize the unchanged published standalone Std forwarding crate at its
    # original virtual path; no shared Cargo/Lake file is edited.
    for name,digest in data['path_provider']['source_files'].items():
        source=ROOT/HERE/'reuse/path'/Path(name).name
        if sha(source)!=digest:raise ValueError('fixed Std path root differs')
        target=project/'verification/experiments/stage1-protocol/unix-path'/name
        target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(source,target)
        changed[str(target.relative_to(project))]=digest
    return derivation,changed


def materialize(project,variant,data):
    installed={}
    for name,record in data['materialization'].items():
        kind=record['kind']
        if kind=='raw-tool-generated':continue
        if name.endswith('/CreateAttachmentFlow.lean') and variant!='actual':continue
        if name.endswith('/CreateAttachmentFlowWitnesses.lean') and variant!='actual':continue
        if kind=='unchanged-repository-provider':source=ROOT/record['path'];value=source.read_bytes()
        elif kind=='existing-template-provider':
            source=ROOT/record['path'];value=source.read_bytes()
            if hashlib.sha256(value).hexdigest()!=record['original_sha256']:raise ValueError('existing proof changed')
            if record['transformation']=='only-existing-library-namespace':
                value=value.replace(b'SandproofStage1OwnerMapping',b'SandproofStage1Request')
            elif record['transformation']=='reviewed-store23-to24-literals':
                value,count=re.subn(rb'\b23\b',b'24',value)
                if count!=record['literal_occurrences']:raise ValueError('store literal adaptation changed')
            elif record['transformation']!='identity':raise ValueError('unreviewed proof adaptation')
        else:value=(ROOT/HERE/record['path']).read_bytes()
        if hashlib.sha256(value).hexdigest()!=record['sha256']:raise ValueError('materialized proof/model changed: '+name)
        target=project/name;target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(value);installed[name]=record['sha256']
    stems=['CreateAttachmentCounterValues']+data['variants'][variant]['extra_templates']
    for stem in stems:
        target=project/PROOFS/(stem+'.lean');shutil.copy2(ROOT/HERE/(stem+'.lean.in'),target);installed[str(target.relative_to(project))]=sha(target)
    if variant!='actual' and (project/PROOFS/'CreateAttachmentFlow.lean').exists():
        raise ValueError('wrong view imported correct parent correspondence')
    lake=project/'lakefile.toml';text=lake.read_text()
    for name,folder in [('SandproofStorage','verification/aeneas/storage'),('SandproofStage1Graphic','verification/aeneas/stage1-protocol/graphic'),('SandproofUnixPath','verification/aeneas/stage1-protocol/unix-path')]:
        if 'name = "'+name+'"' not in text:text+='\n[[lean_lib]]\nname = "'+name+'"\nsrcDir = "'+folder+'"\n'
    lake.write_text(text);installed['lakefile.toml']=sha(lake)
    return installed


def wrapper(project):
    return capacity.wrapper(project)
