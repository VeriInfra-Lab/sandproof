"""Admit the explicitly conditional Guest message contract, not async refinement."""
import json
import os
from pathlib import Path
import re
import sys
import tomllib

try:
    from .check_task_contract import ROOT, check as check_b1, digest
except ImportError:
    from check_task_contract import ROOT, check as check_b1, digest


def check(root=ROOT, cargo_home=None):
    env = json.loads((root / 'verification/owned-message-environment.json').read_text())
    if env['proof_kind'] != 'conditional-interface-model-not-extracted-async-rust':
        raise ValueError('Owned message contract is not extracted async Rust')
    lines = (root / 'verification/owned-message-inputs.sha256').read_text().splitlines()
    names = [line.split('  ', 1)[1] for line in lines]
    if len(names) != len(set(names)) or sorted(names) != sorted(env['inputs']):
        raise ValueError('Missing or duplicate owned-message input')
    for line in lines:
        expected, name = line.split('  ', 1)
        path = (root / name).resolve()
        if not path.is_relative_to(root.resolve()) or digest(path) != expected:
            raise ValueError('Frozen owned-message input changed: ' + name)
    b1 = check_b1(root, cargo_home)
    if env['libraries']['tokio']['version'] != b1['tokio']['version']:
        raise ValueError('B1 and message contracts use different Tokio versions')
    packages = tomllib.loads((root / 'Cargo.lock').read_text())['package']
    cargo = Path(cargo_home or os.environ.get('CARGO_HOME', Path.home() / '.cargo'))
    for name, library in env['libraries'].items():
        matches = [p for p in packages if p['name'] == name]
        if len(matches) != 1 or matches[0]['version'] != library['version']:
            raise ValueError('Unreviewed message library version: ' + name)
        roots = list((cargo / 'registry/src').glob(f"*/{name}-{library['version']}"))
        if len(roots) != 1:
            raise ValueError('Ambiguous or missing message library source: ' + name)
        registry = roots[0]
        archive = cargo / 'registry/cache' / registry.parent.name / f"{name}-{library['version']}.crate"
        if digest(archive) != matches[0]['checksum']:
            raise ValueError('Locked message library archive changed: ' + name)
        for filename, expected in library['reviewed_sources'].items():
            if digest(registry / filename) != expected:
                raise ValueError('Reviewed message library source changed: ' + filename)
    theorems = []
    for name in env['model_modules']:
        text = (root / ('SandboxModel/Boundaries/OwnedMessage/' + name + '.lean')).read_text()
        if re.search(r'\b(?:sorry|admit|axiom)\b', text):
            raise ValueError('Unapproved proof placeholder or assumption')
        theorems += ['SandboxModel.Boundaries.OwnedMessage.' + n for n in re.findall(r'^theorem (\w+)', text, re.M)]
    audit = (root / 'SandboxModel/Boundaries/OwnedMessage/Audit.lean').read_text()
    audited = re.findall(r'^#audit_controller_proof (\S+)$', audit, re.M)
    if sorted(theorems) != sorted(env['theorems']) or len(set(theorems)) != len(theorems):
        raise ValueError('Owned-message theorem inventory changed')
    if sorted(audited) != sorted(theorems) or len(audited) != len(theorems):
        raise ValueError('Missing, duplicate or extra owned-message theorem audit')
    # This is a finite registration, NOT a whole-project channel census.
    # Each trust site is a named operation; never a whole business function.
    ids = [site['id'] for site in env['use_points']]
    if len(set(ids)) != len(ids):
        raise ValueError('Duplicate message use-point identity')
    for site in env['use_points']:
        if (site['file'] not in env['inputs'] or not site['caller_obligations']
                or not site['preconditions'] or not site['public_results']):
            raise ValueError('Message use point lacks source or business provider')
        if any(name not in theorems for name in site['public_results']):
            raise ValueError('Unknown public result for message use point')
        text = (root / site['file']).read_text()
        for anchor in site['anchors']:
            if text.count(anchor) != 1:
                raise ValueError('Reviewed message call anchor changed: ' + site['id'])
    return env


if __name__ == '__main__':
    try:
        environment = check()
    except (OSError, ValueError, KeyError, StopIteration) as error:
        sys.exit(str(error))
    print(f"Owned message: {len(environment['theorems'])} conditional theorem roots; exact source/library admission.")
    print('Async Rust wiring is reviewed/tested, not Aeneas-translated or machine-refined.')
