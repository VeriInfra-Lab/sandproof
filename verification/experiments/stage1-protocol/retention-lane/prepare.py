"""Factor only terminal encoding in an isolated copy of the actual lane validator."""
from hashlib import sha256
from pathlib import Path
import difflib
import json

HERE = Path('verification/experiments/stage1-protocol/retention-lane')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SHIM = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'
MUTATIONS = {
    'ignore-successor': ('if self.current_window != successor {\n            return false;\n        }',
        'if self.current_window != successor {\n            return true;\n        }'),
    'allow-pending': ('last.status == sandproof_protocol::lifecycle::TerminalStatus::Cleaned\n                    && last.matches_request(&last.request)',
        '({\n                    let _ = last.status == sandproof_protocol::lifecycle::TerminalStatus::Cleaned;\n                    true\n                }) && last.matches_request(&last.request)'),
    'ignore-context': ('&& last.request.validate(context).is_ok()',
        '&& {\n                        let _ = last.request.validate(context);\n                        true\n                    }'),
    'omit-slot': ('                    && last.request.retention.slot == self.slot\n', ''),
    'omit-window': ('                    && last.request.retention.window.get() == floor\n', ''),
    'oversize-encoding': ('Ok(bytes) => bytes.len() <= 8192,', 'Ok(bytes) => bytes.len() <= 8193,'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')
INTERFACE = '''pub(super) trait TerminalEncoding {
    fn encode(
        value: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
    ) -> Result<Vec<u8>, serde_json::Error>;
}
struct StandardTerminalEncoding;
impl TerminalEncoding for StandardTerminalEncoding {
    fn encode(
        value: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }
}

'''
HELPER = '''
fn terminal_encoded_within_limit<E: TerminalEncoding>(
    value: &sandproof_protocol::lifecycle::TerminalLifecycleObservation,
) -> bool {
    let encoded = E::encode(value);
    match &encoded {
        Ok(bytes) => bytes.len() <= 8192,
        Err(_) => false,
    }
}
'''


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def candidate(project):
    before = (project/SOURCE).read_text()
    signature = '    fn valid(&self, context: &AuthenticatedCenterContext) -> bool {'
    replacement = signature + '''
        self.valid_using::<StandardTerminalEncoding>(context)
    }
    fn valid_using<E: TerminalEncoding>(&self, context: &AuthenticatedCenterContext) -> bool {'''
    changes = [('impl LifecycleRetentionLane {', INTERFACE+'impl LifecycleRetentionLane {'),
        (signature, replacement),
        ('serde_json::to_vec(last).is_ok_and(|bytes| bytes.len() <= 8192)',
         'terminal_encoded_within_limit::<E>(last)')]
    after = before
    for old, new in changes:
        if after.count(old) != 1:
            raise ValueError('nonunique terminal encoding factorization')
        after = after.replace(old, new, 1)
    return before, after+HELPER, changes


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown retention lane variant')
    inventory = json.loads((project/HERE/'inventory.json').read_text())
    if sha(project/SOURCE) != inventory['production_inputs'][SOURCE]:
        raise ValueError('fixed original lane source differs')
    before, correct, edits = candidate(project)
    actual = before if variant in ('original', 'restored') else correct
    mutation = None
    if variant in MUTATIONS:
        old, new = MUTATIONS[variant]
        if actual.count(old) != 1:
            raise ValueError('nonunique lane mutation')
        actual = actual.replace(old, new, 1)
        mutation = dict(path=SOURCE, old=old, new=new,
            before=sha256(correct.encode()).hexdigest(), after=sha256(actual.encode()).hexdigest())
    actual += '\n'+(project/HERE/'roots.rs.in').read_text()
    if variant not in ('original', 'restored'):
        actual += '\n'+(project/HERE/'generic-root.rs.in').read_text()
    (project/SOURCE).write_text(actual)
    shim = (project/SHIM).read_text()
    if 'fn terminal_matches(' in shim:
        raise ValueError('terminal forwarding root already exists')
    (project/SHIM).write_text(shim+(project/HERE.parent/'terminal-binding/roots.rs.in').read_text())
    return {n:sha(project/n) for n in (SOURCE, SHIM)}, dict(mutation=mutation,
        source_before=sha256(before.encode()).hexdigest(), source_after=sha(project/SOURCE),
        changes=edits, production_modified=False, private_refactor=variant not in ('original', 'restored'),
        original_equivalence_proved=False, standard_serde_instance_adopted=False)


def patch(project):
    before, after, _ = candidate(project)
    return ''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
        fromfile='a/'+SOURCE, tofile='b/'+SOURCE, n=3))
