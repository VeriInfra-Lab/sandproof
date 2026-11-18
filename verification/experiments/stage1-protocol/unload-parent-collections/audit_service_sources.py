"""Read back which frozen Lean files each original service revision covered."""
import argparse
import hashlib
import json
from pathlib import Path

from registration_preflight import ROOT, collect, read_regular, sha

LEGACY_PREFIXES = ('SandboxModel/', 'verification/aeneas/')
DIAGNOSTICS = 'lean_diagnostic_messages'


def fingerprint(modules, paths):
    value = hashlib.sha256()
    for path in paths:
        name = path.encode()
        value.update(len(name).to_bytes(8, 'big'))
        value.update(name)
        value.update(bytes.fromhex(modules[path]))
    return value.hexdigest()


def service_rows(log):
    for line in log.decode().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if isinstance(row, dict) and row.get('request', {}).get('tool') == DIAGNOSTICS:
            yield row


def coverage(modules, plan_bytes, requests, log):
    plan = json.loads(plan_bytes)
    order = [name.replace('.', '/') + '.lean' for name in plan['build_order']]
    if not order or len(order) != len(modules) or set(order) != set(modules):
        raise ValueError('service source audit is missing the frozen request order')
    expected = [
        dict(tool=DIAGNOSTICS, arguments=dict(file_path=name, timeout_s=180),
             expected_sha256=modules[name]) for name in order
    ]
    if [json.loads(line) for line in requests.splitlines()] != expected + [{'tool': 'exit'}]:
        raise ValueError('service requests do not cover the exact frozen view in order')
    original = list(service_rows(log))
    if len(original) != len(expected):
        raise ValueError('missing or extra original service responses')

    all_files = sorted(modules, key=Path)
    legacy = [name for prefix in LEGACY_PREFIXES for name in all_files if name.startswith(prefix)]
    planned = 'source_plan_sha256' in original[0]['revision']
    tracked = all_files if planned else legacy
    wanted = fingerprint(modules, tracked)
    marker = sha(plan_bytes)
    for request, row in zip(expected, original):
        revision = row['revision']
        if (row['request'] != request or row['stale'] is not False
                or revision['file_sha256'] != request['expected_sha256']
                or revision['sources_sha256'] != wanted):
            raise ValueError('stale, changed or wrong-scope original service revision')
        if (planned and revision.get('source_plan_sha256') != marker) or (
                not planned and 'source_plan_sha256' in revision):
            raise ValueError('original service revision does not bind the same source plan')

    omitted = sorted(set(modules) - set(tracked), key=Path)
    return dict(requested_files=len(expected), fingerprinted_files=len(tracked),
                mode='materialized-plan' if planned else 'legacy-prefixes',
                full_closure_bound=not omitted, omitted_files=omitted,
                sources_sha256=wanted, expected_full_sources_sha256=fingerprint(modules, all_files))


def audit(root, view, project, diagnostic):
    current = collect(root, {view: project})['views'][view]
    modules = current['modules']
    raw_result = read_regular(diagnostic, 'result.json')
    result = json.loads(raw_result)
    log = read_regular(diagnostic, 'service.log')
    if (result.get('complete') is not True or result.get('source_sha256') != modules
            or result.get('physical_files') != len(modules)
            or result.get('service_log_sha256') != sha(log)):
        raise ValueError('incomplete diagnostic record or changed original service log')
    observed = coverage(modules, read_regular(project, 'plan.json'),
                        read_regular(diagnostic, 'requests.jsonl'), log)
    return dict(kind='unload-service-source-coverage', public_acceptance=False, proof_executed=False,
                view=view, service_log_sha256=sha(log), diagnostic_sha256=sha(raw_result), **observed)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--view', choices=('parent', 'correct', 'attempt-zero'), required=True)
    parser.add_argument('--project', type=Path, required=True)
    parser.add_argument('--diagnostics', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = audit(ROOT, args.view, args.project.resolve(strict=True), args.diagnostics.resolve(strict=True))
    with args.output.open('x') as stream:
        json.dump(report, stream, indent=2)
        stream.write('\n')
    raise SystemExit(0 if report['full_closure_bound'] else 2)
