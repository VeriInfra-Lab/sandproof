"""Exact expression mutations of the adopted load implementation."""
MUTATIONS={
 'wrong-digest':('effects.digest(&bytes).as_slice() != &*sha',
                 'effects.digest(&bytes).as_slice().len() != sha.len()'),
 'wrong-canonical':('effects.encode_inbox(&inbox)? != bytes',
                    'effects.encode_inbox(&inbox)?.len() != bytes.len()'),
 'wrong-validation':('inbox.validate_using::<R, T, N, U, B>()?;',
                     'let _ = inbox.validate_using::<R, T, N, U, B>();'),
 'wrong-policy':('u64::from(inbox.config.max_operations_per_session)',
                'u64::from(inbox.config.max_sessions)'),
}

FAILURES={
 'wrong-digest': ['invalid_syntax_unknown_fields_and_wrong_digest_are_rejected_by_actual_load',
                  'digest_rejection_precedes_reencoding_and_second_configuration'],
 'wrong-canonical':['equal_length_reordered_json_is_not_canonical_even_with_matching_digest'],
 'wrong-validation':['valid_canonical_encoding_and_digest_still_require_full_inbox_validation',
                     'complete_inbox_validation_still_precedes_second_configuration'],
 'wrong-policy':['complete_value_can_still_exceed_the_native_operation_policy',
                 'final_operation_bound_uses_the_second_valid_configuration'],
}
