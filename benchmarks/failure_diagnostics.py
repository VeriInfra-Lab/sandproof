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
# The pinned task installs GCC, but no system Python. Diagnose only the exact
# reviewed source; the temporary copy returns before model I/O or inference.
GPT2_SOURCE_SHA256 = '707b34cab0fc594717de7eff30101f9a64a53f6e7b45af17345a5bcb0d279cd8'
GPT2_COMMAND = r'''set -eu
expected=707b34cab0fc594717de7eff30101f9a64a53f6e7b45af17345a5bcb0d279cd8
printf '%s  gpt2.c\n' "$expected" | sha256sum --check --status
work=$(mktemp -d /tmp/sandproof-gpt2-allocation.XXXXXXXX)
cleanup() {
    status=$?
    trap - EXIT
    rm -rf -- "$work"
    if ! printf '%s  gpt2.c\n' "$expected" | sha256sum --check --status; then
        printf 'original_source_unchanged_after_probe=false\n'
        exit 80
    fi
    printf 'original_source_unchanged_after_probe=true\n'
    exit "$status"
}
trap cleanup EXIT
printf 'scope=post-score allocation-only diagnostic; original reward unchanged\n'
printf 'original_source_sha256=%s\n' "$expected"
for key in overcommit_memory overcommit_ratio overcommit_kbytes; do
    printf '%s=' "$key"
    cat "/proc/sys/vm/$key"
done
sed -n '/^MemTotal:/p; /^MemAvailable:/p; /^SwapTotal:/p; /^CommitLimit:/p; /^Committed_AS:/p' /proc/meminfo
cat /proc/self/limits
sed -e '/^#include<stdio.h>$/a\
#include<errno.h>' \
    -e '/^D=malloc(2LL\*V\*V\*C\*zz);$/c\
errno=0; D=malloc(2LL*V*V*C*zz);\
{int saved_errno=errno; fprintf(stderr,"allocation_D bytes=%lld success=%d errno=%d\\n",2LL*V*V*C*zz,D!=0,saved_errno);}\
if(!D)return 77;' \
    -e '/^bpe=malloc(1e9);$/c\
errno=0; bpe=malloc(1e9);\
{int saved_errno=errno; fprintf(stderr,"allocation_bpe bytes=1000000000 success=%d errno=%d\\n",bpe!=0,saved_errno);}\
if(!bpe){free(D);return 78;}\
free(bpe);free(D);return 0;' gpt2.c > "$work/guarded.c"
gcc -O3 "$work/guarded.c" -lm -o "$work/guarded"
"$work/guarded" gpt2-124M.ckpt vocab.bpe 'THIS SOFTWARE IS PROVIDED "AS IS", WITHOUT'
'''
PROBES = {
    'build-cython-ext': {'name': 'pyknotid-repository-tests',
                        'command': CYTHON_COMMAND, 'cwd': '/app/pyknotid', 'timeout_sec': 60},
    'gpt2-codegolf': {'name': 'gpt2-original-allocations',
                     'command': GPT2_COMMAND, 'cwd': '/app', 'timeout_sec': 60},
}


async def collect(environment, task_name, trial_dir, *, oracle_mode='original'):
    probe = PROBES.get(task_name)
    if probe is None:
        return {'status': 'not_applicable'}
    if task_name == 'gpt2-codegolf' and oracle_mode != 'original':
        return {'status': 'not_applicable', 'reason': 'probe_requires_original_oracle'}
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
