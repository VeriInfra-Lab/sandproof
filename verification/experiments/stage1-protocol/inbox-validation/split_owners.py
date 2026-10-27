from pathlib import Path
import difflib
import hashlib
import importlib.util

spec = importlib.util.spec_from_file_location('first_split', Path(__file__).with_name('split_sessions.py'))
first = importlib.util.module_from_spec(spec)
spec.loader.exec_module(first)


def prepare(source: str):
    candidate, _, first_record = first.prepare(source)
    start = candidate.index('        for (i, owner) in self.owners.iter().enumerate() {\n', candidate.index('    pub(super) fn validate(&self)'))
    end = candidate.index('        for (i, session) in self.sessions.iter().enumerate() {\n', start)
    loop = candidate[start:end]
    assert loop.endswith('        }\n')
    assert 'validate_owners' not in source
    helper = '    fn validate_owners(&self) -> Result<(), JournalError> {\n' + loop + '        Ok(())\n    }\n\n'
    candidate = candidate[:start] + '        self.validate_owners()?;\n' + candidate[end:]
    marker = '    fn validate_one_session('
    assert candidate.count(marker) == 1
    candidate = candidate.replace(marker, helper + marker, 1)
    path = 'controller-service/src/controller/lifecycle_inbox/model.rs'
    diff = ''.join(difflib.unified_diff(source.splitlines(True), candidate.splitlines(True), fromfile='a/' + path, tofile='b/' + path))
    return candidate, diff, {
        'kind': 'move-owner-loop-and-per-session-body-to-named-helpers',
        'source_sha256': hashlib.sha256(source.encode()).hexdigest(),
        'candidate_sha256': hashlib.sha256(candidate.encode()).hexdigest(),
        'first_split': first_record,
        'owner_loop_sha256': hashlib.sha256(loop.encode()).hexdigest(),
        'owner_loop_byte_equal': True,
        'retains_check_order_and_checked_count': True,
        'production_adopted': False,
        'general_equivalence_proved': False,
    }
