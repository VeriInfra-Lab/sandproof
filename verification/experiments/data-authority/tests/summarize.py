#!/usr/bin/env python3
"""Read the approved Linux proof run's Cargo logs; reject false negatives."""
from pathlib import Path
import hashlib
import json
import re
import sys

batch = Path(sys.argv[1]).resolve()
evidence = batch / 'evidence'
expected = {
    'baseline': (10, 0, 0),
    'source': (10, 0, 0),
    'bad-digest': (9, 1, 101),
    'bad-expiry': (8, 2, 101),
    'bad-access': (9, 1, 101),
}
required_failures = {
    'bad-digest': ['da_redispatch_rechecks_the_actual_host_token_digest'],
    'bad-expiry': ['da_base_unknown_but_durable_expiry_does_not_grant_a_new_permit',
                   'da_redispatch_rechecks_the_exact_deadline_without_a_scanner'],
    'bad-access': ['da_redispatch_rechecks_closed_effective_authority_for_every_kind'],
}
results, observations = {}, {}
for label, (passed, failed, status) in expected.items():
    path = evidence / f'{label}-tests.log'
    raw = path.read_bytes()
    log = raw.decode()
    assert int((evidence / f'{label}-tests.exit').read_text()) == status
    assert 'running 10 tests' in log and 'Finished `test` profile' in log
    counts = re.findall(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', log)
    assert counts == [(str(passed), str(failed), '0')], (label, counts)
    assert 'error[E' not in log and 'could not compile' not in log
    if failed:
        actual = re.findall(r"thread 'data_authority_pilot_tests::([^']+)'[^\n]* panicked", log)
        assert sorted(actual) == sorted(required_failures[label]), (label, actual)
        if label == 'bad-expiry':
            assert 'left: 2\n right: 1' in log
        else:
            assert 'left: 1\n right: 0' in log
    observations[label] = re.findall(r'DA_RECEIPT [^\n]+', log)
    results[label] = {'passed': passed, 'failed': failed, 'ignored': 0, 'exit': status,
                      'test_log_sha256': hashlib.sha256(raw).hexdigest(),
                      'failed_tests': required_failures.get(label, [])}
assert len(observations['baseline']) == 17
assert observations['baseline'] == observations['source']
for label in ['baseline', 'source']:
    assert int((evidence / f'{label}-clippy.exit').read_text()) == 0
    log = (evidence / f'{label}-clippy.log').read_text()
    assert 'Finished `dev` profile' in log and 'error:' not in log
result = {'baseline': '0a23b57af60752f61067fc00b0b0b656e98347cb',
          'results': results, 'matching_observation_count': 17,
          'clippy': {'baseline': 0, 'source': 0},
          'claims': ['Real ControllerApplication and SQLite; controlled Host records spawn_data_relay then returns Busy',
                     'Closed is test-only fault injection, not a proved reachable durable history',
                     'Finite observations and source review support call binding; no general async equivalence claim']}
(evidence / 'business-test-result.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
