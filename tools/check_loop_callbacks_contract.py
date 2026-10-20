"""Admit original finite shared-query roots and their explicit value/borrow scope.

This is source/artifact admission, not a proof of the compiler or a call graph.
The actual equality/refinement and wrong-version refutations are checked by Lean.
"""
import hashlib
import json
from pathlib import Path
import re
import sys

try:
    from .check_foundations_extraction import llbc_digest
except ImportError:
    from check_foundations_extraction import llbc_digest

ROOT = Path(__file__).resolve().parents[1]
CRATE = 'sandproof_loop_callbacks_extract'
NAMESPACE = 'SandboxModel.Foundations.LoopCallbacks.'
ROOT_NAMES = ('lookup', 'contains', 'lookup_u64', 'contains_u64')
METHOD = 'sandbox_controller_core::store::{sandbox_controller_core::store::Store<K, V>}::'
METHOD_NAMES = {METHOD+'get', METHOD+'contains_key'}
DEST = 'verification/aeneas/loop-callbacks'
LLBC = 'verification/aeneas/llbc/sandproof_loop_callbacks.llbc'
MODULES = ('Contracts', 'Loop', 'Refinement', 'Examples')
LAKE_EXTENSION = '\n[[lean_lib]]\nname = \"SandproofLoopCallbacks\"\nsrcDir = \"verification/aeneas/loop-callbacks\"\n'
SOURCE_IDS = {'store.shared-query', 'store.presence-after-query',
    'world.sandbox-shared-view', 'checkpoint.duplicate-row-decision',
    'runtime.effect-presence-decision'}
SOURCE_STATUS = {
    'store.shared-query': ('original-core-in-this-batch','V0'),
    'store.presence-after-query': ('original-core-in-this-batch','V0'),
    'world.sandbox-shared-view': ('source-binding-not-whole-caller-proof','V4'),
    'checkpoint.duplicate-row-decision': ('business-loop-not-proved-by-this-batch','V4'),
    'runtime.effect-presence-decision': ('business-decision-not-proved-by-this-batch','V4/V9'),
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def canonical(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True,
        separators=(',', ':')).encode()).hexdigest()


def foreign_surface(llbc):
    rows=[]
    for declaration in llbc['translated']['fun_decls']:
        if declaration is None or declaration['body']!='Opaque':
            continue
        name=declaration['item_meta']['name']
        rows.append(dict(name_hint='::'.join(part['Ident'][0] if 'Ident' in part
            else '{impl}' for part in name), name_sha256=canonical(name),
            signature_sha256=canonical([declaration['generics'],declaration['signature']])))
    return sorted(rows,key=lambda row:json.dumps(row,sort_keys=True))


def walk(value):
    if isinstance(value, dict):
        for key, child in value.items():
            yield key
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def check_extraction(root, llbc, inventory, env):
    if llbc.get('has_errors') is not False or inventory.get('crate') != CRATE:
        raise ValueError('Missing successful original Charon/Aeneas status')
    translated = llbc['translated']
    options = translated['options']
    if (translated['crate_name'] != CRATE
            or options['start_from'] != ['crate::'+name for name in ROOT_NAMES]
            or options['include'] != ['sandbox_controller_core']
            or options['dest_file'] != '../aeneas/llbc/sandproof_loop_callbacks.llbc'
            or options['preset'] != 'Aeneas'
            or any(options[key] is not False for key in
                   ('skip_borrowck','no_typecheck','no_normalize','no_reorder_decls'))
            or options['error_on_warnings'] is not True):
        raise ValueError('Original query roots, destination or compiler checks changed')
    if translated['target_information'] != env['target_information']:
        raise ValueError('Shared query target or pointer representation changed')
    if foreign_surface(llbc) != env['reviewed_opaque_signatures']:
        raise ValueError('Unreviewed original library function/signature')
    declarations = {d['def_id']:d for d in translated['fun_decls'] if d is not None}
    if len(declarations) != sum(d is not None for d in translated['fun_decls']):
        raise ValueError('Duplicate LLBC function identity')
    ordered = set()
    for row in translated['ordered_decls']:
        if 'Fun' not in row:
            continue
        group=row['Fun']
        if set(group)=={'NonRec'}:
            ordered.add(group['NonRec'])
        elif set(group)=={'Rec'}:
            ordered.update(group['Rec'])
        else:
            raise ValueError('Unreviewed function declaration ordering')
    functions=inventory['functions']
    if len(functions)!=8 or len({f['lean_name'] for f in functions})!=8:
        raise ValueError('Missing, duplicate or extra generated query function')
    parents=[f for f in functions if 'loop' not in f]
    expected={CRATE+'::'+name for name in ROOT_NAMES} | METHOD_NAMES
    if len(parents)!=6 or {f['rust_name'] for f in parents}!=expected:
        raise ValueError('Original delegate or Store method disappeared')
    loops=[f for f in functions if 'loop' in f]
    parent=next(f for f in parents if f['rust_name']==METHOD+'get')
    if (len(loops)!=2 or {f['loop']['is_body'] for f in loops}!={False,True}
            or any(f['rust_name']!=METHOD+'get' or f['def_id']!=parent['def_id']
                   or f.get('parent_lean_name')!=parent['lean_name']
                   or f['loop']['id']!=0 or f['loop']['pos']!=[0] for f in loops)):
        raise ValueError('Original loop/body roles are not bound to the original get')
    generated=(root/DEST/'SandproofLoopCallbacks/Funs.lean').read_text()
    for function in functions:
        declaration=declarations.get(function['def_id'],{})
        body=declaration.get('body')
        if (function['is_opaque'] or not isinstance(body,dict)
                or set(body)!={'Structured'} or function['def_id'] not in ordered
                or function['lean_file']!='SandproofLoopCallbacks/Funs.lean'):
            raise ValueError('Original owned query lacks a real structured body')
        name=function['lean_name'].removeprefix('SandproofLoopCallbacksRust.')
        if not re.search(r'^def '+re.escape(name)+r'\b',generated,re.M):
            raise ValueError('Generated function metadata has no actual Lean definition')
        if {'Drop','DropGlue','Error'} & set(walk(body)):
            raise ValueError('New destruction/error node requires separate admission')
    owned=[]
    for declaration in declarations.values():
        first=declaration['item_meta']['name'][0].get('Ident',[''])[0]
        if first==CRATE or first.startswith(('sandbox_','sandproof_')):
            owned.append(declaration['def_id'])
    if set(owned)!={f['def_id'] for f in parents}:
        raise ValueError('Project-owned body omitted or silently made external')
    expected_types=[{'lean_name':'SandproofLoopCallbacksRust.sandbox_controller_core.store.Store',
        'rust_name':'sandbox_controller_core::store::Store',
        'lean_file':'SandproofLoopCallbacks/Types.lean'}]
    actual_types=[{key:t[key] for key in expected_types[0]} for t in inventory['types']]
    if actual_types!=expected_types:
        raise ValueError('Original shared Store representation changed')
    return dict(original_functions=6,loop_artifacts=2,custom_drop_nodes=0)


def check(root=ROOT):
    root=Path(root).resolve()
    env=json.loads((root/'verification/loop-callbacks-environment.json').read_text())
    if env['proof_kind']!='original-rust-shared-query-with-explicit-lawful-equality':
        raise ValueError('Shared query claim must name original Rust and its comparison condition')
    lines=(root/'verification/loop-callbacks-inputs.sha256').read_text().splitlines()
    paths=[]
    for line in lines:
        expected,name=line.split('  ',1);paths.append(name);p=(root/name).resolve()
        if (not re.fullmatch('[0-9a-f]{64}',expected) or not p.is_relative_to(root)
                or digest(p)!=expected):
            raise ValueError('Frozen shared query input changed: '+name)
    if len(paths)!=len(set(paths)) or set(paths)!=set(env['inputs']):
        raise ValueError('Missing, extra or duplicate shared query input')
    required={'Cargo.toml','Cargo.lock','controller-core/Cargo.toml',
        'lakefile.toml','lake-loop-callbacks.toml','lake-manifest.json','lean-toolchain',
        'tools/check_foundations_extraction.py','tools/check_first_chain_extraction.py',
        'tools/lean_session.py','scripts/lean-mcp-server.sh','tools/acceptance_receipts.py',
        'verification/extract-loop-callbacks/Cargo.toml',
        'verification/extract-loop-callbacks/Cargo.lock',
        'verification/extract-loop-callbacks/src/lib.rs'}
    required.update(str(p.relative_to(root)) for p in (root/'controller-core/src').rglob('*.rs'))
    for parent in env['parent_inputs']:
        required.add(parent)
        required.update(line.split('  ',1)[1] for line in (root/parent).read_text().splitlines())
    if not required<=set(paths):
        raise ValueError('Missing original source/import/tool/parent input closure')
    base_config=(root/'lakefile.toml').read_text().rstrip()
    view=(root/'lake-loop-callbacks.toml').read_text().rstrip()
    expected=base_config if 'name = "SandproofLoopCallbacks"' in base_config else (base_config+'\n'+LAKE_EXTENSION).rstrip()
    if view!=expected:
        raise ValueError('Independent query Lake view must retain all original libraries')
    source=json.loads((root/'verification/experiments/loop-callbacks/source-map.json').read_text())
    if source['kind']!='finite-reviewed-source-neighborhoods-not-complete-call-graph':
        raise ValueError('Finite query source map is not a whole-program call graph')
    if {x['id'] for x in source['use_points']}!=SOURCE_IDS or len(source['use_points'])!=5:
        raise ValueError('Finite source neighborhood was dropped or duplicated')
    for name,expected in source['production_sources'].items():
        if name not in paths or digest(root/name)!=expected:
            raise ValueError('Original source or classified neighborhood changed: '+name)
    for point in source['use_points']:
        if (point['status'],point['provider'])!=SOURCE_STATUS[point['id']]:
            raise ValueError('Query caller business obligation cannot become trusted wiring')
        text=(root/point['file']).read_text();start,end=point['range']
        if text.count(start)!=1 or text.count(end)!=1 or not point['provider'] or not point['conditions']:
            raise ValueError('Ambiguous source range or missing condition/provider')
        fragment=text[text.index(start):text.index(end,text.index(start)+len(start))]
        positions=[]
        for anchor in point['anchors']:
            if fragment.count(anchor)!=1:
                raise ValueError('Original query/caller source anchor changed: '+point['id'])
            positions.append(fragment.index(anchor))
        if positions!=sorted(positions):
            raise ValueError('Original query/decision order changed: '+point['id'])
    script=(root/'scripts/regenerate-loop-callbacks-lean.sh').read_text()
    if tuple(re.findall(r'--start-from crate::([a-z0-9_]+)',script))!=ROOT_NAMES:
        raise ValueError('Generator roots differ from the four original delegates')
    delegates=(root/'verification/extract-loop-callbacks/src/lib.rs').read_text()
    if tuple(re.findall(r'pub fn ([a-z0-9_]+)',delegates))!=ROOT_NAMES:
        raise ValueError('Public validation delegate root inventory changed')
    names=[]
    for module in MODULES:
        text=(root/f'SandboxModel/Foundations/LoopCallbacks/{module}.lean').read_text()
        if re.search(r'\b(?:sorry|admit|axiom|native_decide)\b',text):
            raise ValueError('Unapproved proof placeholder, assumption or evaluator')
        names += [NAMESPACE+n for n in re.findall(r'^theorem (\w+)',text,re.M)]
    expected_variants={'skip-first':('QueryContract',2),'invert-presence':('MembershipContract',3)}
    if set(env['wrong_variants'])!=set(expected_variants):
        raise ValueError('Missing or unreviewed wrong Rust variant')
    for variant,(contract,count) in expected_variants.items():
        info=env['wrong_variants'][variant];template=(root/info['template']).read_text()
        targets=re.findall(r'^#audit_controller_proof (\S+)$',template,re.M)
        if (info['template'] not in paths or info['contract']!=contract
                or '¬ '+contract not in template or len(targets)!=count
                or len(set(targets))!=count or targets!=info['theorems']
                or re.search(r'\b(?:sorry|admit|axiom|native_decide)\b',template)):
            raise ValueError('Same-contract wrong-version proof template is incomplete')
    audit=(root/'SandboxModel/Foundations/LoopCallbacks/Audit.lean').read_text()
    audited=re.findall(r'^#audit_controller_proof (\S+)$',audit,re.M)
    if (len(names)!=len(set(names)) or sorted(names)!=sorted(env['theorems'])
            or sorted(audited)!=sorted(names)):
        raise ValueError('Missing, duplicate or extra query theorem/audit target')
    files={str(p.relative_to(root/DEST)) for p in (root/DEST).rglob('*') if p.is_file()}
    if files!={'SandproofLoopCallbacks/Types.lean','SandproofLoopCallbacks/Funs.lean','translation.json'}:
        raise ValueError('Missing or unreviewed generated query file/template')
    llbc=json.loads((root/LLBC).read_text());inventory=json.loads((root/DEST/'translation.json').read_text())
    check_extraction(root,llbc,inventory,env)
    if llbc_digest(llbc)!=env['llbc_digest']:
        raise ValueError('Complete original LLBC changed')
    return env


if __name__=='__main__':
    try:
        result=check()
    except (OSError,ValueError,KeyError,TypeError,StopIteration) as error:
        sys.exit(str(error))
    print(f"Original shared queries: {len(result['theorems'])} audited theorem targets; four actual delegates.")
    print('Valid shared value borrows and lawful equality required; no heap snapshot, Drop or caller-business claim.')
