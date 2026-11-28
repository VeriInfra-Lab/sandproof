"""Exact Rust composition mistakes; specifications and assertions remain shared."""
from pathlib import Path
import hashlib

SOURCE = 'controller-service/src/controller/lifecycle_inbox/model.rs'
MUTATIONS = {
    'wrong-owner-result': (
        '        self.validate_owners()?;',
        '        let _ = self.validate_owners();'),
    'wrong-lane-guard': (
        'if self.lanes.len() != usize::from(self.config.max_sessions)\n            || self.lanes_invalid::<T>(&context)',
        'if self.lanes.len() != usize::from(self.config.max_sessions)\n            && self.lanes_invalid::<T>(&context)'),
    'wrong-session-result': (
        '            self.validate_one_session::<R, T, N, U>(i, session, &context)?;',
        '            let _ = self.validate_one_session::<R, T, N, U>(i, session, &context);'),
}
FAILURES = {
    'wrong-owner-result': ['owner_table_checks_both_unique_fields_and_high_water'],
    'wrong-lane-guard': ['lane_count_and_slot_order_are_outer_invariants'],
    'wrong-session-result': [
        'local_membership_requires_one_matching_external_internal_pair',
        'current_terminal_and_incarnation_match_the_actual_entry_fold',
        'destroy_versions_are_strict_even_when_predecessor_adjacency_is_exempt',
        'duplicate_destroy_cannot_hide_behind_a_nonterminal_flag'],
}


def mutate(project, variant):
    path = project / SOURCE
    before = path.read_text()
    old, new = MUTATIONS[variant]
    if before.count(old) != 1 or new in before:
        raise ValueError('mutation must affect exactly one current actual Rust expression: '+variant)
    after = before.replace(old, new, 1)
    path.write_text(after)
    return dict(variant=variant, path=SOURCE, old=old, new=new,
                before_sha256=hashlib.sha256(before.encode()).hexdigest(),
                after_sha256=hashlib.sha256(after.encode()).hexdigest())
