"""Check finite original-lock bindings; not async Rust refinement or full acceptance."""
import json
import os
from pathlib import Path
import re

try:
    from .check_task_contract import ROOT, check as check_b1, digest
except ImportError:
    from check_task_contract import ROOT, check as check_b1, digest

NAMESPACE = 'SandboxModel.Boundaries.TokioRwLockContract.'
SOURCES = {'sandbox-guest-service/src/session.rs',
           'sandbox-guest-service/src/session/file_authorization.rs'}
MECHANICAL = {'current.construct', 'authorize.read-map', 'apply.shared-read',
              'apply.exclusive-write', 'permit.defaults', 'permit.guard-field', 'file.read-map'}
BUSINESS = {'authorize.credentials', 'apply.replacement', 'file.authorization'}
LIBRARY = {'src/sync/rwlock.rs', 'src/sync/rwlock/read_guard.rs',
           'src/sync/rwlock/owned_read_guard.rs', 'src/sync/rwlock/write_guard.rs',
           'src/sync/batch_semaphore.rs'}
SCOPE = dict(async_rust_refinement=False, fifo_or_unconditional_progress=False,
             queue_reservations_are_not_live_guards=True,
             payload_frame_requires_surviving_original_lock=True,
             linear_mapping_does_not_copy_guard_ownership=True,
             caller_provides_write_relation_and_actual_B2_events=True)


def check_bindings(root, env):
    if (env['proof_kind'] != 'conditional-library-event-model-with-finite-production-wiring'
            or env['scope'] != SCOPE):
        raise ValueError('RwLock scope cannot claim async refinement, queue progress or final Arc framing')
    if set(env['production_sources']) != SOURCES or any(
            digest(root / name) != expected for name, expected in env['production_sources'].items()):
        raise ValueError('Original RwLock production source changed')
    if env['model_modules'] != ['Core', 'Mapping', 'Examples']:
        raise ValueError('RwLock model scope changed')
    theorems = []
    for module in env['model_modules']:
        source = (root / f'SandboxModel/Boundaries/TokioRwLock/{module}.lean').read_text()
        if re.search(r'\b(?:sorry|admit|axiom)\b', source):
            raise ValueError('Unapproved RwLock proof placeholder or assumption')
        prefix = NAMESPACE + ('Examples.' if module == 'Examples' else '')
        theorems += [prefix + name for name in re.findall(r'^theorem (\w+)', source, re.M)]
    if len(theorems) != 42 or len(set(theorems)) != 42 or sorted(theorems) != sorted(env['theorems']):
        raise ValueError('RwLock theorem inventory changed')
    audit = (root / 'SandboxModel/Boundaries/TokioRwLock/Audit.lean').read_text()
    if sorted(re.findall(r'^#audit_controller_proof (\S+)$', audit, re.M)) != sorted(theorems):
        raise ValueError('Missing, duplicate or extra RwLock audit')
    ids = [point['id'] for point in env['use_points']]
    if len(ids) != len(set(ids)) or set(ids) != MECHANICAL | BUSINESS:
        raise ValueError('Finite RwLock use-point registration changed')
    for point in env['use_points']:
        if point['file'] not in SOURCES or not all(point.get(k) for k in
                ['original_object', 'preconditions', 'caller_obligations']):
            raise ValueError('RwLock use point lacks original object, conditions or provider')
        if any(name not in theorems for name in point['public_results']):
            raise ValueError('Unknown RwLock public result')
        business = point['id'] in BUSINESS
        if (business and (point['status'] != 'business-obligation' or point['public_results'])
                or not business and (point['status'] != 'conditional-mechanical' or not point['public_results'])):
            raise ValueError('RwLock business decision cannot become trusted wiring')
        source = (root / point['file']).read_text()
        start, end = point['range']
        if source.count(start) != 1 or source.count(end) != 1:
            raise ValueError('Ambiguous RwLock source range: ' + point['id'])
        lower = source.index(start)
        fragment = source[lower:source.index(end, lower + len(start))]
        if not point['anchors'] or any(fragment.count(anchor) != 1 for anchor in point['anchors']):
            raise ValueError('Reviewed RwLock call anchor changed: ' + point['id'])
        positions = [fragment.index(anchor) for anchor in point['anchors']]
        if point['ordered'] and positions != sorted(positions):
            raise ValueError('Original RwLock call order changed: ' + point['id'])
    return env


def check(root=ROOT, cargo_home=None):
    env = check_bindings(root, json.loads((root / 'verification/tokio-rwlock-environment.json').read_text()))
    b1 = check_b1(root, cargo_home)
    if b1['tokio']['version'] != env['tokio']['version'] or set(env['tokio']['reviewed_sources']) != LIBRARY:
        raise ValueError('Missing reviewed RwLock library or fixed Tokio version')
    cargo = Path(cargo_home or os.environ.get('CARGO_HOME', Path.home() / '.cargo'))
    sources = list((cargo / 'registry/src').glob('*/tokio-' + env['tokio']['version']))
    if len(sources) != 1:
        raise ValueError('Missing unambiguous original RwLock library')
    for name, expected in env['tokio']['reviewed_sources'].items():
        if digest(sources[0] / name) != expected:
            raise ValueError('Reviewed RwLock library changed: ' + name)
    return env


if __name__ == '__main__':
    contract = check()
    print(f"RwLock: {len(contract['theorems'])} model declarations; seven mechanical and three business source groups.")
    print('Finite source/library admission only; async Rust refinement and full acceptance are not claimed.')
