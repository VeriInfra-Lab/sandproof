"""Original capacity function and exact mutations; no production refactor."""
from pathlib import Path
import hashlib

HERE = Path('verification/experiments/stage1-protocol/inbox-config')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
ORIGINAL = '''    pub fn required_bytes(&self) -> u64 {
        // Each finite position keeps its bounded operation history, an explicit
        // native mapping, one capture work and immutable bundle receipt.
        // The final operation position is reserved for terminal responsibility.
        u64::from(self.max_sessions)
            * (4 * u64::from(self.max_options_bytes)
                + u64::from(self.max_operations_per_session) * 8192
                + 2 * u64::from(self.max_manifest_bytes)
                + u64::from(self.max_profile_bytes)
                // The full latest SDK renewal confirmation has its own fixed
                // reservation, alongside terminal confirmation/retirement.
                + 4 * 8192)
            + 8192
    }
'''
MUTATIONS = {
    'halve-options-reservation': ('4 * u64::from(self.max_options_bytes)',
                                '2 * u64::from(self.max_options_bytes)'),
    'omit-manifest-copy': ('2 * u64::from(self.max_manifest_bytes)',
                           'u64::from(self.max_manifest_bytes)'),
    'omit-global-reserve': ('            + 8192\n', ''),
}
VARIANTS = ('original', *MUTATIONS, 'restored')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown config-budget variant')
    before = (project/MODEL).read_text()
    if before.count(ORIGINAL) != 1 or 'fn proof_inbox_required_bytes(' in before:
        raise ValueError('nonunique original capacity function or existing root')
    after = before
    mutation = None
    if variant in MUTATIONS:
        old, new = MUTATIONS[variant]
        if ORIGINAL.count(old) != 1:
            raise ValueError('nonunique config-budget mutation')
        changed = ORIGINAL.replace(old, new, 1)
        after = before.replace(ORIGINAL, changed, 1)
        mutation = dict(path=MODEL, old=old, new=new,
            before=hashlib.sha256(before.encode()).hexdigest(),
            after=hashlib.sha256(after.encode()).hexdigest())
    (project/MODEL).write_text(after+'\n'+(project/HERE/'roots.rs.in').read_text())
    return {MODEL: sha(project/MODEL)}, dict(
        original_inputs={MODEL: hashlib.sha256(before.encode()).hexdigest()},
        mutation=mutation, production_modified=False, private_refactor=False,
        original_body_unchanged=mutation is None)
