"""Admit finite JoinSet contracts and named source bindings, not async refinement."""
import json
from pathlib import Path
import re
import sys

try:
    from .check_task_contract import ROOT, check as check_b1, digest
except ImportError:
    from check_task_contract import ROOT, check as check_b1, digest

EXPECTED_SITES = {
    'center.construct-and-spawn', 'center.backgrounds', 'center.supervise', 'center.result-policy',
    'protected.construct-and-reap', 'protected.capture', 'protected.abort-and-drain', 'protected.callback-policy',
    'guest.construct', 'guest.admission', 'guest.spawn', 'guest.reap', 'guest.normal-drain',
    'guest.interruption', 'guest.result-policy',
}
BUSINESS_SITES = {'center.result-policy', 'protected.callback-policy', 'guest.result-policy'}
SOURCE_FILES = {'sandbox-center/src/runtime.rs', 'sandbox-internal/src/protected_server.rs',
                'sandbox-guest-service/src/listener.rs'}
NAMESPACE = 'SandboxModel.Boundaries.JoinSetContract.'


def check(root=ROOT, cargo_home=None):
    env = json.loads((root / 'verification/joinset-environment.json').read_text())
    if env['proof_kind'] != 'conditional-interface-model-not-extracted-async-rust':
        raise ValueError('JoinSet model is not extracted async Rust')
    entries = (root / 'verification/joinset-inputs.sha256').read_text().splitlines()
    paths = [line.split('  ', 1)[1] for line in entries]
    if len(paths) != len(set(paths)) or sorted(paths) != sorted(env['inputs']):
        raise ValueError('Missing or duplicate JoinSet input')
    for line in entries:
        expected, name = line.split('  ', 1)
        path = (root / name).resolve()
        if not path.is_relative_to(root.resolve()) or digest(path) != expected:
            raise ValueError('Frozen JoinSet input changed: ' + name)
    # B1 checks the same original Tokio archive/source inventory and permit rules.
    # A group result may still own a permit; joining alone does not return it.
    b1 = check_b1(root, cargo_home)
    if b1['tokio']['version'] != env['tokio']['version']:
        raise ValueError('JoinSet and permit library versions differ')
    import os
    cargo = Path(cargo_home or os.environ.get('CARGO_HOME', Path.home() / '.cargo'))
    sources = list((cargo / 'registry/src').glob('*/tokio-' + env['tokio']['version']))
    if len(sources) != 1:
        raise ValueError('Missing unambiguous reviewed JoinSet source')
    for name, expected in env['tokio']['reviewed_sources'].items():
        if digest(sources[0] / name) != expected:
            raise ValueError('Reviewed JoinSet library source changed: ' + name)
    theorems = []
    for module in env['model_modules']:
        source = (root / ('SandboxModel/Boundaries/JoinSet/' + module + '.lean')).read_text()
        if re.search(r'\b(?:sorry|admit|axiom)\b', source):
            raise ValueError('Unapproved proof placeholder or assumption')
        theorems.extend(NAMESPACE + n for n in re.findall(r'^theorem (\w+)', source, re.M))
    if sorted(theorems) != sorted(env['theorems']) or len(set(theorems)) != len(theorems):
        raise ValueError('JoinSet theorem inventory changed')
    audit = (root / 'SandboxModel/Boundaries/JoinSet/Audit.lean').read_text()
    audited = re.findall(r'^#audit_controller_proof (\S+)$', audit, re.M)
    if sorted(audited) != sorted(theorems):
        raise ValueError('Missing, duplicate or extra JoinSet audit')
    observed = {str(p.relative_to(root)) for pattern in ('sandbox-*/src/**/*.rs', 'controller-*/src/**/*.rs')
                for p in root.glob(pattern) if re.search(r'\bJoinSet\b', p.read_text())}
    if observed != SOURCE_FILES:
        raise ValueError('JoinSet production-file census changed')
    if set(env['production_sources']) != SOURCE_FILES or any(digest(root/n) != value for n, value in env['production_sources'].items()):
        raise ValueError('Original JoinSet production source changed')
    ids = [site['id'] for site in env['use_points']]
    if len(set(ids)) != len(ids) or set(ids) != EXPECTED_SITES:
        raise ValueError('Finite JoinSet use-point registration changed')
    for site in env['use_points']:
        if (site['file'] not in paths or not site['caller_obligations']
                or not site['preconditions'] or not site['status']):
            raise ValueError('JoinSet use point lacks source, condition or provider')
        if any(n not in theorems for n in site['public_results']):
            raise ValueError('Unknown JoinSet public theorem')
        if site['status'] == 'conditional-mechanical' and not site['public_results']:
            raise ValueError('Mechanical JoinSet site has no reusable conclusion')
        if site['id'] in BUSINESS_SITES and (site['status'] != 'business-obligation' or site['public_results']):
            raise ValueError('JoinSet business callback or result decision is not trusted wiring')
        if site['id'] not in BUSINESS_SITES and site['status'] != 'conditional-mechanical':
            raise ValueError('Required public JoinSet rule cannot be moved to business')
        text = (root / site['file']).read_text()
        start, end = site['range']
        if text.count(start) != 1 or text.count(end) != 1:
            raise ValueError('Ambiguous JoinSet function range: ' + site['id'])
        lower = text.index(start); upper = text.index(end, lower + len(start))
        fragment = text[lower:upper]
        positions = []
        for anchor in site['anchors']:
            if fragment.count(anchor) != 1:
                raise ValueError('Reviewed JoinSet call anchor changed: ' + site['id'])
            positions.append(fragment.index(anchor))
        if site.get('ordered') and positions != sorted(positions):
            raise ValueError('JoinSet admission or abort/drain order changed: ' + site['id'])
    return env


if __name__ == '__main__':
    try:
        env = check()
    except (OSError, ValueError, KeyError, StopIteration) as error:
        sys.exit(str(error))
    print(f"JoinSet: {len(env['theorems'])} conditional theorem roots; twelve mechanical groups.")
    print('Business callbacks/results and physical exit remain separate obligations; no async refinement.')
