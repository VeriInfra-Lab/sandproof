"""Opt-in guest probes after the original score; never repair or rescore tasks."""
import asyncio
import hashlib
import json


# The original outer test captures this pytest subprocess and omits its stdout
# from the assertion. The oracle already cloned these tests at this commit.
# Run that same suite directly, without another download or editing its files.
CYTHON_COMMAND = """set -eu
test "$(git rev-parse HEAD)" = 441c807dbec2ee32e1da572e24e58d52a4eb7afa
git diff --exit-code HEAD -- tests
git rev-parse HEAD
python --version
python -m pip list --format=json
python -m pytest tests --ignore tests/test_random_curves.py --ignore tests/test_catalogue.py -v --tb=long
"""
PROBES = {'build-cython-ext': {'name': 'pyknotid-repository-tests',
          'command': CYTHON_COMMAND, 'cwd': '/app/pyknotid', 'timeout_sec': 60}}


async def collect(environment, task_name, trial_dir):
    probe = PROBES.get(task_name)
    if probe is None:
        return {'status': 'not_applicable'}
    report = {k: v for k, v in probe.items() if k != 'command'}
    report.update(status='running', scope='post-score diagnostic; original reward unchanged',
                  command_sha256=hashlib.sha256(probe['command'].encode()).hexdigest())
    try:
        result = await asyncio.wait_for(environment.exec(probe['command'], cwd=probe['cwd'],
                                       timeout_sec=probe['timeout_sec']), probe['timeout_sec'])
        # A nonzero exit is useful diagnostic evidence, never a replacement score.
        report.update(status='completed', return_code=result.return_code)
        environment.retain_text(trial_dir / 'failure-diagnostic.txt',
                                (result.stdout or '') + (result.stderr or ''))
    except (asyncio.CancelledError, KeyboardInterrupt):
        report['status'] = 'interrupted'
        raise
    except Exception as error:
        report.update(status='error', error_type=type(error).__name__)
    finally:
        # Guest output and this report share the existing per-trial budget.
        # Report failure cannot prevent the caller's exact-VM cleanup.
        try:
            environment.retain_text(trial_dir / 'failure-diagnostic.json',
                                    json.dumps(report, indent=2) + '\n')
        except Exception as error:
            report['report_error_type'] = type(error).__name__
    return report
