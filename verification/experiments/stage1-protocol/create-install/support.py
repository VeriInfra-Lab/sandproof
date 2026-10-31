"""Actual Create write roots; reuse fixed source, library and admission infrastructure."""
import importlib.util
import json
from pathlib import Path
import re

ROOT=Path(__file__).resolve().parents[4]
HERE=Path('verification/experiments/stage1-protocol/create-install')
SOURCE='controller-service/src/controller/lifecycle_inbox.rs'
GEN=Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
PROOFS=Path('SandboxModel/Lifecycle/Protocol')
PREFIX='sandproof_controller_service::controller::lifecycle_inbox::'
ROOT_NAME=PREFIX+'proof_create_attachment_install'
ACTUAL=PREFIX+'create_attachment_install'
ROOTS=[ROOT_NAME,PREFIX+'proof_create_attachment_selection',*['crate::'+n for n in ('request_validate','replay_matches','accepted_validate','recorded_matches')]]
INCLUDE=['sandproof_protocol','sandbox_controller_core','sandproof_controller_service']

def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
legacy=module('install_original_request_admission',ROOT/HERE.parent/'create-selection/support.py')
legacy.ROOTS=ROOTS
parent,mapping,error_binding,journal,sha=legacy.parent,legacy.mapping,legacy.error_binding,legacy.journal,legacy.sha
wrapper=legacy.wrapper

def inventory():
 data=json.loads((ROOT/HERE/'inventory.json').read_text())
 for group,prefix in [('source_inputs',ROOT),('providers',ROOT),('templates',ROOT/HERE),('drivers',ROOT/HERE)]:
  for name,digest in data[group].items():
   if sha(prefix/name)!=digest:raise ValueError('reviewed '+group+' differs: '+name)
 return data

def prepare(project,variant,data,command,folder,append_root=True):
 if sha(project/SOURCE)!=data['source_inputs'][SOURCE]:raise ValueError('actual production source differs')
 mutation=data['variants'][variant]['mutation']
 if mutation:
  path=project/SOURCE;text=path.read_text();start=text.index('fn create_attachment_install(');brace=text.index('{',start);depth=0;end=None
  for i in range(brace,len(text)):
   depth+=int(text[i]=='{')-int(text[i]=='}')
   if depth==0:end=i+1;break
  if end is None:raise ValueError('unclosed actual function')
  body=text[start:end]
  if body.count(mutation['before'])!=1:raise ValueError('mutation is not unique in actual business body')
  path.write_text(text[:start]+body.replace(mutation['before'],mutation['after'],1)+text[end:])
 before_root=sha(project/SOURCE)
 if append_root:
  with (project/SOURCE).open('ab') as stream:stream.write((ROOT/HERE/'root.rs.in').read_bytes())
 return dict(path=SOURCE,original_sha256=data['source_inputs'][SOURCE],variant_sha256=before_root,derived_sha256=sha(project/SOURCE),mutation=mutation,actual_production_base=True,private_business_replacement=False)

def admit(raw,opaque):
 selection=legacy.admit(raw,opaque)
 functions=[f for f in raw['translated']['fun_decls'] if f]
 def get(name):
  matches=[f for f in functions if f['item_meta']['name']==[{'Ident':[n,0]} for n in name.split('::')]]
  if len(matches)!=1 or matches[0]['item_meta']['opacity']!='Transparent' or 'Structured' not in matches[0]['body']:raise ValueError('transparent business body missing '+name)
  return matches[0]
 root,actual=get(ROOT_NAME),get(ACTUAL);get(PREFIX+'create_owner_present')
 calls,assignments=[],[]
 def walk(x):
  if isinstance(x,dict):
   if 'Call' in x:calls.append(x['Call']['call'])
   if 'Assign' in x:assignments.append(x['Assign'])
   for y in x.values():walk(y)
  elif isinstance(x,list):
   for y in x:walk(y)
 walk(root['body'])
 if len(calls)!=1 or len(assignments)!=5 or root['body']['Structured']['locals']['arg_count']!=4 or calls[0]['dest']['kind']!={'Local':0} or calls[0]['func']['Regular']['kind']['Fun']!=actual['def_id'] or len(calls[0]['args'])!=4:raise ValueError('changed complete four-argument forwarding call')
 unit,borrow,*copies=assignments
 if unit[0]['kind']!={'Local':0} or unit[1].get('Aggregate',[None,None])[1]!=[]:raise ValueError('nonunit root result initializer')
 destination,expression=borrow;reference=expression.get('Ref',{});projection=reference.get('place',{}).get('kind',{}).get('Projection')
 if calls[0]['args'][0].get('Move',{}).get('kind')!=destination['kind'] or reference.get('kind')!='TwoPhaseMut' or not projection or projection[0]['kind']!={'Local':1} or projection[1]!='Deref':raise ValueError('changed original full mutable Inbox borrow')
 for argument,(destination,expression),index in zip(calls[0]['args'][1:],copies,[2,3,4]):
  if argument.get('Move',{}).get('kind')!=destination['kind'] or expression.get('Use')!=[{'Copy':{'kind':{'Local':index},'ty':destination['ty']}},'Yes']:raise ValueError('changed original index or complete local binding')
 return dict(selection=selection,install=dict(root=root['def_id'],actual=actual['def_id'],original_full_mutable_borrow=True,original_s_e_and_complete_local_copies=True))

def capture(project,folder,label,opaque,command,fixed):
 ir=Path('artifacts')/(label+'.llbc');argv=[str(mapping.VIRTUAL/'.tools/aeneas/charon/bin/charon'),'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default']
 for name in INCLUDE:argv+=['--include',name]
 for name in opaque:argv+=['--opaque',name]
 for name in ROOTS:argv+=['--start-from',name]
 argv+=['--dest-file',str(mapping.VIRTUAL/ir),'--','--manifest-path','verification/experiments/stage1-protocol/inbox-record/Cargo.toml','--lib','--locked','--offline','--target',fixed['target']]
 command(project,folder,label+'-charon',wrapper(project)+argv);raw=json.loads((project/ir).read_text());return raw,ir,admit(raw,opaque)

def admit_generated(generated,data,translated=None):
 if translated is None:translated=json.loads((generated/'translation.json').read_text())
 names=[{k:f[k] for k in ('lean_name','rust_name','is_opaque','lean_file')} for f in translated['functions']]
 if names!=data['generated_functions']:raise ValueError('actual transparent/external function closure differs')
 external={f['lean_name'] for f in translated['functions'] if f['is_opaque']}
 if external!={'Bool.Insts.CoreCmpPartialEqBool.ne','core.option.Option.Insts.CoreCmpPartialEqOption.eq','alloc.string.String.Insts.CoreCmpPartialEqString.eq','alloc.string.String.Insts.CoreCloneClone.clone','alloc.vec.Vec.is_empty'}:raise ValueError('new library model or business opacity')
 for root in [ACTUAL,PREFIX+'create_owner_present',PREFIX+'create_attachment_selection']:
  name='SandproofStage1RequestRust.'+root.replace('::','.');matches=[f for f in translated['functions'] if f['lean_name']==name]
  if len(matches)!=1 or matches[0]['is_opaque']:raise ValueError('generated business root missing or external')
  text=(generated/matches[0]['lean_file']).read_text()
  if len(re.findall(r'^namespace SandproofStage1RequestRust$',text,re.M))!=1 or len(re.findall(r'^end SandproofStage1RequestRust$',text,re.M))!=1 or len(re.findall(r'^def\s+'+re.escape(name.removeprefix('SandproofStage1RequestRust.'))+r'\b',text,re.M))!=1:raise ValueError('actual generated body absent')
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
