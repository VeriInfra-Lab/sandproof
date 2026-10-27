"""Private reverse-scan candidate and exact mutations of the real origin selector."""
from pathlib import Path
import difflib
import hashlib

HERE = Path('verification/experiments/stage1-protocol/inbox-origin-selection')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
ORIGINAL = '''    pub(super) fn current_execution_origin(&self) -> Option<&Entry> {
        self.entries.iter().rev().find(|entry| {
            matches!(
                entry.intent.request.operation,
                LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }
            )
        })
    }
'''
CANDIDATE = '''    pub(super) fn current_execution_origin(&self) -> Option<&Entry> {
        let mut index = self.entries.len();
        while index > 0 {
            index -= 1;
            let entry = &self.entries[index];
            if matches!(
                entry.intent.request.operation,
                LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }
            ) {
                return Some(entry);
            }
        }
        None
    }
'''
MUTATIONS = {
    'skip-resume': (
        'LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }',
        'LifecycleOperation::Create { .. }'),
    'require-local-operation': (
        '            ) {\n                return Some(entry);',
        '            ) && entry.local_operation.is_some()\n            {\n                return Some(entry);'),
    'return-first-slot': ('return Some(entry);', 'return Some(&self.entries[0]);'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def candidate(project):
    before = (project/MODEL).read_text()
    if before.count(ORIGINAL) != 1:
        raise ValueError('nonunique original origin selector')
    return before, before.replace(ORIGINAL, CANDIDATE, 1)


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown origin-selection variant')
    before, after = candidate(project)
    mutation = None
    if variant in ('original', 'restored'):
        after = before
    elif variant in MUTATIONS:
        old, new = MUTATIONS[variant]
        if CANDIDATE.count(old) != 1:
            raise ValueError('nonunique origin-selection mutation')
        changed = CANDIDATE.replace(old, new, 1)
        modified = before.replace(ORIGINAL, changed, 1)
        mutation = dict(path=MODEL, old=old, new=new,
            before=hashlib.sha256(after.encode()).hexdigest(),
            after=hashlib.sha256(modified.encode()).hexdigest())
        after = modified
    if 'fn proof_execution_origin(' in after:
        raise ValueError('origin-selection root already exists')
    (project/MODEL).write_text(after+'\n'+(project/HERE/'roots.rs.in').read_text())
    return {MODEL: sha(project/MODEL)}, dict(
        original_inputs={MODEL: hashlib.sha256(before.encode()).hexdigest()},
        mutation=mutation, production_modified=False, original_equivalence_proved=False)


def patch(project):
    before, after = candidate(project)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile='a/'+MODEL, tofile='b/'+MODEL, n=0))
