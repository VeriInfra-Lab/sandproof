"""Prepare views of the one adopted SHA delegate; mutate only private copies."""
from pathlib import Path
import hashlib
import json

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-digest')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
LEAF = 'controller-service/src/controller/lifecycle_inbox/sha256.rs'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def admitted_source(project):
    data = json.loads((ROOT/HERE/'candidate.json').read_text())
    source = project/SOURCE
    if (source.is_symlink() or sha(source) != data['adopted_source_sha256']
            or sha(project/LEAF) != data['adopted_leaf_sha256']):
        raise ValueError('requires the exact adopted codec and unique SHA leaf')
    text = source.read_text()
    if 'fn sha256_bytes(' in text:
        raise ValueError('a second production SHA implementation is forbidden')
    start = text.index('impl InboxLoadEffects for ')
    end = text.index('pub(in crate::controller) fn load(', start)
    if text[start:end].count(data['candidate_method']) != 1:
        raise ValueError('the actual standard Load delegate changed')
    return data, text, start, end


def candidate(project, variant='actual'):
    data, original, start, end = admitted_source(project)
    if variant not in ('original', 'actual', 'input-empty', 'nonempty-tail-zero', 'restored'):
        raise ValueError('unknown actual Rust view')
    source = project/SOURCE
    block = original[start:end]
    method = data['candidate_method']
    mutation = None
    restoration = None
    if variant in ('input-empty', 'nonempty-tail-zero'):
        mutation = json.loads((ROOT/HERE/'mutations.json').read_text())[variant]['method']
        changed_block = block.replace(method, mutation, 1)
        prepared = original[:start]+changed_block+original[end:]
        if prepared[:start] != original[:start] or prepared[start+len(changed_block):] != original[end:]:
            raise ValueError('unrelated current production bytes changed')
    else:
        prepared = original
        if variant == 'restored':
            wrong = json.loads((ROOT/HERE/'mutations.json').read_text())['nonempty-tail-zero']['method']
            source.write_text(original[:start]+block.replace(method, wrong, 1)+original[end:])
            restoration = dict(actual_wrong_sha256=sha(source), exact_adopted_source_restored=True)
    source.write_text(prepared)
    return dict(variant=variant, source=SOURCE, original_sha256=data['adopted_source_sha256'],
        prepared_sha256=sha(source), exact_unrelated_bytes_preserved=True,
        restoration=restoration, mutation=mutation, production_adopted=True,
        actual_unique_leaf=LEAF, private_business_replacement=False), {SOURCE: sha(source)}


def view(project, variant='actual'):
    derivation, changed = candidate(project, variant)
    source = project/SOURCE
    if 'fn proof_stage1_load_digest(' in source.read_text():
        raise ValueError('the transparent root was already appended')
    source.write_bytes(source.read_bytes()+(ROOT/HERE/'root.rs.in').read_bytes())
    changed[SOURCE] = sha(source)
    return derivation, changed
