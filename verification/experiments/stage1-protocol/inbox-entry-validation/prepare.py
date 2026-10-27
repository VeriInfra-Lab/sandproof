"""Private source candidate for actual Inbox::entry_kind; production is read-only."""
from pathlib import Path
import difflib
import hashlib
import importlib.util

MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
UNLOAD = 'controller-service/src/controller/lifecycle_inbox/unload_resolution.rs'
PARENT = Path('verification/experiments/stage1-protocol/inbox-unload-resolution')
HERE = Path('verification/experiments/stage1-protocol/inbox-entry-validation')
MUTATIONS = {
    'duplicate-cap': ('count += 1;', 'count = 1;'),
    'key-sequence-only': ('entry.intent.request.binding.key == *key',
                          'entry.intent.request.binding.key.sequence == key.sequence'),
    'requested-deadline-only': ('''(if previous > *end_at_ms {
                                    previous
                                } else {
                                    *end_at_ms
                                })''', '''{
                                    let _ = previous;
                                    *end_at_ms
                                }'''),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def candidate(project):
    spec = importlib.util.spec_from_file_location('entry_existing_unload', project/PARENT/'prepare.py')
    parent = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(parent)
    unload_before, unload, changes = parent.candidate(project)
    if unload.count('struct StandardUnloadEncoding;') != 1:
        raise ValueError('standard encoder is not unique')
    unload = unload.replace('struct StandardUnloadEncoding;', 'pub(super) struct StandardUnloadEncoding;')
    old_wrapper = """pub(super) fn validate_entry(entry: &Entry) -> Result<(), JournalError> {
    validate_entry_using::<StandardUnloadEncoding>(entry)
}

"""
    if unload.count(old_wrapper) != 1:
        raise ValueError('nonunique standard wrapper')
    unload = unload.replace(old_wrapper, '')
    before = (project/MODEL).read_text()
    signature = '''    fn entry_kind(
        &self,
        session: &Session,
        entry: &Entry,
        context: &AuthenticatedCenterContext,
    ) -> Result<usize, JournalError> {
'''
    if before.count(signature) != 1:
        raise ValueError('nonunique entry_kind signature')
    generic = signature.replace('fn entry_kind(', 'fn entry_kind_using<E: super::unload_resolution::UnloadEncoding>(')
    wrapper = signature + '''        self.entry_kind_using::<super::unload_resolution::StandardUnloadEncoding>(
            session, entry, context,
        )
    }

'''
    after = before.replace(signature, wrapper+generic)
    call = 'super::unload_resolution::validate_entry(entry)?;'
    if after.count(call) != 1:
        raise ValueError('nonunique unload call')
    after = after.replace(call, 'super::unload_resolution::validate_entry_using::<E>(entry)?;')
    count_query = """self
                .sessions
                .iter()
                .flat_map(|s| &s.entries)
                .filter(|e| e.intent.request.binding.key == binding.key)
                .count()"""
    if after.count(count_query) != 1:
        raise ValueError('nonunique complete count query')
    after = after.replace(count_query + '\n                != 1', 'self.count_entry_key(&binding.key) != 1')
    inbox_helper = """    fn count_entry_key(&self, key: &sandproof_protocol::lifecycle::OperationKey) -> usize {
        let mut count = 0_usize;
        for session in &self.sessions {
            count = session.count_entry_key_from(key, count);
        }
        count
    }

"""
    after = after.replace(signature, inbox_helper+signature, 1)
    session_helper = """impl Session {
    fn count_entry_key_from(
        &self,
        key: &sandproof_protocol::lifecycle::OperationKey,
        mut count: usize,
    ) -> usize {
        for entry in &self.entries {
            if entry.intent.request.binding.key == *key {
                count += 1;
            }
        }
        count
    }

"""
    if after.count('impl Session {\n') != 1:
        raise ValueError('nonunique Session impl')
    after = after.replace('impl Session {\n', session_helper)
    if after.count('previous.max(*end_at_ms)') != 1:
        raise ValueError('nonunique signed maximum')
    old_maximum = '''entry
                                .previous_effective_end_at_ms
                                .is_none_or(|previous| end != previous.max(*end_at_ms))'''
    if after.count(old_maximum) != 1:
        raise ValueError('nonunique maximum callback')
    after = after.replace(old_maximum, '''entry.previous_effective_end_at_ms.is_none_or(|previous| {
                                end != (if previous > *end_at_ms {
                                    previous
                                } else {
                                    *end_at_ms
                                })
                            })''')
    return {MODEL: (before, after), UNLOAD: (unload_before, unload)}, changes


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown entry validation variant')
    sources, changes = candidate(project)
    mutation = None
    if variant not in ('original', 'restored'):
        for name, (_, body) in sources.items():
            (project/name).write_text(body)
        if variant in MUTATIONS:
            old, new = MUTATIONS[variant]
            body = (project/MODEL).read_text()
            if body.count(old) != 1:
                raise ValueError('entry mutation is not unique')
            (project/MODEL).write_text(body.replace(old, new, 1))
            mutation = dict(path=MODEL, old=old, new=new,
                before=hashlib.sha256(body.encode()).hexdigest(), after=sha(project/MODEL))
    root = project/MODEL
    if 'fn proof_entry_kind(' in root.read_text():
        raise ValueError('entry forwarding root already exists')
    with root.open('a') as stream:
        stream.write('\n'+(project/HERE/'roots.rs.in').read_text())
        if variant not in ('original', 'restored'):
            stream.write('\n'+(project/HERE/'generic-root.rs.in').read_text())
    return {name: sha(project/name) for name in sources}, dict(
        original_inputs={name: hashlib.sha256(value[0].encode()).hexdigest() for name, value in sources.items()},
        parent_changes=changes, mutation=mutation, production_modified=False,
        original_equivalence_proved=False, standard_serde_instance_adopted=False)


def patch(project):
    sources, _ = candidate(project)
    return ''.join(''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile='a/'+name, tofile='b/'+name, n=0)) for name, (before, after) in sources.items())
