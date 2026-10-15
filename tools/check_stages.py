#!/usr/bin/env python3
"""Bounded diagnostic receipts for existing checks; never runs or retries them."""
import argparse
import json
import math
import os
from pathlib import Path
import re
import stat
import sys
import time

EVENT_LIMIT = 128 * 1024
LOG_LIMIT = 16 * 1024 * 1024
KINDS = ('setup', 'contract', 'build', 'test', 'lint', 'documentation')
NAME = re.compile(r'[a-z][a-z0-9-]{0,79}\Z')
PREFIX = '[check-stage] '


def read_bounded(path, limit):
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 'rb') as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            raise ValueError('diagnostic input must be a bounded regular file')
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError('diagnostic input exceeds byte budget')
    return data


def load(path):
    data = read_bounded(path, EVENT_LIMIT)
    if not data.endswith(b'\n'):
        raise ValueError('incomplete diagnostic receipt')
    events = [json.loads(line) for line in data.splitlines()]
    if not events or not all(isinstance(event, dict) for event in events):
        raise ValueError('invalid diagnostic records')
    if events[0].get('event') != 'run-start' or type(events[0].get('schema')) is not int or events[0]['schema'] != 1:
        raise ValueError('missing diagnostic session')
    active = None
    names = set()
    ended = False
    previous = 0
    for event in events:
        at = event.get('at_ns')
        if type(at) is not int or not previous <= at <= 2**63 - 1 or ended:
            raise ValueError('invalid diagnostic order')
        previous = at
        kind = event.get('event')
        if kind == 'run-start':
            if event is not events[0] or event.get('mode') not in ('linux', 'portable'):
                raise ValueError('duplicate or invalid diagnostic session')
        elif kind == 'stage-start':
            name = event.get('stage', '')
            if active or not isinstance(name, str) or not NAME.fullmatch(name) or name in names:
                raise ValueError('invalid or duplicate stage')
            if event.get('kind') not in KINDS:
                raise ValueError('invalid stage kind')
            names.add(name)
            active = name
        elif kind == 'stage-end':
            if not active or event.get('stage') != active or type(event.get('exit_code')) is not int:
                raise ValueError('end does not match active stage')
            active = None
        elif kind == 'run-end':
            if active or type(event.get('exit_code')) is not int:
                raise ValueError('run ended without stage result')
            ended = True
        else:
            raise ValueError('unknown diagnostic event')
    return events


def append(path, event):
    data = (json.dumps(event, separators=(',', ':')) + '\n').encode()
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW | os.O_NONBLOCK), 'wb') as output:
        info = os.fstat(output.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size + len(data) > EVENT_LIMIT:
            raise ValueError('diagnostic receipt exceeds byte budget')
        output.write(data)
    # Only fixed identifiers, numbers and classifications enter public output.
    # These records are diagnostic, not durable process/cleanup receipts.
    print(PREFIX + json.dumps(event, separators=(',', ':')), file=sys.stderr, flush=True)


def initialize(path, mode):
    with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), 'wb') as output:
        event = {'schema': 1, 'event': 'run-start', 'mode': mode, 'at_ns': time.monotonic_ns()}
        output.write((json.dumps(event) + '\n').encode())


def start(path, stage, kind):
    events = load(path)
    if events[-1]['event'] not in ('run-start', 'stage-end') or events[-1].get('exit_code', 0):
        raise ValueError('another stage is still active or run already ended')
    if not NAME.fullmatch(stage) or any(event.get('stage') == stage for event in events):
        raise ValueError('invalid or repeated stage')
    print(f'::group::{stage} ({kind})', file=sys.stderr, flush=True)
    append(path, {'event': 'stage-start', 'stage': stage, 'kind': kind, 'at_ns': time.monotonic_ns()})


def finish(path, stage, code):
    events = load(path)
    if events[-1]['event'] != 'stage-start' or events[-1]['stage'] != stage:
        raise ValueError('no matching active stage')
    append(path, {'event': 'stage-end', 'stage': stage, 'exit_code': code, 'at_ns': time.monotonic_ns()})
    print('::endgroup::', file=sys.stderr, flush=True)


def end_run(path, code):
    events = load(path)
    if events[-1]['event'] not in ('run-start', 'stage-end'):
        raise ValueError('cannot finish an active or completed run')
    if not code and (len(events) == 1 or any(event.get('exit_code', 0) for event in events)):
        raise ValueError('cannot report success after missing or failed checks')
    append(path, {'event': 'run-end', 'exit_code': code, 'at_ns': time.monotonic_ns()})


def report(events):
    rows = []
    active = None
    for event in events:
        if event['event'] == 'stage-start':
            active = event
        elif event['event'] == 'stage-end':
            rows.append({'stage': active['stage'], 'kind': active['kind'],
                         'seconds': round((event['at_ns'] - active['at_ns']) / 1e9, 3),
                         'exit_code': event['exit_code']})
            active = None
    complete = events[-1]['event'] == 'run-end'
    return {'schema': 1, 'mode': events[0]['mode'], 'complete': complete,
            'exit_code': events[-1]['exit_code'] if complete else None,
            'unfinished_stage': active['stage'] if active else None, 'stages': rows}


def markdown(result):
    lines = ['## 检查阶段耗时', '', '| 阶段 | 类型 | 秒 | 原退出码 |', '|---|---|---:|---:|']
    for row in result['stages']:
        lines.append(f'| {row["stage"]} | {row["kind"]} | {row["seconds"]:.3f} | {row["exit_code"]} |')
    if not result['complete']:
        lines += ['', '未确认完整结束；最后活动阶段：' + (result['unfinished_stage'] or '未知') + '。']
    else:
        lines += ['', f'原检查退出码：{result["exit_code"]}；模式：{result["mode"]}。']
    lines += ['', '仅作诊断，不重试、不替代 Actions 结论；未结束不等于断言失败，退出码 124 也不能单独证明超时。']
    return '\n'.join(lines) + '\n'


# Text observations are triage hints, never authoritative failure causes or
# permission to rerun an effect. Successful logs may contain negative examples.
SIGNALS = {
    'network': r'Could not resolve host|Temporary failure in name resolution|Failed to connect to ',
    'dependency_or_material': r'failed to download|failed to get .+ as a dependency|no matching package named|No solution found when resolving dependencies|Redis material directory is missing',
    'test_timeout': r'subprocess\.TimeoutExpired|subprocess timeout|TimeoutError:',
    'assertion': r'AssertionError|assertion .+ failed|assertion failed:',
    'cleanup': r'CLEANUP_(?:FAILED|UNKNOWN)\b|ERROR: tearDown(?:Class|Module)?\b|cleanup_(?:state|status)["\x27]?\s*[:=]\s*["\x27]?(?:failed|unknown)',
}


def analyze(text, conclusion):
    hints = []
    if conclusion != 'success':
        hints = [name for name, pattern in SIGNALS.items() if re.search(pattern, text, re.IGNORECASE)]
        if conclusion == 'timed_out':
            hints.insert(0, 'job_timeout')
        elif conclusion == 'cancelled':
            hints.insert(0, 'interrupted')
        if not hints:
            hints.append('unclassified_failure')
    groups = []
    current = None
    stage = None
    builds = []
    for line in text.splitlines():
        # Do not parse a marker-like suffix of an oversized/untrusted line.
        if len(line) > 8192:
            continue
        if PREFIX in line:
            try:
                event = json.loads(line.split(PREFIX, 1)[1])
                if isinstance(event, dict) and event.get('event') == 'stage-start' and NAME.fullmatch(event.get('stage', '')):
                    stage = event['stage']
            except (TypeError, ValueError):
                pass
        build = re.search(r'Finished `([^`]+)` profile .* in ((?:[0-9]+m\s*)?[0-9.]+s)\s*$', line)
        if build:
            duration = re.fullmatch(r'(?:(\d+)m\s*)?([0-9]+(?:\.[0-9]+)?)s', build[2])
            if duration:
                seconds = int(duration[1] or 0) * 60 + float(duration[2])
                if math.isfinite(seconds):
                    builds.append({'stage': stage, 'profile': build[1][:80], 'seconds': seconds})
                    builds = sorted(builds, key=lambda item: item['seconds'], reverse=True)[:10]
        running = re.search(r'Running .+? \(([^)]+)\)', line)
        if running:
            current = Path(running[1]).name
        result = re.search(r'test result: (ok|FAILED)\..*finished in ([0-9.]+)s', line)
        if current and result:
            seconds = float(result[2])
            if math.isfinite(seconds):
                groups.append({'stage': stage, 'target': current[:160], 'seconds': seconds, 'status': result[1]})
                groups = sorted(groups, key=lambda item: item['seconds'], reverse=True)[:10]
            current = None
    return {'schema': 1, 'acceptance': 'not_evaluated', 'job_conclusion': conclusion,
            'possible_causes': hints, 'slow_cargo_build_reports': builds, 'slow_rust_targets': groups,
            'note': 'Text signals can overlap; confirm against the original error and cleanup evidence. No automatic retry.'}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ('init', 'start', 'finish', 'end', 'summary'):
        sub = commands.add_parser(name)
        sub.add_argument('--events', type=Path, required=True)
        if name == 'init':
            sub.add_argument('--mode', choices=('linux', 'portable'), required=True)
        if name in ('start', 'finish'):
            sub.add_argument('--stage', required=True)
        if name == 'start':
            sub.add_argument('--kind', choices=KINDS, required=True)
        if name in ('finish', 'end'):
            sub.add_argument('--exit-code', type=int, required=True)
        if name == 'summary':
            sub.add_argument('--github-summary', type=Path)
    sub = commands.add_parser('analyze-log')
    sub.add_argument('--log', type=Path, required=True)
    sub.add_argument('--conclusion', choices=('success', 'failure', 'cancelled', 'timed_out'), required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == 'init':
            initialize(args.events, args.mode)
        elif args.command == 'start':
            start(args.events, args.stage, args.kind)
        elif args.command == 'finish':
            finish(args.events, args.stage, args.exit_code)
        elif args.command == 'end':
            end_run(args.events, args.exit_code)
        elif args.command == 'summary':
            result = report(load(args.events))
            print(markdown(result))
            if args.github_summary:
                with args.github_summary.open('a', encoding='utf-8') as output:
                    output.write(markdown(result))
        else:
            result = analyze(read_bounded(args.log, LOG_LIMIT).decode('utf-8', errors='replace'), args.conclusion)
            print(json.dumps(result, indent=2, ensure_ascii=False))
    except (OSError, ValueError, KeyError, TypeError) as error:
        # No arbitrary paths, raw commands, tokens or child output are echoed.
        print('check diagnostics failed: ' + type(error).__name__, file=sys.stderr)
        return 70
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
