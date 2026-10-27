"""Isolated candidate and exact mutations for the real retired-identity validator."""
from pathlib import Path
import difflib
import hashlib
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'

def candidate(project):
    before = (project/MODEL).read_text()
    old = '''self.lanes[..index].iter().any(|other| {
                other.last_retired.as_ref().is_some_and(|last| {
                    last.request.request.binding.session == binding.session
                        || last.request.request.binding.key == binding.key
                })
            }) || self.sessions.iter().any(|session| {
                session.id == binding.session
                    || session
                        .entries
                        .iter()
                        .any(|entry| entry.intent.request.binding.key == binding.key)
            })'''
    new = '''self.prior_retired_identity_conflict(index, &binding.session, &binding.key)
                || self.current_identity_conflict(&binding.session, &binding.key)'''
    if before.count(old+' {') != 1:
        raise ValueError('nonunique original retired scan')
    after = before.replace(old+' {', new+'\n            {')
    marker = '    fn validate_retired_identities(&self) -> Result<(), JournalError> {\n'
    helpers = '''    fn prior_retired_identity_conflict(
        &self,
        index: usize,
        id: &SessionId,
        key: &sandproof_protocol::lifecycle::OperationKey,
    ) -> bool {
        for other in &self.lanes[..index] {
            if let Some(last) = &other.last_retired
                && (last.request.request.binding.session == *id
                    || last.request.request.binding.key == *key)
            {
                return true;
            }
        }
        false
    }

    fn current_identity_conflict(
        &self,
        id: &SessionId,
        key: &sandproof_protocol::lifecycle::OperationKey,
    ) -> bool {
        for session in &self.sessions {
            if session.id == *id || session.contains_entry_key(key) {
                return true;
            }
        }
        false
    }

'''
    if after.count(marker) != 1:
        raise ValueError('nonunique retired validator')
    after = after.replace(marker,helpers+marker)
    helper = '''impl Session {
    fn contains_entry_key(&self, key: &sandproof_protocol::lifecycle::OperationKey) -> bool {
        for entry in &self.entries {
            if entry.intent.request.binding.key == *key {
                return true;
            }
        }
        false
    }

'''
    if after.count('impl Session {\n') != 1:
        raise ValueError('nonunique Session impl')
    after = after.replace('impl Session {\n',helper)
    return before,after

HERE = Path('verification/experiments/stage1-protocol/inbox-retired-identities')
MUTATIONS = {
    'key-sequence-only': ('entry.intent.request.binding.key == *key',
        'entry.intent.request.binding.key.sequence == key.sequence'),
    'ignore-active-conflicts': (
        "self.prior_retired_identity_conflict(index, &binding.session, &binding.key)\n"
        "                || self.current_identity_conflict(&binding.session, &binding.key)\n            {",
        "self.prior_retired_identity_conflict(index, &binding.session, &binding.key) || {\n"
        "                let _ = self.current_identity_conflict(&binding.session, &binding.key);\n"
        "                false\n            } {"),
    'ignore-prior-session': (
        "last.request.request.binding.session == *id\n                    || last.request.request.binding.key == *key",
        "{\n                    let _ = last.request.request.binding.session == *id;\n"
        "                    false\n                } || last.request.request.binding.key == *key"),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown retired-identity variant')
    before, after = candidate(project)
    mutation = None
    if variant not in ('original', 'restored'):
        if variant in MUTATIONS:
            old, new = MUTATIONS[variant]
            if after.count(old) != 1:
                raise ValueError('nonunique retired-identity mutation')
            changed = after.replace(old, new, 1)
            mutation = dict(path=MODEL, old=old, new=new,
                before=hashlib.sha256(after.encode()).hexdigest(),
                after=hashlib.sha256(changed.encode()).hexdigest())
            after = changed
    else:
        after = before
    if 'fn proof_validate_retired_identities(' in after:
        raise ValueError('retired-identity root already exists')
    (project/MODEL).write_text(after+'\n'+(project/HERE/'roots.rs.in').read_text())
    return {MODEL: sha(project/MODEL)}, dict(
        original_inputs={MODEL: hashlib.sha256(before.encode()).hexdigest()},
        mutation=mutation, production_modified=False, original_equivalence_proved=False)


def patch(project):
    before, after = candidate(project)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile='a/'+MODEL, tofile='b/'+MODEL, n=0))
