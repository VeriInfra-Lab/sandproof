"""Admit finite Notify contracts and named source bindings, not async refinement."""
import json
from pathlib import Path
import re
import sys

try:
    from .check_task_contract import ROOT, check as check_b1, digest
except ImportError:
    from check_task_contract import ROOT, check as check_b1, digest

EXPECTED_SITES = {'pty.construct', 'pty.close', 'pty.read', 'pty.write',
                  'record.construct', 'record.retire', 'record.wait',
                  'process.close-decision', 'reaper.condvar'}
NAMESPACE = 'SandboxModel.Boundaries.NotifyContract.'


def check(root=ROOT, cargo_home=None):
    env = json.loads((root / 'verification/notify-environment.json').read_text())
    if env['proof_kind'] != 'conditional-interface-model-not-extracted-async-rust':
        raise ValueError('Notify model is not extracted async Rust')
    entries = (root / 'verification/notify-inputs.sha256').read_text().splitlines()
    paths = [line.split('  ', 1)[1] for line in entries]
    if len(paths) != len(set(paths)) or sorted(paths) != sorted(env['inputs']):
        raise ValueError('Missing or duplicate Notify input')
    for line in entries:
        expected, name = line.split('  ', 1)
        path = (root / name).resolve()
        if not path.is_relative_to(root.resolve()) or digest(path) != expected:
            raise ValueError('Frozen Notify input changed: ' + name)
    # B1 also checks the full original Tokio archive/source inventory. Its
    # permit contract is reused by the actual Record ownership regression.
    b1 = check_b1(root, cargo_home)
    if b1['tokio']['version'] != env['tokio']['version']:
        raise ValueError('Notify and permit library versions differ')
    import os
    cargo = Path(cargo_home or os.environ.get('CARGO_HOME', Path.home() / '.cargo'))
    sources = list((cargo / 'registry/src').glob('*/tokio-' + env['tokio']['version']))
    if len(sources) != 1:
        raise ValueError('Missing unambiguous reviewed Notify source')
    for name, expected in env['tokio']['reviewed_sources'].items():
        if digest(sources[0] / name) != expected:
            raise ValueError('Reviewed Notify library source changed: ' + name)
    theorems = []
    for module in env['model_modules']:
        source = (root / ('SandboxModel/Boundaries/Notify/' + module + '.lean')).read_text()
        if re.search(r'\b(?:sorry|admit|axiom)\b', source):
            raise ValueError('Unapproved proof placeholder or assumption')
        theorems.extend(NAMESPACE + n for n in re.findall(r'^theorem (\w+)', source, re.M))
    if sorted(theorems) != sorted(env['theorems']) or len(set(theorems)) != len(theorems):
        raise ValueError('Notify theorem inventory changed')
    audit = (root / 'SandboxModel/Boundaries/Notify/Audit.lean').read_text()
    audited = re.findall(r'^#audit_controller_proof (\S+)$', audit, re.M)
    if sorted(audited) != sorted(theorems):
        raise ValueError('Missing, duplicate or extra Notify audit')
    ids = [site['id'] for site in env['use_points']]
    if len(set(ids)) != len(ids) or set(ids) != EXPECTED_SITES:
        raise ValueError('Finite Notify use-point registration changed')
    for site in env['use_points']:
        if (site['file'] not in paths or not site['caller_obligations']
                or not site['preconditions'] or not site['status']):
            raise ValueError('Notify use point lacks source, condition or provider')
        if any(n not in theorems for n in site['public_results']):
            raise ValueError('Unknown Notify public theorem')
        if site['status'] == 'conditional-mechanical' and not site['public_results']:
            raise ValueError('Mechanical Notify site has no reusable conclusion')
        if site['id'] == 'reaper.condvar' and (site['status'] != 'public-gap' or site['public_results']):
            raise ValueError('Condvar cannot be relabelled as covered by Notify')
        if site['id'] == 'process.close-decision' and site['status'] != 'business-obligation':
            raise ValueError('Close authorization is not trusted wiring')
        text = (root / site['file']).read_text()
        start, end = site['range']
        if text.count(start) != 1 or text.count(end) != 1:
            raise ValueError('Ambiguous Notify function range: ' + site['id'])
        lower = text.index(start); upper = text.index(end, lower + len(start))
        fragment = text[lower:upper]
        positions = []
        for anchor in site['anchors']:
            if fragment.count(anchor) != 1:
                raise ValueError('Reviewed Notify call anchor changed: ' + site['id'])
            positions.append(fragment.index(anchor))
        if site.get('ordered') and positions != sorted(positions):
            raise ValueError('Notify prepare/check or publish/broadcast order changed: ' + site['id'])
    return env


if __name__ == '__main__':
    try:
        env = check()
    except (OSError, ValueError, KeyError, StopIteration) as error:
        sys.exit(str(error))
    print(f"Notify: {len(env['theorems'])} conditional theorem roots; seven mechanical groups.")
    print('Close decisions remain business obligations; Condvar remains a public gap.')
