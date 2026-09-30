"""Reject apparent browser passes when the original verifier swallowed errors."""
import json
import re


def audit_browser_evidence(stdout, lifecycle):
    declared = re.findall(r'Total batches to test: (\d+)', stdout)
    batches = [int(x) for x in re.findall(r'RUN BATCH (\d+) ', stdout)]
    successes = stdout.count('  No alert detected')
    errors = {key: stdout.count(marker) for key, marker in {
        'timeout_treated_as_pass': ' - treating as passed',
        'driver_error': '  Failed to create driver or process file:',
        'load_error': '  Failed to load URL:',
        'alert_check_error': '  Error checking for alert:',
    }.items()}
    records = [json.loads(line) for line in lifecycle.splitlines() if line.strip()]
    expected = int(declared[0]) if len(declared) == 1 else None
    complete = (expected is not None and expected > 0
                and batches == list(range(expected)) and successes == expected
                and not any(errors.values()) and len(records) == expected
                and all(row.get('remaining_owned_children') == [] for row in records))
    return {'complete': complete, 'declared_batches': expected,
            'observed_batches': len(batches), 'successful_no_alert_checks': successes,
            'browser_lifecycle_records': len(records), 'swallowed_errors': errors,
            'scope': 'Additional evidence gate; original reward and tests are not modified.'}
