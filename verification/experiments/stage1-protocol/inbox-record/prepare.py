"""Add thin roots and scoped mutants to copies of the adopted production code."""
from hashlib import sha256
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/inbox-record')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SDK = 'controller-service/src/controller/lifecycle_inbox/renewal_confirmation.rs'
VARIANTS = ('original', 'candidate', 'skip-retention-window', 'wrong-recorded-version', 'restored')
MUTATIONS = {
    'skip-retention-window': (
        '        if lane.current_window != Some(intent.retention.window) {\n'
        '            return Err(JournalError::Conflict);\n        }',
        '        let _ = lane;'),
    'wrong-recorded-version': (
        '                    intent: intent.clone(),',
        '                    intent: {\n'
        '                        let mut recorded = intent.clone();\n'
        '                        recorded.accepted_version = 0;\n'
        '                        recorded\n'
        '                    },'),
}

def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown record variant')
    if 'fn proof_record(' in (project/MODEL).read_text():
        raise ValueError('forwarding root already exists')
    mutation = None
    source = (project/MODEL).read_text()
    for symbol in ('fn find_entry(', 'fn session_index(', 'fn retention_lane_index(',
                   'fn retention_slot_used(', 'fn last_entry('):
        if source.count(symbol) != 1:
            raise ValueError('adopted production helper missing or duplicated: ' + symbol)
    if variant in MUTATIONS:
        path = project/MODEL
        text = path.read_text()
        start = text.index('    fn record_fresh(')
        end = text.index('\n}\n\nimpl Session', start)
        body = text[start:end]
        old, new = MUTATIONS[variant]
        if body.count(old) != 1:
            raise ValueError('fresh-record mutation is not unique')
        before = sha(path)
        changed = text[:start]+body.replace(old, new)+text[end:]
        style_change = None
        if variant == 'wrong-recorded-version':
            # A nontrivial initializer no longer triggers vec_init_then_push.
            # Remove only that now-unfulfilled expectation in this mutant.
            annotation = ('    #[expect(\n        clippy::vec_init_then_push,\n'
                '        reason = "fixed extraction cannot lower vec! MaybeUninit construction"\n'
                '    )]\n')
            if changed.count(annotation) != 1:
                raise ValueError('mutant lint expectation is not unique')
            changed = changed.replace(annotation, '')
            style_change = dict(old=annotation, new='')
        path.write_text(changed)
        mutation = dict(path=MODEL, old=old, new=new, before=before, after=sha(path),
                        scope='only the original record_fresh body', style_change=style_change)
    with (project/MODEL).open('a') as f:
        f.write('\n'+(project/HERE/'roots.rs.in').read_text())
    return {name:sha(project/name) for name in (MODEL, SDK)}, mutation
