"""Use the adopted unload validator; add thin roots or scoped mutants to copies."""
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


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def candidate(project):
    before = (project / SOURCE).read_text()
    for symbol in ('pub(super) trait UnloadEncoding', 'fn validate_entry_using<',
                   'pub(super) struct StandardUnloadEncoding;',
                   'fn is_unload_entry('):
        if before.count(symbol) != 1:
            raise ValueError('adopted unload checker missing or duplicated: ' + symbol)
    return before, before, []


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
