"""Shared source derivation and exact-root admission for the private candidate."""
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-selection')
SOURCE = 'controller-service/src/controller/lifecycle_inbox.rs'
GEN = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
ROOT_NAME = 'sandproof_controller_service::controller::lifecycle_inbox::proof_create_attachment_selection'
ACTUAL = 'sandproof_controller_service::controller::lifecycle_inbox::create_attachment_selection'
ROOTS = [ROOT_NAME,*['crate::'+n for n in
    ('request_validate','replay_matches','accepted_validate','recorded_matches')]]
INCLUDE = ['sandproof_protocol','sandbox_controller_core','sandproof_controller_service']


def module(name,path):
    spec = importlib.util.spec_from_file_location(name,path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


parent = module('selection_original_load',ROOT/HERE.parent/'inbox-load/formal.py')
mapping = module('selection_fixed_mapping',ROOT/HERE.parent/'formal_runtime.py')
error_binding = module('selection_complete_error',ROOT/HERE.parent/'envelope-writer/binding.py')
journal = module('selection_original_journal',ROOT/HERE.parent/'inbox-lookup/journal_binding.py')
sha = parent.sha


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group,prefix in [('source_inputs',ROOT),('providers',ROOT),('templates',ROOT/HERE)]:
        for name,digest in data[group].items():
            if sha(prefix/name)!=digest:
                raise ValueError('reviewed '+group+' differs: '+name)
    for name,digest in data['drivers'].items():
        if sha(ROOT/HERE/name)!=digest:
            raise ValueError('reviewed driver differs: '+name)
    return data


def function_text(text, marker):
    if text.count(marker) != 1:
        raise ValueError('current production function is not unique')
    start = text.index(marker)
    opening = text.index('{', start)
    depth = 0
    for index in range(opening, len(text)):
        depth += (text[index] == '{') - (text[index] == '}')
        if depth == 0:
            return text[start:index + 1]
    raise ValueError('unclosed current production function')


def prepare(project,variant,data,command,folder,append_root=True):
    if sha(project/SOURCE)!=data['source_inputs'][SOURCE]:
        raise ValueError('original production source differs')
    # The reviewed candidate is now the unique production implementation.
    # Keep its original comparison fingerprint; never re-apply the old patch.
    source = (project/SOURCE).read_text()
    contract = data['current_production_function']
    body = function_text(source, contract['marker'])
    if hashlib.sha256(body.encode()).hexdigest() != contract['sha256']:
        raise ValueError('current production function differs from the proved adopted body')
    mutation = data['variants'][variant]['mutation']
    if mutation is not None:
        text = (project/SOURCE).read_text()
        body = function_text(text, contract['marker'])
        if body.count(mutation['before']) != 1:
            raise ValueError('real Rust mutation is not unique within the admitted function')
        start = text.index(contract['marker'])
        wrong_body = body.replace(mutation['before'], mutation['after'], 1)
        (project/SOURCE).write_text(text[:start] + wrong_body + text[start + len(body):])
    before_root = sha(project/SOURCE)
    if append_root:
        with (project/SOURCE).open('ab') as stream: stream.write((ROOT/HERE/'root.rs.in').read_bytes())
    return dict(path=SOURCE,original_sha256=data['source_inputs'][SOURCE],
        historical_candidate_sha256=data['candidate_source_sha256'],current_function_sha256=contract['sha256'],variant_sha256=before_root,
        derived_sha256=sha(project/SOURCE),mutation=mutation,production_modified=False,current_production_base=True,private_business_replacement=False,
        original_general_equivalence_proved=False)


def wrapper(project):
    paths = [Path.home(),mapping.tool_root(ROOT),mapping.package_root(ROOT)]
    for key in ('CARGO_HOME','RUSTUP_HOME','ELAN_HOME','SANDPROOF_PROOF_RUNTIME_ROOT'):
        if os.environ.get(key): paths.append(Path(os.environ[key]).resolve(strict=True))
    return ['bwrap','--ro-bind','/','/','--dev','/dev','--proc','/proc','--tmpfs','/tmp','--tmpfs','/home',
        *[a for p in dict.fromkeys(paths) for a in ('--ro-bind',str(p),str(p))],
        '--bind',str(project),str(mapping.VIRTUAL),'--chdir',str(mapping.VIRTUAL),
        '--setenv','CARGO_TARGET_DIR',str(mapping.VIRTUAL/'artifacts/extract-target'),
        '--setenv','CARGO_NET_OFFLINE','true','--setenv','CARGO_BUILD_JOBS','4',
        '--unsetenv','RUSTFLAGS','--unsetenv','CARGO_ENCODED_RUSTFLAGS','--']


def admit(raw,opaque):
    if opaque not in (['rusqlite::error::Error'],
                       ['rusqlite::error::Error','sandproof_protocol::ContractError']):
        raise ValueError('unreviewed opacity boundary')
    translated = raw['translated']
    options = translated['options']
    if (raw['has_errors'] or options['start_from']!=ROOTS or options['include']!=INCLUDE
            or options['opaque']!=opaque or options['exclude'] or options['skip_borrowck']
            or options['no_typecheck'] or options['monomorphize'] or options['remove_unused_clauses']
            or {t['key'] for t in translated['target_information']}!={'x86_64-unknown-linux-gnu'}):
        raise ValueError('incomplete or unreviewed capture')
    journal.admit(raw)
    functions = [f for f in translated['fun_decls'] if f]
    roots = [f for f in functions if f['item_meta']['name']==[{'Ident':[n,0]} for n in ROOT_NAME.split('::')]]
    if len(roots)!=1 or roots[0]['item_meta']['opacity']!='Transparent' or 'Structured' not in roots[0]['body']:
        raise ValueError('transparent forwarding root missing')
    calls,assignments = [],[]
    def walk(node):
        if isinstance(node,dict):
            if 'Call' in node: calls.append(node['Call']['call'])
            if 'Assign' in node: assignments.append(node['Assign'])
            for child in node.values(): walk(child)
        elif isinstance(node,list):
            for child in node: walk(child)
    walk(roots[0]['body'])
    if (len(calls)!=1 or len(assignments)!=2 or len(calls[0]['args'])!=2
            or calls[0]['dest']['kind']!={'Local':0}
            or roots[0]['body']['Structured']['locals']['arg_count']!=2):
        raise ValueError('forwarder must preserve one complete two-argument call')
    for argument,assignment,n in zip(calls[0]['args'],assignments,[1,2]):
        destination,expression = assignment
        if argument.get('Move',{}).get('kind')!=destination['kind'] or expression.get('Ref',{}).get('kind')!='Shared':
            raise ValueError('changed shared-borrow binding')
        projection = expression['Ref']['place']['kind']['Projection']
        if projection[0]['kind']!={'Local':n} or projection[1]!='Deref':
            raise ValueError('reordered or projected original inputs')
    actual = next(f for f in functions if f['def_id']==calls[0]['func']['Regular']['kind']['Fun'])
    if (actual['item_meta']['name']!=[{'Ident':[n,0]} for n in ACTUAL.split('::')]
            or actual['item_meta']['opacity']!='Transparent' or 'Structured' not in actual['body']):
        raise ValueError('actual business body missing, external or replaced')
    return dict(root=roots[0]['def_id'],actual=actual['def_id'],
        two_original_shared_borrows_in_order=True,call=calls[0])


def capture(project,folder,label,opaque,command,fixed):
    ir = Path('artifacts')/(label+'.llbc')
    argv = [str(mapping.VIRTUAL/'.tools/aeneas/charon/bin/charon'),'cargo','--preset=aeneas',
        '--error-on-warnings','--sysroot','default']
    for name in INCLUDE: argv += ['--include',name]
    for name in opaque: argv += ['--opaque',name]
    for name in ROOTS: argv += ['--start-from',name]
    argv += ['--dest-file',str(mapping.VIRTUAL/ir),'--','--manifest-path',
        'verification/experiments/stage1-protocol/inbox-record/Cargo.toml','--lib','--locked','--offline','--target',fixed['target']]
    command(project,folder,label+'-charon',wrapper(project)+argv)
    raw = json.loads((project/ir).read_text())
    binding = admit(raw,opaque)
    return raw,ir,binding


def admit_generated(generated,data,translated=None):
    if translated is None: translated = json.loads((generated/'translation.json').read_text())
    names = [{k:f[k] for k in ('lean_name','rust_name','is_opaque','lean_file')} for f in translated['functions']]
    if names!=data['generated_functions']:
        raise ValueError('actual transparent/external function closure differs')
    external = {f['lean_name'] for f in translated['functions'] if f['is_opaque']}
    if external!={'Bool.Insts.CoreCmpPartialEqBool.ne','core.option.Option.Insts.CoreCmpPartialEqOption.eq',
                  'alloc.string.String.Insts.CoreCmpPartialEqString.eq','alloc.vec.Vec.is_empty'}:
        raise ValueError('new library model or business opacity')
    name = 'SandproofStage1RequestRust.'+ACTUAL.replace('::','.')
    actual = [f for f in translated['functions'] if f['lean_name']==name]
    if len(actual)!=1 or actual[0]['is_opaque']:
        raise ValueError('generated business root missing or external')
    text = (generated/actual[0]['lean_file']).read_text()
    if (len(re.findall(r'^namespace SandproofStage1RequestRust$',text,re.M))!=1
            or len(re.findall(r'^end SandproofStage1RequestRust$',text,re.M))!=1
            or len(re.findall(r'^def\s+'+re.escape(name.removeprefix('SandproofStage1RequestRust.'))+r'\b',text,re.M))!=1):
        raise ValueError('actual generated function definition absent')
    return translated


def admission_rejections(raw,generated,data):
    from copy import deepcopy
    rejected = []
    for label in ('missing-business-root','external-business-root','swapped-original-inputs'):
        changed = deepcopy(raw)
        functions = changed['translated']['fun_decls']
        target = next(f for f in functions if f and f['item_meta']['name']==[{'Ident':[n,0]} for n in ACTUAL.split('::')])
        if label=='missing-business-root':
            functions[functions.index(target)] = None
        elif label=='external-business-root': target['item_meta']['opacity'] = 'Opaque'
        else:
            root = next(f for f in functions if f and f['item_meta']['name']==[{'Ident':[n,0]} for n in ROOT_NAME.split('::')])
            def swap(node):
                if isinstance(node,dict):
                    if 'Call' in node:
                        args = node['Call']['call']['args']
                        args[0],args[1] = args[1],args[0]
                    for child in node.values(): swap(child)
                elif isinstance(node,list):
                    for child in node: swap(child)
            swap(root['body'])
        try: admit(changed,['rusqlite::error::Error','sandproof_protocol::ContractError'])
        except (ValueError,StopIteration): rejected.append(label)
        else: raise ValueError('admission accepted '+label)
    actual = json.loads((generated/'translation.json').read_text())
    for label in ('missing-generated-root','external-generated-root'):
        changed = deepcopy(actual)
        name = 'SandproofStage1RequestRust.'+ACTUAL.replace('::','.')
        if label=='missing-generated-root':
            changed['functions'] = [f for f in changed['functions'] if f['lean_name']!=name]
        else: next(f for f in changed['functions'] if f['lean_name']==name)['is_opaque'] = True
        try: admit_generated(generated,data,changed)
        except ValueError: rejected.append(label)
        else: raise ValueError('admission accepted '+label)
    return rejected
