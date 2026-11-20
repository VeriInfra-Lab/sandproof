"""Reuse the reviewed record candidate and mutate only record_existing in private copies."""
from hashlib import sha256
import importlib.util
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/inbox-append')
PARENT = HERE.parent / 'inbox-record'
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SDK = 'controller-service/src/controller/lifecycle_inbox/renewal_confirmation.rs'
VARIANTS = ('original', 'candidate', 'skip-sdk-gate', 'skip-prior-capture', 'restored')
MUTATIONS = {
    'skip-sdk-gate': [(
        '        if super::renewal_confirmation::is_sdk_renewal(intent)\n'
        '            && super::renewal_confirmation::has_unconfirmed_sdk(session)\n'
        '        {\n            return Err(JournalError::UnresolvedAction);\n        }\n',
        '        let _ = super::renewal_confirmation::has_unconfirmed_sdk(session);\n')],
    'skip-prior-capture': [(
        '            LifecycleOperation::Resume {\n                source_incarnation,\n'
        '                capture,\n                target_incarnation,',
        '            LifecycleOperation::Resume {\n                source_incarnation,\n'
        '                capture: _,\n                target_incarnation,'), (
        'LifecycleOperation::Unload { source_incarnation: old, capture: old_capture }\n'
        '                        if old == *source_incarnation && old_capture == *capture)',
        'LifecycleOperation::Unload { source_incarnation: old, .. }\n'
        '                        if old == *source_incarnation)')],
}


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown append variant')
    spec = importlib.util.spec_from_file_location('append_parent_prepare', project/PARENT/'prepare.py')
    parent = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parent)
    parent.prepare(project, 'original' if variant == 'original' else 'candidate')
    mutation = None
    if variant in MUTATIONS:
        path = project/MODEL
        text = path.read_text()
        start = text.index('    fn record_existing(')
        end = text.index('    fn record_fresh(', start)
        body = text[start:end]
        before = sha(path)
        for old, new in MUTATIONS[variant]:
            if body.count(old) != 1:
                raise ValueError('existing-record mutation is not unique')
            body = body.replace(old, new)
        changed = text[:start]+body+text[end:]
        path.write_text(changed)
        mutation = dict(path=MODEL, replacements=[dict(old=old,new=new) for old,new in MUTATIONS[variant]],
                        before=before, after=sha(path), scope='only the original record_existing body')
    return {name:sha(project/name) for name in (MODEL, SDK)}, mutation
