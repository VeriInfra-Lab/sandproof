"""Actual config validation candidates; production inputs remain read-only."""
from pathlib import Path
import hashlib

HERE = Path('verification/experiments/stage1-protocol/inbox-config-validation')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
CAPTURE = 'controller-service/src/capture.rs'
ORIGINAL = '    pub fn valid(&self) -> bool {\n        self.center.deployment == self.node.deployment_id\n            && self\n                .host_epoch\n                .as_ref()\n                .is_none_or(crate::capture::HostEpochRegistration::valid)\n            && (1..=4096).contains(&self.max_sessions)\n            && (8..=128).contains(&self.max_operations_per_session)\n            && (1..=65_536).contains(&self.max_options_bytes)\n            && (1..=65_536).contains(&self.max_profile_bytes)\n            && (1..=262_144).contains(&self.max_manifest_bytes)\n            && self.required_bytes() <= 16 * 1024 * 1024\n    }\n'
CANDIDATE = '    #[must_use]\n    pub fn valid(&self) -> bool {\n        self.center.deployment == self.node.deployment_id\n            && match self.host_epoch.as_ref() {\n                None => true,\n                Some(registration) => registration.valid(),\n            }\n            && (1..=4096).contains(&self.max_sessions)\n            && (8..=128).contains(&self.max_operations_per_session)\n            && (1..=65_536).contains(&self.max_options_bytes)\n            && (1..=65_536).contains(&self.max_profile_bytes)\n            && (1..=262_144).contains(&self.max_manifest_bytes)\n            && self.required_bytes() <= 16 * 1024 * 1024\n    }\n'
MUTATIONS = {
    'host-version-range': (CAPTURE, 'self.version == 1\n            && !self.boot_id.is_empty()',
                           'self.version <= 2\n            && !self.boot_id.is_empty()'),
    'omit-profile-bound': (MODEL, '            && (1..=65_536).contains(&self.max_profile_bytes)\n', ''),
    'double-budget-limit': (MODEL, '&& self.required_bytes() <= 16 * 1024 * 1024',
                          '&& self.required_bytes() <= 32 * 1024 * 1024'),
}
VARIANTS = ('original', 'candidate', *MUTATIONS, 'restored')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown config validation variant')
    paths = {n: (project/n).read_text() for n in [MODEL, CAPTURE]}
    original_inputs = {n: hashlib.sha256(text.encode()).hexdigest() for n, text in paths.items()}
    before = paths[MODEL]
    if before.count(ORIGINAL) != 1 or 'fn proof_inbox_config_valid(' in before:
        raise ValueError('nonunique config validator or existing root')
    rewritten = variant not in ('original', 'restored')
    paths[MODEL] = before.replace(ORIGINAL, CANDIDATE, 1) if rewritten else before
    mutation = None
    if variant in MUTATIONS:
        name, old, new = MUTATIONS[variant]
        prior = paths[name]
        if prior.count(old) != 1:
            raise ValueError('nonunique config mutation')
        paths[name] = prior.replace(old, new, 1)
        mutation = dict(path=name, old=old, new=new,
            before=hashlib.sha256(prior.encode()).hexdigest(),
            after=hashlib.sha256(paths[name].encode()).hexdigest())
    paths[MODEL] += '\n'+(project/HERE/'roots.rs.in').read_text()
    for name, text in paths.items():
        (project/name).write_text(text)
    return {n: sha(project/n) for n in paths}, dict(
        original_inputs=original_inputs,
        mutation=mutation, production_modified=False, private_refactor=rewritten,
        original_equivalence_proved=False, original_body_unchanged=not rewritten)
