"""Unique-site mutations of the actual generic save body, with unchanged assertions."""
from pathlib import Path
import hashlib
import subprocess
SOURCE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'

MUTATIONS = {
    'wrong-validation': (
        '    inbox.validate_using::<R, T, N, U, B>()?;',
        '    let _ = inbox.validate_using::<R, T, N, U, B>();'),
    'wrong-budget': (
        '    if bytes.len() as u64 > inbox.config.required_bytes() {\n'
        '        return Err(JournalError::MaintenanceCapacity);\n    }\n', ''),
    'wrong-zero-row': ('    )? != 1', '    )? > 1'),
    'wrong-hash': ('    let digest = effects.digest(&bytes);', '    let digest = effects.digest(&[]);'),
}
FAILURES = {
    'wrong-validation': {
        'actual_save_rejects_the_invalid_poststate_before_replacing_the_row',
        'failed_validation_skips_all_save_effects'},
    'wrong-budget': {'exact_budget_passes_and_one_extra_byte_stops_before_digest'},
    'wrong-zero-row': {'zero_or_two_rows_are_errors_after_the_update_effect',
                       'actual_sqlite_save_requires_the_one_target_row'},
    'wrong-hash': {'saved_bytes_digest_input_sql_and_effect_order_are_complete',
                   'exact_budget_passes_and_one_extra_byte_stops_before_digest',
                   'zero_or_two_rows_are_errors_after_the_update_effect',
                   'update_error_preserves_prior_effects_and_the_original_error'},
}


def mutate(project, variant):
    old, new = MUTATIONS[variant]
    path = project/SOURCE
    source = path.read_text()
    start = source.index('pub(super) fn save_using<')
    end = source.index('\npub(in crate::controller) fn validate(', start)
    before, body, after = source[:start], source[start:end], source[end:]
    if body.count(old) != 1:
        raise ValueError('mutation no longer matches actual Rust: '+variant)
    path.write_text(before+body.replace(old,new,1)+after)
    subprocess.run(['rustfmt','--edition','2024',SOURCE],cwd=project,check=True)
    record = dict(variant=variant,path=SOURCE,old=old,new=new,
        before_sha256=hashlib.sha256(source.encode()).hexdigest(),
        after_sha256=hashlib.sha256(path.read_bytes()).hexdigest())
    if record['before_sha256']==record['after_sha256']:
        raise ValueError('mutation had no effect')
    return record
