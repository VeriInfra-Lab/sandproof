"""Collect development diagnostics from the original registered Lean service.

This is not a public acceptance entry. It preserves inherited warnings and
rejects errors, partial/stale results, missing files and missing dependency audits.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess

HERE = Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def diagnose(project, output, lock):
    plan = json.loads((project / 'plan.json').read_bytes())
    before = {name: sha(project / name) for name in plan['modules']}
    if any(before[name] != entry['sha256'] for name, entry in plan['modules'].items()):
        raise ValueError('materialized source differs from its exact plan')
    names = [name.replace('.', '/') + '.lean' for name in plan['build_order']]
    if set(names) != set(before):
        raise ValueError('physical diagnostics do not cover the whole source closure')
    requests = [dict(tool='lean_diagnostic_messages', arguments=dict(file_path=name, timeout_s=180),
                     expected_sha256=before[name]) for name in names]
    payload = ''.join(json.dumps(row) + '\n' for row in requests) + '{"tool":"exit"}\n'
    output.mkdir(mode=0o700)
    (output / 'requests.jsonl').write_text(payload)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    result = dict(development_only=True, public_acceptance=False, complete=False,
                  physical_files=len(names), warnings=[], errors=[], audited=[], rejected=[])
    try:
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with (output / 'service.log').open('x') as log:
                child = subprocess.Popen([str(project / '.tools/lean-mcp/bin/python'), 'tools/lean_session.py'],
                    cwd=project, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, text=True,
                    env=dict(os.environ, LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536'), start_new_session=True)
                try:
                    child.communicate(payload, timeout=3600)
                except BaseException:
                    os.killpg(child.pid, signal.SIGTERM)
                    child.wait(timeout=10)
                    raise
                if child.returncode:
                    raise ValueError('original service client failed')
        records = []
        for line in (output / 'service.log').read_text().splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get('request', {}).get('tool') == 'lean_diagnostic_messages':
                records.append(row)
        if len(records) != len(requests):
            raise ValueError('missing actual physical-file responses')
        for request, row in zip(requests, records):
            name = request['arguments']['file_path']
            if (row.get('request') != request or row.get('stale') is not False
                    or row['revision']['file_sha256'] != before[name]):
                raise ValueError('changed or stale source: ' + name)
            response = row['result']
            for _ in range(3):
                if 'success' in response:
                    break
                response = response.get('structuredContent', response.get('result', {}))
            if (response.get('success') is not True or response.get('partial') is not False
                    or response.get('timed_out') is not False or response.get('failed_dependencies')
                    or response.get('still_elaborating_lines') not in (None, [])):
                result['rejected'].append(dict(file=name, response=response))
            expected = set(re.findall(r'^#audit_controller_proof (\S+)$', (project / name).read_text(), re.M))
            audited = set()
            for item in response.get('items', []):
                severity = item.get('severity')
                if severity in ('warning', 'error'):
                    result['warnings' if severity == 'warning' else 'errors'].append(dict(file=name, diagnostic=item))
                message = str(item.get('message', item.get('data', '')))
                match = re.fullmatch(r'Checked ([\w.]+): \d+ standard logical dependencies', message)
                if match:
                    audited.add(match[1])
                    result['audited'].append(dict(file=name, message=message))
            if expected != audited:
                result['rejected'].append(dict(file=name, missing_audits=sorted(expected - audited),
                                               extra_audits=sorted(audited - expected)))
        if before != {name: sha(project / name) for name in names}:
            raise ValueError('transitive source closure changed during service inspection')
        own = {path.name.removesuffix('.in') for path in HERE.glob('*.lean.in')}
        result['own_warnings'] = [item for item in result['warnings'] if item['file'] in own]
        result['complete'] = not (result['rejected'] or result['errors'] or result['own_warnings'])
        result['source_sha256'] = before
    except BaseException as error:
        result['error'] = repr(error)
    finally:
        result['service_log_sha256'] = sha(output / 'service.log') if (output / 'service.log').is_file() else None
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: value for key, value in result.items()
                     if key not in ('warnings', 'errors', 'audited', 'rejected', 'source_sha256')}, indent=2))
    print('warnings', len(result['warnings']), 'errors', len(result['errors']),
          'rejected', len(result['rejected']), 'audits', len(result['audited']))
    return not result['complete']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    args = parser.parse_args()
    raise SystemExit(diagnose(args.project.resolve(strict=True), args.output.resolve(), args.lock.resolve(strict=True)))
