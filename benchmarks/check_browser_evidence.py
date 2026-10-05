"""Reject apparent browser passes when the original verifier swallowed errors."""
import json
import re


def audit_browser_execution(stdout):
    """Check the original verifier's browser work independently of its reward."""
    declared = re.findall(r'Total batches to test: (\d+)', stdout)
    batches = [int(x) for x in re.findall(r'RUN BATCH (\d+) ', stdout)]
    successes = stdout.count('  No alert detected')
    errors = {key: stdout.count(marker) for key, marker in {
        'timeout_treated_as_pass': ' - treating as passed',
        'driver_error': '  Failed to create driver or process file:',
        'load_error': '  Failed to load URL:',
        'alert_check_error': '  Error checking for alert:',
    }.items()}
    expected = int(declared[0]) if len(declared) == 1 else None
    complete = (expected is not None and expected > 0
                and batches == list(range(expected)) and successes == expected
                and not any(errors.values()))
    return {'complete': complete, 'declared_batches': expected,
            'observed_batches': len(batches), 'successful_no_alert_checks': successes,
            'swallowed_errors': errors, 'browser_lifecycle_audit': 'not_instrumented',
            'scope': ('Original browser execution evidence only; original reward retained. '
                      'Browser process cleanup is not established by stdout.')}


def audit_browser_evidence(stdout, lifecycle):
    report = audit_browser_execution(stdout)
    try:
        records = [json.loads(line) for line in lifecycle.splitlines() if line.strip()]
        valid = all(isinstance(row, dict) and row.get('remaining_owned_children') == []
                    for row in records)
    except ValueError:
        records, valid = [], False
    expected = report['declared_batches']
    lifecycle_complete = (valid and expected is not None and expected > 0
                          and len(records) == expected)
    complete = report['complete'] and lifecycle_complete
    return report | {'complete': complete, 'browser_lifecycle_records': len(records),
                     'browser_lifecycle_audit': 'passed' if lifecycle_complete else 'failed',
                     'scope': 'Additional execution and lifecycle evidence gate; original reward and tests are not modified.'}
