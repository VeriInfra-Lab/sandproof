"""Exact minimum source naming candidate; no production file is written."""
from pathlib import Path
import hashlib
import json

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-digest')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def candidate(project, variant='actual'):
    data = json.loads((ROOT/HERE/'candidate.json').read_text())
    source = project/SOURCE
    original = source.read_text()
    if source.is_symlink() or sha(source) != data['original_sha256']:
        raise ValueError('requires the exact reviewed original codec source')
    old = data['original_method']
    position = original.index(old)
    if not (original.index('impl InboxLoadEffects for ') < position
            < original.index('pub(in crate::controller) fn load(')):
        raise ValueError('selected method is not the original standard Load field')
    if 'fn sha256_bytes(' in original:
        raise ValueError('another library naming candidate already exists')
    restoration = None
    if variant == 'original':
        prepared = original
    else:
        method = data['candidate_method']
        if variant in ('input-empty', 'nonempty-tail-zero'):
            method = json.loads((ROOT/HERE/'mutations.json').read_text())[variant]['method']
        elif variant not in ('actual', 'restored'):
            raise ValueError('unknown actual Rust view')
        prepared = original.replace(old, method, 1)+data['helper']
        if prepared.removesuffix(data['helper']).replace(method, old, 1) != original:
            raise ValueError('an unrelated original implementation changed')
        if variant == 'restored':
            wrong = json.loads((ROOT/HERE/'mutations.json').read_text())['nonempty-tail-zero']['method']
            mutated = original.replace(old, wrong, 1)+data['helper']
            source.write_text(mutated)
            restoration = dict(actual_wrong_sha256=sha(source), original_candidate_restored=True)
    source.write_text(prepared)
    return dict(variant=variant, source=SOURCE, original_sha256=data['original_sha256'],
        prepared_sha256=sha(source), exact_unrelated_bytes_preserved=True,
        restoration=restoration, production_adopted=False,
        private_method_naming_changed=variant != 'original'), {SOURCE: sha(source)}


def view(project, variant='actual'):
    if variant == 'original':
        changed = {}
        derivation = dict(variant=variant, production_method_unchanged=True)
    else:
        derivation, changed = candidate(project, variant)
    source = project/SOURCE
    text = source.read_text()
    if 'fn proof_stage1_load_digest(' in text:
        raise ValueError('the transparent root was already appended')
    source.write_bytes(source.read_bytes()+(ROOT/HERE/'root.rs.in').read_bytes())
    changed[SOURCE] = sha(source)
    return derivation, changed
