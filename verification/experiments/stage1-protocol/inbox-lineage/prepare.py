"""Scope mutations to the original lineage validator in private source copies."""
from hashlib import sha256
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/inbox-lineage')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
MUTATIONS = {
    'advance-unattached-resume': (
        '                    if entry.local_operation.is_some() {\n'
        '                        incarnation = Some(target_incarnation);\n'
        '                    }',
        '                    incarnation = Some(target_incarnation);'),
    'repeat-capture': (
        'incarnation != Some(source_incarnation) || capture.get() <= capture_frontier',
        'incarnation != Some(source_incarnation) || capture.get() < capture_frontier'),
    'skip-final-incarnation': (
        '        if self.incarnation != incarnation {\n'
        '            return Err(JournalError::CorruptStore);\n'
        '        }', ''),
}
VARIANTS = ('original', *MUTATIONS, 'restored')


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown lineage variant')
    path = project/MODEL
    text = path.read_text()
    start = text.index('    fn validate_execution_lineage(')
    # It is the last method in this fixed implementation block.
    end = text.index('\n    }\n}', start)+len('\n    }')
    body = text[start:end]
    before = sha(path)
    mutation = None
    if variant in MUTATIONS:
        old, new = MUTATIONS[variant]
        if body.count(old) != 1:
            raise ValueError('lineage mutation is not unique')
        body = body.replace(old, new, 1)
        text = text[:start]+body+text[end:]
        path.write_text(text)
        mutation = dict(path=MODEL, old=old, new=new, before=before, after=sha(path),
                        scope='only the original Session::validate_execution_lineage body')
    if 'fn proof_execution_lineage(' in text:
        raise ValueError('lineage root already exists')
    path.write_text(text+'\n'+(project/HERE/'roots.rs.in').read_text())
    return {MODEL: sha(path)}, mutation
