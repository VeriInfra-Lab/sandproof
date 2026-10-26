"""Isolate the actual unload validator's encoding call; preserve its checks."""
from hashlib import sha256
from pathlib import Path
import difflib

HERE = Path('verification/experiments/stage1-protocol/inbox-unload-resolution')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/unload_resolution.rs'
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
MUTATIONS = {
    'ignore-snapshot-phase': (
        '|| original.phase != core::Phase::Running',
        '|| { let _: core::Phase = original.phase; false }'),
    'ignore-capture-operation': (
        '} else if entry.local_operation.is_none() {',
        '} else if false {'),
    'ignore-result-maximum': (
        '|| result.end_at_ms > result.max_end_at_ms',
        '|| { let _ = result.max_end_at_ms; false }'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')
OPERATION_GUARD = '''matches!(
        entry.intent.request.operation,
        LifecycleOperation::Unload { .. }
    )'''
OPERATION_HELPER = '''
fn is_unload_entry(entry: &Entry) -> bool {
    matches!(
        entry.intent.request.operation,
        LifecycleOperation::Unload { .. }
    )
}
'''
INTERFACE = '''pub(super) trait UnloadEncoding {
    fn encode(value: &UnloadResolution) -> Result<Vec<u8>, serde_json::Error>;
}
struct StandardUnloadEncoding;
impl UnloadEncoding for StandardUnloadEncoding {
    fn encode(value: &UnloadResolution) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }
}

pub(super) fn validate_entry(entry: &Entry) -> Result<(), JournalError> {
    validate_entry_using::<StandardUnloadEncoding>(entry)
}

pub(super) fn validate_entry_using<E: UnloadEncoding>(entry: &Entry) -> Result<(), JournalError> {'''


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def candidate(project):
    before = (project / SOURCE).read_text()
    signature = 'pub(super) fn validate_entry(entry: &Entry) -> Result<(), JournalError> {'
    changes = [(signature, INTERFACE), ('serde_json::to_vec(resolution)?', 'E::encode(resolution)?'),
               (OPERATION_GUARD, 'is_unload_entry(entry)'),
               ('if let UnloadResolution::BeforeAdmission { original, .. } = resolution {\n',
                'if let UnloadResolution::BeforeAdmission { original, .. } = resolution {\n'
                '        let original = &**original;\n'),
               ('if !is_unload_entry(entry) || E::encode(resolution)?.len() > 4096\n    {',
                'if !is_unload_entry(entry) || E::encode(resolution)?.len() > 4096 {')]
    after = before
    for old, new in changes:
        if after.count(old) != 1:
            raise ValueError('unload encoding factorization is not unique')
        after = after.replace(old, new, 1)
    if 'fn is_unload_entry(' in before:
        raise ValueError('operation helper already exists')
    return before, after + OPERATION_HELPER, changes


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown unload-resolution variant')
    before, correct, changes = candidate(project)
    mutation = None
    if variant not in ('original', 'restored'):
        actual = correct
        if variant in MUTATIONS:
            old, new = MUTATIONS[variant]
            if actual.count(old) != 1:
                raise ValueError('unload mutation is not unique')
            actual = actual.replace(old, new, 1)
            mutation = dict(path=SOURCE, old=old, new=new,
                before=sha256(correct.encode()).hexdigest(), after=sha256(actual.encode()).hexdigest())
        (project / SOURCE).write_text(actual)
    root = project / MODEL
    if 'fn proof_unload_entry(' in root.read_text():
        raise ValueError('forwarding root already exists')
    with root.open('a') as stream:
        stream.write('\n' + (project / HERE / 'roots.rs.in').read_text())
        if variant not in ('original', 'restored'):
            stream.write('\n' + (project / HERE / 'generic-root.rs.in').read_text())
    return {n: sha(project / n) for n in (SOURCE, MODEL)}, dict(
        source_before=sha256(before.encode()).hexdigest(), source_after=sha(project / SOURCE),
        mutation=mutation, changes=changes, production_modified=False,
        original_equivalence_proved=False, standard_serde_instance_adopted=False)


def patch(project):
    before, after, _ = candidate(project)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile='a/' + SOURCE, tofile='b/' + SOURCE, n=0))
