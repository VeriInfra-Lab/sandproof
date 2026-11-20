"""Reuse the record candidate and unchanged query body in one private source view."""
import importlib.util
from pathlib import Path

HERE = Path('verification/experiments/stage1-protocol/inbox-position')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
SDK = 'controller-service/src/controller/lifecycle_inbox/renewal_confirmation.rs'
VARIANTS = ('original', 'candidate', 'always-current', 'drop-operation', 'restored')
MUTATIONS = {'always-current', 'drop-operation'}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def prepare(project, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown position variant')
    record = module('position_record_prepare', project/HERE.parent/'inbox-record/prepare.py')
    state = module('position_state_prepare', project/HERE.parent/'inbox-state/prepare.py')
    before = (project/MODEL).read_text()
    start = before.index('    pub(super) fn state(')
    end = before.index('\n    pub(super) fn ', start+1)
    original_state = before[start:end]
    record.prepare(project, 'original' if variant == 'original' else 'candidate')
    text = (project/MODEL).read_text()
    start = text.index('    pub(super) fn state(')
    end = text.index('\n    pub(super) fn ', start+1)
    if text[start:end] != original_state or 'fn proof_state(' in text:
        raise ValueError('record preparation changed the original query body or added its root')
    (project/MODEL).write_text(text+'\n'+(project/HERE.parent/'inbox-state/roots.rs.in').read_text())
    _, mutation = state.prepare(project, variant if variant in MUTATIONS else 'original')
    return {name:state.sha(project/name) for name in (MODEL, SDK)}, mutation
