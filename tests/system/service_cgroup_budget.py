"""Independent Linux cgroup probe; no VM, credentials, or network changes."""

import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools import service_cgroup as cg


def line(process, timeout=20):
    if not select.select([process.stdout], [], [], timeout)[0]:
        raise TimeoutError('child readiness barrier')
    value = process.stdout.readline()
    if not value:
        raise RuntimeError('child exited before readiness')
    return json.loads(value)


def exercise(parent, evidence):
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain']).strip():
        raise ValueError('clean reviewed commit required')
    commit = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    cg.filesystem(parent)
    cg.private(evidence)
    if any(evidence.iterdir()):
        raise ValueError('new empty evidence directory required')
    if int(next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines()
                if line.startswith('MemAvailable:'))) < 512 * 1024:
        raise ValueError('512 MiB available host headroom required')
    record, sentinel_record = evidence / 'load.json', evidence / 'sentinel.json'
    tag = 'svc-probe-' + os.urandom(8).hex()
    result = {'commit': commit, 'status': 'running', 'scope': 'kernel enforcement only; no real service/relay/VM',
              'parent': str(parent), 'cases': [], 'records': [str(record), str(sentinel_record)]}
    children, records = [], []
    def save():
        (evidence / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
    def launch(record, program):
        p = subprocess.Popen([*cg.prefix(record), sys.executable, '-u', '-c', program],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        children.append(p)
        return p
    save()
    try:
        for destination, suffix, quota in ((record, 'load', 20000), (sentinel_record, 'sentinel', 10000)):
            cg.prepare(parent, tag + '-' + suffix, destination, cpu_quota_us=quota, memory_bytes=64 * 1024**2)
            records.append(destination)
        sentinel = launch(sentinel_record, 'import json,os,sys; print(json.dumps({"pid":os.getpid()})); sys.stdin.readline()')
        assert line(sentinel)['pid'] == sentinel.pid
        initial = cg.observe(sentinel_record, pids=[sentinel.pid])
        # Parent and its CPU-bound child must share the hard quota before the
        # child reports ready. Pipe handshakes, not timing guesses, start work.
        child = 'import json,os,sys; print(json.dumps({"pid":os.getpid()})); sys.stdin.readline();\nwhile True: pass'
        program = ('import json,os,signal,subprocess,sys,time\n'
                   'signal.signal(signal.SIGTERM,lambda *_:sys.exit(1))\n'
                   'p=None\ntry:\n'
                   f' p=subprocess.Popen([sys.executable,"-u","-c",{child!r}],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)\n'
                   ' child=json.loads(p.stdout.readline())\n'
                   ' print(json.dumps({"pid":os.getpid(),"child":child["pid"]}))\n'
                   ' sys.stdin.readline(); p.stdin.write("go\\n"); p.stdin.flush()\n'
                   ' end=time.monotonic()+2\n'
                   ' while time.monotonic()<end: pass\n'
                   'finally:\n'
                   ' if p is not None: p.terminate(); p.wait(timeout=5)\n')
        cpu = launch(record, program)
        ready = line(cpu)
        assert ready['pid'] == cpu.pid
        before = cg.observe(record, pids=[cpu.pid, ready['child']])
        with_error = False
        try:
            cg.remove(record)
        except ValueError:
            with_error = True
        assert with_error, 'live workload must block cleanup'
        cpu.stdin.write('go\n'); cpu.stdin.flush()
        assert cpu.wait(timeout=30) == 0
        after = cg.observe(record)
        assert after['cpu_stat']['nr_throttled'] > before['cpu_stat']['nr_throttled']
        assert after['cpu_stat']['throttled_usec'] > before['cpu_stat']['throttled_usec']
        result['cpu'] = {'before': before, 'after': after}
        result['cases'].append('inherited-child-membership-and-shared-cpu-throttling')
        # Lowering the limit is not necessary: allocate/touch beyond this
        # already verified 64 MiB group. Total attempted allocation is bounded.
        memory = launch(record, 'import json,os,sys; print(json.dumps({"pid":os.getpid()})); sys.stdin.readline(); x=bytearray(192*1024**2)')
        assert line(memory)['pid'] == memory.pid
        cg.observe(record, pids=[memory.pid])
        memory.stdin.write('go\n'); memory.stdin.flush()
        assert memory.wait(timeout=30) == -9
        killed = cg.observe(record)
        assert killed['memory_events']['oom_kill'] > after['memory_events'].get('oom_kill', 0)
        assert sentinel.poll() is None
        survived = cg.observe(sentinel_record, pids=[sentinel.pid])
        assert survived['memory_events'].get('oom_kill', 0) == initial['memory_events'].get('oom_kill', 0)
        result['memory'] = {'workload': killed, 'sibling': survived}
        result['cases'].append('memory-oom-contained-sibling-survives')
        again = launch(record, 'print("{\\"ok\\":true}")')
        assert line(again) == {'ok': True} and again.wait(timeout=10) == 0
        result['cases'].append('new-service-can-rejoin-same-record-after-oom')
        sentinel.stdin.write('stop\n'); sentinel.stdin.flush()
        assert sentinel.wait(timeout=10) == 0
        result['status'] = 'passed'
    except BaseException as error:
        result['status'], result['error_type'] = 'failed', type(error).__name__
        raise
    finally:
        errors = []
        for index, p in reversed(list(enumerate(children))):
            try:
                if p.poll() is None:
                    p.terminate()
                    p.wait(timeout=10)
                (evidence / f'child-{index}-stderr.txt').write_text(p.stderr.read(65536))
                for stream in (p.stdin, p.stdout, p.stderr):
                    stream.close()
            except Exception as error:
                errors.append(type(error).__name__)
        result['cleanup'] = []
        for record in reversed(records):
            try:
                result['cleanup'].append(cg.remove(record))
            except Exception as error:
                errors.append(type(error).__name__)
        if errors:
            result['status'] = 'cleanup_unknown'
            result['cleanup_errors'] = errors
        save()
        if errors:
            raise RuntimeError('retain exact cgroup records for recovery')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--parent', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(exercise(args.parent, args.evidence)))
