"""Finite original Mutex bindings and exact input/audit admission, not async Rust proof."""
from pathlib import Path
import hashlib
import json
import re

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[2]
OWN=HERE.relative_to(ROOT)
MODEL=Path('SandboxModel/Boundaries/TokioMutex')
FACADE=Path('SandboxModel/Boundaries/TokioMutex.lean')
EXPECTED_AUDITS=['SandboxModel.Boundaries.TokioMutexContract.initial_safe', 'SandboxModel.Boundaries.TokioMutexContract.step_refines_existing_lock', 'SandboxModel.Boundaries.TokioMutexContract.step_keeps_empty_readers', 'SandboxModel.Boundaries.TokioMutexContract.step_safe', 'SandboxModel.Boundaries.TokioMutexContract.history_safe', 'SandboxModel.Boundaries.TokioMutexContract.history_refines_existing_lock', 'SandboxModel.Boundaries.TokioMutexContract.returned_guard_is_original', 'SandboxModel.Boundaries.TokioMutexContract.live_access_is_exclusive', 'SandboxModel.Boundaries.TokioMutexContract.held_guard_blocks_return', 'SandboxModel.Boundaries.TokioMutexContract.original_lock_identity', 'SandboxModel.Boundaries.TokioMutexContract.release_invalidates_access', 'SandboxModel.Boundaries.TokioMutexContract.forget_invalidates_access', 'SandboxModel.Boundaries.TokioMutexContract.forget_keeps_exclusion', 'SandboxModel.Boundaries.TokioMutexContract.normal_release_keeps_inline_payload', 'SandboxModel.Boundaries.TokioMutexContract.invariant_from_caller_writes', 'SandboxModel.Boundaries.TokioMutexContract.acquired_use_is_protected', 'SandboxModel.Boundaries.TokioMutexContract.early_release_rejected_by_same_protected_use', 'SandboxModel.Boundaries.TokioMutexContract.Examples.nonempty_legal_history', 'SandboxModel.Boundaries.TokioMutexContract.Examples.own_mutation_and_release_do_not_rollback', 'SandboxModel.Boundaries.TokioMutexContract.Examples.retained_use_is_legal', 'SandboxModel.Boundaries.TokioMutexContract.Examples.detached_early_release_is_rejected']
EXPECTED_USES={'init.new','init.apply','input.new','input.write','input.close','input.install','input.finish'}

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def check_inputs(root=ROOT):
    environment=json.loads((root/OWN/'environment.json').read_text())
    frozen=json.loads((root/OWN/'frozen.json').read_text())
    if environment.get('proof_kind')!='conditional-library-event-model-with-finite-production-wiring':
        raise ValueError('Not an async Rust refinement')
    for field in ('async_rust_refinement','owned_or_map_api','fifo_or_unconditional_progress','poison_result','panic_rolls_back_payload'):
        if environment.get(field) is not False:
            raise ValueError('Unsupported scope: '+field)
    points=environment['use_points']
    if len(points)!=7 or {p['id'] for p in points}!=EXPECTED_USES:
        raise ValueError('Seven original Mutex bindings required')
    for point in points:
        text=(root/point['file']).read_text();position=0
        if not point.get('original_object') or not point.get('caller_obligations'):
            raise ValueError('Missing original object/caller responsibility')
        for anchor in point['ordered_anchors']:
            position=text.find(anchor,position)
            if position<0:
                raise ValueError('Original Mutex source binding changed: '+point['id'])
            position+=len(anchor)
    audits=[]
    for path in (root/MODEL).glob('*.lean'):
        text=path.read_text()
        if re.search(r'\b(?:sorry|admit|axiom)\b',text):
            raise ValueError('Unapproved proof placeholder/assumption')
        audits+=re.findall(r'^#audit_controller_proof (\S+)$',text,re.M)
    if sorted(audits)!=sorted(EXPECTED_AUDITS) or environment['audits']!=EXPECTED_AUDITS:
        raise ValueError('Exact 21 audit names required')
    own={str(p.relative_to(root)) for d in (root/MODEL,root/OWN) for p in d.rglob('*')
         if p.is_file() and '__pycache__' not in p.parts and p.name!='frozen.json'}
    own.add(str(FACADE))
    if frozen.get('schema')!=1 or set(frozen.get('files',{}))!=own:
        raise ValueError('Missing/unregistered component input')
    for name,digest in {**environment['base_inputs'],**frozen['files'],**environment['production_sources']}.items():
        path=Path(name)
        if path.is_absolute() or '..' in path.parts or (root/path).is_symlink() or sha(root/path)!=digest:
            raise ValueError('Frozen input differs: '+name)
    return frozen

if __name__=='__main__':
    value=check_inputs()
    print(json.dumps(dict(success=True,files=len(value['files']),audits=21,use_groups=7)))
