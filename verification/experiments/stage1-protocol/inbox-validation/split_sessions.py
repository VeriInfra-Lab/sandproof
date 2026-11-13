from pathlib import Path
import difflib
import hashlib


def prepare(source: str):
    start = source.index('        for (i, session) in self.sessions.iter().enumerate() {\n', source.index('    pub(super) fn validate(&self)'))
    end = source.index('        Ok(())\n    }\n\n    fn validate_retired_identities', start)
    old = source[start:end]
    assert old.endswith('        }\n')
    inner = old.split('\n', 1)[1].removesuffix('        }\n')
    moved = '\n'.join(line[4:] if line else line for line in inner.split('\n'))
    assert moved.count('self.entry_kind(session, entry, &context)?') == 1
    moved = moved.replace('self.entry_kind(session, entry, &context)?', 'self.entry_kind(session, entry, context)?')
    helper = '''    fn validate_one_session(
        &self,
        i: usize,
        session: &Session,
        context: &AuthenticatedCenterContext,
    ) -> Result<(), JournalError> {
''' + moved + '        Ok(())\n    }\n\n'
    assert 'fn validate_one_session(' not in source
    loop = '''        for (i, session) in self.sessions.iter().enumerate() {
            self.validate_one_session(i, session, &context)?;
        }
'''
    candidate = source[:start] + loop + source[end:]
    marker = '    fn validate_retired_identities'
    assert candidate.count(marker) == 1
    candidate = candidate.replace(marker, helper + marker, 1)
    path = 'controller-service/src/controller/lifecycle_inbox/model.rs'
    diff = ''.join(difflib.unified_diff(source.splitlines(True), candidate.splitlines(True), fromfile='a/' + path, tofile='b/' + path))
    normalized = moved.replace('self.entry_kind(session, entry, context)?', 'self.entry_kind(session, entry, &context)?')
    assert normalized == '\n'.join(line[4:] if line else line for line in inner.split('\n'))
    return candidate, diff, {
        'kind': 'move-per-session-body-to-named-helper',
        'source_sha256': hashlib.sha256(source.encode()).hexdigest(),
        'candidate_sha256': hashlib.sha256(candidate.encode()).hexdigest(),
        'original_session_body_sha256': hashlib.sha256(inner.encode()).hexdigest(),
        'only_body_adjustments': ['dedent four spaces', 'pass borrowed context without a second borrow'],
        'normalized_body_equal': True,
        'retains_check_order_and_checked_count': True,
        'production_adopted': False,
        'general_equivalence_proved': False,
    }
