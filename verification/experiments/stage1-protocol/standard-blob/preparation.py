"""Minimal read-expression candidate and extraction-only source annotations."""
from pathlib import Path
import hashlib
import json

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-blob')
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
READ_ROOT = '''

#[expect(dead_code, reason = "selected by fixed Charon")]
fn proof_stage1_load_blob_pair(
    receiver: &mut super::super::checkpoint::StandardConfigRead<'_>,
    statement: &str,
) -> Result<(Vec<u8>, Vec<u8>), JournalError> {
    InboxLoadEffects::query_bytes_pair(receiver, statement)
}
'''


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def candidate(project, variant='actual'):
    plan = json.loads((ROOT/HERE/'source-candidate.json').read_text())
    source = project/SOURCE
    original = source.read_text()
    if sha(source) != plan['baseline_source_sha256'] or original.count(plan['before']) != 1:
        raise ValueError('candidate requires the exact reviewed upstream read source')
    if 'fn read_blob_pair(' in original:
        raise ValueError('the read expression already has a different helper')
    named = plan['original_array_named_after'] if variant == 'array-params' else plan['after']
    prepared = original.replace(plan['before'], named, 1)+plan['helper_append']
    # Every byte outside this one expression and appended helper is preserved.
    if prepared.removesuffix(plan['helper_append']).replace(named, plan['before'], 1) != original:
        raise ValueError('candidate changed an unrelated implementation')
    mutations = json.loads((ROOT/HERE/'mutations.json').read_text())
    mutation = mutations.get(variant)
    restoration = None
    if variant == 'restored':
        wrong = mutations['repeat-first']
        if prepared.count(wrong['before']) != 1:
            raise ValueError('restoration must mutate the real unique helper')
        source.write_text(prepared.replace(wrong['before'], wrong['after'], 1))
        wrong_sha = sha(source)
        source.write_text(prepared)
        restoration = dict(wrong_source_sha256=wrong_sha, exact_candidate_restored=True)
    elif mutation:
        if prepared.count(mutation['before']) != 1:
            raise ValueError('real Rust helper mutation is not unique')
        source.write_text(prepared.replace(mutation['before'], mutation['after'], 1))
    elif variant in ('actual', 'array-params'):
        source.write_text(prepared)
    else:
        raise ValueError('unknown actual source view: '+variant)
    return dict(source=SOURCE, original_sha256=plan['baseline_source_sha256'],
        candidate_sha256=sha(source), variant=variant, mutation=mutation,
        restoration=restoration, unrelated_production_bytes_preserved=True,
        source_adopted=False), {SOURCE: sha(source)}


def append(project, relative, provider, marker):
    source = project/relative
    if marker in source.read_text():
        raise ValueError('the transparent root was already instrumented: '+marker)
    fragment = (ROOT/provider).read_bytes()
    source.write_bytes(source.read_bytes()+fragment)
    return sha(source)


def metadata_names(project):
    plan = json.loads((ROOT/HERE/'enum-names.json').read_text())
    if plan['enum_count'] != len(plan['mappings']) or plan['enum_count'] != 16:
        raise ValueError('incomplete original enum naming plan')
    before = {}
    registration = '#![feature(register_tool)]\n#![register_tool(charon)]\n'
    for entry in plan['mappings']:
        relative = entry['source']
        source = project/relative
        before.setdefault(relative, source.read_text())
        text = source.read_text()
        marker = 'pub enum '+entry['original_name']+' {'
        if text.count(marker) != 1:
            raise ValueError('original enum declaration changed: '+entry['rust_name'])
        annotation = '#[charon::rename("'+entry['extraction_name']+'")]\n'
        source.write_text(text.replace(marker, annotation+marker, 1))
    for crate in sorted({Path(e['source']).parts[0] for e in plan['mappings']}):
        relative = crate+'/src/lib.rs'
        source = project/relative
        before.setdefault(relative, source.read_text())
        source.write_text(registration+source.read_text())
    changed = {}
    for relative, original in before.items():
        source = project/relative
        restored = source.read_text().removeprefix(registration)
        for entry in plan['mappings']:
            if entry['source'] == relative:
                marker = 'pub enum '+entry['original_name']+' {'
                annotation = '#[charon::rename("'+entry['extraction_name']+'")]\n'
                if restored.count(annotation+marker) != 1:
                    raise ValueError('tool-name metadata inverse is not unique')
                restored = restored.replace(annotation+marker, marker, 1)
        if restored != original or original != (ROOT/relative).read_text():
            raise ValueError('name-only metadata changed an original type/function body')
        changed[relative] = sha(source)
    return dict(enum_count=16, all_original_files_byte_restored_by_inverse=True,
        type_and_function_bodies_unchanged=True, extra_opacity_added=False,
        plan_sha256=sha(ROOT/HERE/'enum-names.json')), changed


def read_view(project, variant):
    derivation, changed = candidate(project, variant)
    source = project/SOURCE
    if 'fn proof_stage1_load_blob_pair(' in source.read_text():
        raise ValueError('actual standard root already exists')
    source.write_text(source.read_text()+READ_ROOT)
    changed[SOURCE] = sha(source)
    metadata, named = metadata_names(project)
    changed.update(named)
    return dict(candidate=derivation, name_metadata=metadata), changed


def load_view(project):
    # Reuse only existing transparent wrappers around unchanged actual methods.
    candidate_record, candidate_changed = candidate(project, 'actual')
    providers = [
        (SOURCE, 'inbox-load/generic-root.rs.in', 'proof_inbox_load_using'),
        ('controller-service/src/controller/lifecycle_inbox/model.rs',
         'inbox-validation/generic-root.rs.in', 'proof_inbox_validate_using'),
        ('controller-service/src/capture.rs', 'inbox-validation/primitive-roots.rs.in',
         'proof_standard_graphic_bytes'),
        ('verification/experiments/stage1-protocol/inbox-record/lib.rs',
         'terminal-binding/roots.rs.in', 'terminal_matches'),
    ]
    changed = dict(candidate_changed)
    for relative, suffix, marker in providers:
        changed[relative] = append(project, relative, HERE.parent/suffix, marker)
    relative = 'verification/experiments/stage1-protocol/inbox-record/lib.rs'
    source = project/relative
    fragment = json.loads((ROOT/HERE.parent/'inbox-renewal-session/fragments.json').read_text())['renewal_roots']
    if 'fn renewal_valid(' in source.read_text():
        raise ValueError('renewal transparent roots already exist')
    source.write_text(source.read_text()+fragment)
    changed[relative] = sha(source)
    return dict(candidate=candidate_record, original_generic_load_body_unchanged=True,
        only_existing_transparent_roots_added=True), changed
