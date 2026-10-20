"""Admit the original exclusive-borrow root and finite conditional guard rules.

This is a source/AST/library admission check, not a proof of stdlib or the
shared-lock implementation. No substring census is a complete call graph.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess

try:
    from .check_task_contract import ROOT, digest
    from .check_foundations_extraction import llbc_digest
except ImportError:
    from check_task_contract import ROOT, digest
    from check_foundations_extraction import llbc_digest

NAMESPACE = 'SandboxModel.Boundaries.StdMutexContract.'
SOURCE_FILES = {'controller-service/src/application/diagnostics.rs',
                'sandbox-guest-service/src/task_accounting.rs',
                'sandbox-guest-runtime/src/pty.rs'}
EXPECTED_SITES = {'serial.new','serial.get-mut','serial.lock','serial.deref',
                  'serial.deref-mut','serial.drop','serial.business-callback',
                  'meter.lock','meter.business-policy','pty.new','pty.descriptor',
                  'pty.close','pty.business-policy'}
BUSINESS_SITES = {'serial.business-callback','meter.business-policy','pty.business-policy'}
LIBRARY_FILES = {'std/src/sync/poison/mutex.rs','std/src/sync/poison.rs',
                 'std/src/sys/sync/mutex/futex.rs'}


def walk(value):
    yield value
    if isinstance(value, dict):
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def extraction(env, llbc, inventory):
    if llbc.get('has_errors') is not False:
        raise ValueError('Original standard Mutex extraction has errors')
    if llbc_digest(llbc) != env['extraction']['normalized_sha256']:
        raise ValueError('Full standard Mutex LLBC changed; no ordered AST fields may be removed')
    translated = llbc['translated']
    options = translated['options']
    if ({t['key'] for t in translated['target_information']} != {env['target']}
            or options['start_from'] != ['crate::application::diagnostics::_::get_mut']
            or options['opaque'] != ['std::sync', 'tokio', 'rustix']
            or options['include'] != ['sandproof_controller_service']
            or any(options[k] for k in ('skip_borrowck','no_typecheck','extract_opaque_bodies'))):
        raise ValueError('Unreviewed standard Mutex extraction configuration')
    functions = inventory['functions']
    actual = {(f['def_id'], f['is_local'], f['is_opaque']) for f in functions}
    if len(functions) != 2 or actual != {(11,True,False),(20,False,True)}:
        raise ValueError('Missing original owned get_mut body or unreviewed external function')
    if (len(inventory['types']) != 3 or
            {t['def_id'] for t in inventory['types']} != {1,3,5} or
            any(inventory[k] for k in ('globals','trait_decls','trait_impls'))):
        raise ValueError('Unreviewed standard Mutex translated inventory')
    local = [f for f in translated['fun_decls'] if f and f['item_meta']['is_local']]
    if len(local) != 1 or local[0]['def_id'] != 11:
        raise ValueError('Original local root disappeared or was replaced')
    root = local[0]
    body = root['body']
    if not isinstance(body, dict) or set(body) != {'Structured'}:
        raise ValueError('Original get_mut body must remain transparent Structured LLBC')
    calls = [n['Call']['call'] for n in walk(body)
             if isinstance(n,dict) and set(n)=={'Call'}]
    if len(calls) != 1 or calls[0]['func']['Regular']['kind'] != {'Fun':20}:
        raise ValueError('Original wrapper must call the original standard Mutex get_mut once')
    if calls[0]['dest']['kind'] != {'Local':0} or len(calls[0]['args']) != 1:
        raise ValueError('Original get_mut argument/return connection changed')
    if not any(isinstance(n,dict) and n.get('Field') == [None,0] for n in walk(body)):
        raise ValueError('Missing original SerialMutex field-zero borrow')
    if not any(isinstance(n,dict) and n.get('kind') == 'Return' for n in walk(body)):
        raise ValueError('Missing original get_mut return')
    foreign = next(f for f in translated['fun_decls'] if f and f['def_id']==20)
    if foreign['body'] != 'Opaque' or foreign['item_meta']['is_local']:
        raise ValueError('Standard library is an explicit boundary, not an owned Rust proof')


def check(root=ROOT, sysroots=None):
    env = json.loads((root/'verification/std-mutex-environment.json').read_text())
    if env['proof_kind'] != 'conditional-guard-model-plus-original-synchronous-get-mut':
        raise ValueError('Standard Mutex guard model is not full shared-lock Rust refinement')
    entries = (root/'verification/std-mutex-inputs.sha256').read_text().splitlines()
    paths = [line.split('  ',1)[1] for line in entries]
    if len(paths) != len(set(paths)) or sorted(paths) != sorted(env['inputs']):
        raise ValueError('Missing or duplicate standard Mutex input')
    for line in entries:
        expected, name = line.split('  ',1)
        path = (root/name).resolve()
        if not path.is_relative_to(root.resolve()) or digest(path) != expected:
            raise ValueError('Frozen standard Mutex input changed: '+name)
    if set(env['production_sources']) != SOURCE_FILES or any(
            digest(root/n) != value for n,value in env['production_sources'].items()):
        raise ValueError('Original standard Mutex production source changed')
    if set(env['standard_library']) != {'production','frontend'}:
        raise ValueError('Missing production or frontend standard library')
    for role, library in env['standard_library'].items():
        if set(library['sources']) != LIBRARY_FILES:
            raise ValueError('Missing reviewed standard Mutex library source')
        sysroot = Path(sysroots[role]) if sysroots is not None else Path(subprocess.check_output(
            ['rustup','run',library['toolchain'],'rustc','--print','sysroot'],text=True).strip())
        for name, expected in library['sources'].items():
            if digest(sysroot/'lib/rustlib/src/rust/library'/name) != expected:
                raise ValueError('Reviewed standard Mutex library source changed: '+role+'/'+name)
    theorems = []
    for module in env['model_modules']:
        source = (root/('SandboxModel/Boundaries/StdMutex/'+module+'.lean')).read_text()
        if re.search(r'\b(?:sorry|admit|axiom)\b',source):
            raise ValueError('Unapproved proof placeholder or assumption')
        suffix = '' if module=='Guard' else ('Borrow.' if module in ('BorrowModel','GetMut') else 'Bridge.')
        theorems.extend(NAMESPACE+suffix+n for n in re.findall(r'^theorem (\w+)',source,re.M))
    if len(theorems)!=46 or sorted(theorems)!=sorted(env['theorems']) or len(set(theorems))!=len(theorems):
        raise ValueError('Standard Mutex theorem inventory changed')
    audit = (root/'SandboxModel/Boundaries/StdMutex/Audit.lean').read_text()
    if sorted(re.findall(r'^#audit_controller_proof (\S+)$',audit,re.M))!=sorted(theorems):
        raise ValueError('Missing, duplicate or extra standard Mutex audit')
    ids = [s['id'] for s in env['use_points']]
    if len(set(ids))!=len(ids) or set(ids)!=EXPECTED_SITES:
        raise ValueError('Finite standard Mutex use-point registration changed')
    for site in env['use_points']:
        if (site['file'] not in paths or not all(site.get(k) for k in
                ('caller_obligations','preconditions','status','original_object'))):
            raise ValueError('Standard Mutex use point lacks original object, condition or provider')
        if any(n not in theorems for n in site['public_results']):
            raise ValueError('Unknown standard Mutex public theorem')
        if site['id'] in BUSINESS_SITES:
            if site['status']!='business-obligation' or site['public_results']:
                raise ValueError('Standard Mutex business decision cannot be trusted wiring')
        elif not site['public_results'] or site['status'] != (
                'extracted-synchronous' if site['id']=='serial.get-mut' else 'conditional-mechanical'):
            raise ValueError('Required public standard Mutex rule cannot be moved to business')
        text = (root/site['file']).read_text()
        start,end = site['range']
        if text.count(start)!=1 or text.count(end)!=1:
            raise ValueError('Ambiguous standard Mutex function range: '+site['id'])
        lower = text.index(start); upper = text.index(end,lower+len(start))
        fragment = text[lower:upper];positions=[]
        for anchor in site['anchors']:
            if fragment.count(anchor)!=1:
                raise ValueError('Reviewed standard Mutex call anchor changed: '+site['id'])
            positions.append(fragment.index(anchor))
        if site['ordered'] and positions!=sorted(positions):
            raise ValueError('Original standard Mutex call/release order changed: '+site['id'])
    extraction(env,json.loads((root/env['extraction']['llbc']).read_text()),
               json.loads((root/env['extraction']['inventory']).read_text()))
    return env


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    env=check()
    print(f"Standard Mutex: {len(env['theorems'])} audited roots, thirteen finite source groups.")
    print('Only original SerialMutex::get_mut is extracted; shared guard rules remain conditional A6/B2.')
