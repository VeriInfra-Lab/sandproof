"""Admit complete original IR, generated method closure and existing Vec use."""
import json
from pathlib import Path

ROOTS = ['sandproof_controller_service::controller::lifecycle_inbox::readiness::proof_native_create_snapshot_completed']
INCLUDE = ['sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service']
SNAPSHOT_FIELDS = ['owner', 'phase', 'terminal', 'epoch', 'generation', 'current_vm',
                   'current_operation', 'lifetime', 'uncertain_actions']
PREFIX = 'SandproofNativeReadinessRust.'
METHOD = 'sandproof_controller_service.controller.lifecycle_inbox.readiness.NativeCreateSnapshot.has_completed_original_execution'
COMMON = {METHOD, 'sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq',
    'sandbox_controller_core.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq',
    'sandbox_controller_core.model.Phase.Insts.CoreCmpPartialEqPhase.eq',
    'sandbox_controller_core.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq'}
STATUS = {'sandbox_controller_core.model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq',
          'sandbox_controller_core.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq'}


def admit_ir(raw):
    if raw.get('has_errors') is not False:
        raise ValueError('incomplete original capture')
    translated = raw['translated']
    options = translated['options']
    if (options['start_from'] != ROOTS or options['include'] != INCLUDE or options['opaque']
            or options['exclude'] or options['skip_borrowck'] or options['no_typecheck']
            or options['monomorphize'] or options['remove_unused_clauses']):
        raise ValueError('unreviewed extraction options')
    if {item['key'] for item in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('wrong original target')
    functions = [f for f in translated['fun_decls'] if f]
    roots = [f for f in functions if f['item_meta']['name'][-1] == {'Ident': ['proof_native_create_snapshot_completed', 0]}]
    if len(roots) != 1 or roots[0]['item_meta']['opacity'] != 'Transparent':
        raise ValueError('missing unique transparent root')
    calls = []
    def walk(node):
        if isinstance(node, dict):
            if 'Call' in node:
                calls.append(node['Call']['call'])
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)
    walk(roots[0]['body'])
    if len(calls) != 1:
        raise ValueError('root does not contain exactly one call')
    target = calls[0]['func']['Regular']['kind']['Fun']
    actual = next(f for f in functions if f['def_id'] == target)
    expected = [{'Ident': [s, 0]} for s in ('sandproof_controller_service', 'controller', 'lifecycle_inbox', 'readiness')]
    if (actual['item_meta']['name'][:4] != expected or actual['item_meta']['name'][-1] !=
            {'Ident': ['has_completed_original_execution', 0]} or
            actual['item_meta']['opacity'] != 'Transparent' or 'Structured' not in actual['body']):
        raise ValueError('root calls a different/opaque checker')
    snapshots = [t for t in translated['type_decls'] if t and t['item_meta']['name'] ==
                 expected+[{'Ident': ['NativeCreateSnapshot', 0]}]]
    if len(snapshots) != 1 or [f['name'] for f in snapshots[0]['kind']['Struct']] != SNAPSHOT_FIELDS:
        raise ValueError('original complete snapshot fields differ')
    return dict(root=roots[0], call=calls[0], actual_method=actual, snapshot=snapshots[0])


def admit_generated(directory, variant):
    data = json.loads((directory/'translation.json').read_text())
    transparent = {f['lean_name'].removeprefix(PREFIX) for f in data['functions'] if not f['is_opaque']}
    expected = COMMON | (STATUS if variant == 'correct' else set())
    if transparent != expected or any(f['lean_file'] != 'SandproofNativeReadiness/Funs.lean'
            for f in data['functions'] if not f['is_opaque']):
        raise ValueError('missing, extra or changed original generated functions')
    if {f['lean_name'] for f in data['functions'] if f['is_opaque']} != {'alloc.vec.Vec.is_empty'}:
        raise ValueError('unreviewed function model or business opacity')
    if any(p.name == 'TypesExternal_Template.lean' for p in directory.rglob('*.lean')):
        raise ValueError('unexpected external type carrier')
    return data
