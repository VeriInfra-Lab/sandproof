"""Admit the finite Condvar source bindings; this is not Rust refinement."""
import argparse
import json
from pathlib import Path
import re
import subprocess

try:
    from .check_task_contract import ROOT, digest
except ImportError:
    from check_task_contract import ROOT, digest

NAMESPACE = 'SandboxModel.Boundaries.CondvarContract.'
SOURCE_FILES = {'sandbox-runtime/src/child_process/reaper.rs'}
LIBRARY_FILES = {'std/src/sync/poison/condvar.rs', 'std/src/sys/sync/condvar/futex.rs'}
BUSINESS_SITES = {'run.business-predicate', 'run.poison-policy'}
EXPECTED_SITES = {'reaper.construct', 'retain.notify', 'run.original-lock', 'run.wait',
                  'run.wait-timeout', *BUSINESS_SITES}


def check(root=ROOT, sysroots=None):
    env = json.loads((root / 'verification/condvar-environment.json').read_text())
    if env['proof_kind'] != 'conditional-library-event-model-with-frozen-production-wiring':
        raise ValueError('Condvar model cannot claim full Reaper Rust refinement')
    entries = (root / 'verification/condvar-inputs.sha256').read_text().splitlines()
    paths = [line.split('  ', 1)[1] for line in entries]
    if len(paths) != len(set(paths)) or sorted(paths) != sorted(env['inputs']):
        raise ValueError('Missing or duplicate Condvar input')
    for line in entries:
        expected, name = line.split('  ', 1)
        path = (root / name).resolve()
        if not path.is_relative_to(root.resolve()) or digest(path) != expected:
            raise ValueError('Frozen Condvar input changed: ' + name)
    if set(env['production_sources']) != SOURCE_FILES or any(
            digest(root / name) != expected for name, expected in env['production_sources'].items()):
        raise ValueError('Original Condvar production source changed')
    if set(env['standard_library']) != {'production', 'frontend'}:
        raise ValueError('Missing Condvar production or frontend standard library')
    for role, library in env['standard_library'].items():
        if set(library['sources']) != LIBRARY_FILES:
            raise ValueError('Missing reviewed Condvar library source')
        sysroot = Path(sysroots[role]) if sysroots is not None else Path(subprocess.check_output(
            ['rustup', 'run', library['toolchain'], 'rustc', '--print', 'sysroot'], text=True).strip())
        for name, expected in library['sources'].items():
            if digest(sysroot / 'lib/rustlib/src/rust/library' / name) != expected:
                raise ValueError('Reviewed Condvar library source changed: ' + role + '/' + name)
    theorems = []
    if env['model_modules'] != ['Core', 'Protocol', 'Examples']:
        raise ValueError('Condvar module scope changed')
    for module in env['model_modules']:
        source = (root / f'SandboxModel/Boundaries/Condvar/{module}.lean').read_text()
        if re.search(r'\b(?:sorry|admit|axiom)\b', source):
            raise ValueError('Unapproved Condvar placeholder or assumption')
        suffix = '' if module == 'Core' else 'Waiting.'
        theorems.extend(NAMESPACE + suffix + name for name in re.findall(r'^theorem (\w+)', source, re.M))
    if len(theorems) != 46 or len(set(theorems)) != 46 or sorted(theorems) != sorted(env['theorems']):
        raise ValueError('Condvar theorem inventory changed')
    audit = (root / 'SandboxModel/Boundaries/Condvar/Audit.lean').read_text()
    if sorted(re.findall(r'^#audit_controller_proof (\S+)$', audit, re.M)) != sorted(theorems):
        raise ValueError('Missing, duplicate or extra Condvar audit')
    ids = [site['id'] for site in env['use_points']]
    if len(ids) != len(set(ids)) or set(ids) != EXPECTED_SITES:
        raise ValueError('Finite Condvar use-point registration changed')
    for site in env['use_points']:
        if site['file'] not in SOURCE_FILES or not all(site.get(key) for key in
                ('caller_obligations', 'preconditions', 'status', 'original_object')):
            raise ValueError('Condvar use point lacks original object, condition or provider')
        if any(name not in theorems for name in site['public_results']):
            raise ValueError('Unknown Condvar public theorem')
        if site['id'] in BUSINESS_SITES:
            if site['status'] != 'business-obligation' or site['public_results']:
                raise ValueError('Condvar business decision cannot be trusted wiring')
        elif site['status'] != 'conditional-mechanical' or not site['public_results']:
            raise ValueError('Required Condvar public rule cannot be moved to business')
        text = (root / site['file']).read_text()
        start, end = site['range']
        if text.count(start) != 1 or text.count(end) != 1:
            raise ValueError('Ambiguous Condvar source range: ' + site['id'])
        lower = text.index(start)
        fragment = text[lower:text.index(end, lower + len(start))]
        positions = []
        for anchor in site['anchors']:
            if fragment.count(anchor) != 1:
                raise ValueError('Reviewed Condvar call anchor changed: ' + site['id'])
            positions.append(fragment.index(anchor))
        if site['ordered'] and positions != sorted(positions):
            raise ValueError('Original Condvar call order changed: ' + site['id'])
    return env


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    env = check()
    print(f"Condvar: {len(env['theorems'])} conditional model declarations, seven finite source groups.")
    print('Original shared-lock Reaper remains unrefined; A6/B2 wiring is explicit.')
