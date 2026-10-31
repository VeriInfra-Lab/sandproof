"""Exact current Rust helper, existing per-symbol models and full error carriers."""
from pathlib import Path
import importlib.util
import json
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-deadline')
SOURCE = 'controller-service/src/controller/lifecycle_inbox.rs'
LIB_ROOT = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'
GEN = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
PREFIX = 'sandproof_controller_service::controller::lifecycle_inbox::'
ROOTS = ['crate::'+n for n in ('request_validate','replay_matches','accepted_validate','recorded_matches','proof_deadline_cast')]+[PREFIX+'proof_create_attachment_deadline']


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


load = module('deadline_existing_load_tools', ROOT/HERE.parent/'inbox-load/formal.py')
runtime = module('deadline_fixed_runtime', ROOT/HERE.parent/'formal_runtime.py')
existing = module('deadline_existing_value_bindings', ROOT/HERE.parent/'inbox-validation/library_binding.py')
capacity = module('deadline_existing_wrapper', ROOT/HERE.parent/'create-load-capacity/support.py')
error_binding = module('deadline_complete_original_error', ROOT/HERE.parent/'envelope-writer/binding.py')
sha, wrapper = load.sha, capacity.wrapper
INCLUDE = list(load.INCLUDE)+['core::convert::num::*::try_from','core::num::error','core::result::*::map_err']
SYMBOLS = ['Bool.Insts.CoreCmpPartialEqBool.ne','alloc.string.String.Insts.CoreCmpPartialEqString.eq','alloc.vec.Vec.is_empty','core.option.Option.Insts.CoreCmpPartialEqOption.eq']


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group,prefix in [('source_inputs',ROOT),('providers',ROOT),('templates',ROOT/HERE),('drivers',ROOT/HERE)]:
        for n,h in data[group].items():
            if sha(prefix/n)!=h:
                raise ValueError('reviewed deadline '+group+' changed: '+n)
    existing.check(ROOT)
    return data


def library_review(data):
    fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
    reviewed = {}
    for kind,toolchain in [('production','1.97.0'),('extraction',fixed['extraction_frontend'])]:
        sysroot = Path(subprocess.check_output(['rustup','run',toolchain,'rustc','--print','sysroot'],text=True).strip())
        folder = sysroot/'lib/rustlib/src/rust/library/core/src'
        values = {n:sha(folder/n) for n in ('convert/num.rs','num/error.rs','result.rs')}
        if values!=data['standard_library_sources'][kind]:
            raise ValueError('reviewed standard library changed: '+kind)
        for relative,fragment in data['selected_std_fragments'].items():
            if fragment not in (folder/relative).read_text():
                raise ValueError('reviewed actual Std body changed: '+kind+'/'+relative)
        reviewed[kind] = dict(toolchain=toolchain,sysroot=str(sysroot),sources=values)
    for n,h in data['fixed_model_sources'].items():
        if sha(ROOT/'.tools/aeneas/backends/lean/Aeneas'/n)!=h:
            raise ValueError('fixed backend model changed: '+n)
    return reviewed


def mutate(project,mutation):
    path=project/SOURCE;before=sha(path)
    if mutation is not None:
        text=path.read_text();marker='pub(super) fn create_attachment_deadline(value: u64)';start=text.index(marker);end=text.index('\n}',start)+2
        body=text[start:end]
        if body.count(mutation['before'])!=1:
            raise ValueError('actual helper mutation is not unique')
        path.write_text(text[:start]+body.replace(mutation['before'],mutation['after'],1)+text[end:])
    return dict(source=SOURCE,original_sha256=before,variant_sha256=sha(path),mutation=mutation,actual_production_base=True,private_business_replacement=False)


def install(project,variant,data):
    installed={}
    for leaf,provider in [('TypesExternal.lean',HERE.parent/'inbox-record/TypesExternal.lean.in'),('SharedValueBindings.lean',HERE.parent/'inbox-validation/SharedValueBindings.lean.in')]:
        target=project/GEN/leaf;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/provider,target);installed[str(GEN/leaf)]=sha(target)
    provider=(ROOT/'verification/aeneas/stage1-protocol/request/SandproofStage1Request/FunsExternal.lean').read_text()
    text='import SandproofStage1Request.Types\nimport SandproofStage1Request.SharedValueBindings\nopen Aeneas Aeneas.Std\nnamespace SandproofStage1RequestRust\n\n'+(ROOT/HERE/'ErrorCarrier.lean.in').read_text()+'\n'+'\n\n'.join(existing.fragment(provider,n).strip() for n in SYMBOLS)+'\n\nend SandproofStage1RequestRust\n'
    target=project/GEN/'FunsExternal.lean';target.write_text(text);installed[str(GEN/'FunsExternal.lean')]=sha(target)
    for stem in data['common_templates']+data['variants'][variant]['templates']:
        target=project/PROOFS/(stem+'.lean');shutil.copy2(ROOT/HERE/(stem+'.lean.in'),target);installed[str(PROOFS/target.name)]=sha(target)
    return installed
