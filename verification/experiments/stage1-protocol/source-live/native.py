"""Check five real live_native native views; official acceptance belongs to ning."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

import support

HERE = Path(__file__).resolve().parent
PREFIX = 'proof_source_live_'
FAILURES = {'original': set(), 'skip-version': {PREFIX + 'version'},
            'allow-expired': {PREFIX + 'deadline'},
            'ignore-startup': {PREFIX + 'startup'}, 'restored': set()}


def test_names():
    names = re.findall(r'^    fn (proof_source_live_\w+)\(',
                       (HERE / 'native-tests.rs.in').read_text(), re.M)
    support.require(len(names) == 11 and len(set(names)) == 11, 'actual assertion inventory differs')
    return set(names)


def assertions(log, failures, project, code):
    text = log.read_text()
    tests = test_names()
    starts = list(re.finditer(r'^test (\S+) \.\.\. ', text, re.M))
    support.require(len(starts) == len(tests), 'missing or extra actual native tests')
    stops = [x for x in (text.find('\nfailures:'), text.find('\ntest result:')) if x >= 0]
    support.require(stops, 'native test summary missing')
    stop = min(stops)
    outcomes = []
    for i, start in enumerate(starts):
        end = starts[i + 1].start() if i + 1 < len(starts) else stop
        status = re.search(r'(?:^|\s)(ok|FAILED|ignored)\Z', text[start.end():end].rstrip())
        support.require(status is not None, 'native test outcome missing')
        outcomes.append((start.group(1).split('::')[-1], status.group(1)))
    support.require({n for n, _ in outcomes} == tests and len({n for n, _ in outcomes}) == len(tests),
                    'same complete assertion inventory not reached')
    support.require({n for n, s in outcomes if s == 'FAILED'} == failures and
                    all(s != 'ignored' for _, s in outcomes), 'exact native failure set differs')
    support.require(code == (101 if failures else 0) and
                    f'{len(tests)-len(failures)} passed; {len(failures)} failed; 0 ignored;' in text,
                    'compiler/crash/skipped outcome is not a property rejection')
    sites = {}
    source = project / support.SOURCE
    for name in failures:
        marker = '::' + name + ' stdout ----'
        support.require(text.count(marker) == 1, 'actual property failure block missing')
        block = text.split(marker, 1)[1].split('\n---- ', 1)[0]
        found = re.search(r'panicked at ([^\n:]+):(\d+):(\d+):\n([^\n]+)', block)
        support.require(found is not None, 'actual panic location missing')
        file, line, column, message = found.groups()
        support.require(file == support.SOURCE, 'failure is outside the same actual contract test')
        lines = source.read_text().splitlines()
        index = int(line) - 1
        support.require(0 <= index < len(lines), 'native failure line outside original source')
        owners = re.findall(r'^    fn (proof_source_live_\w+)\(', '\n'.join(lines[:index]), re.M)
        support.require(owners and owners[-1] == name, 'setup failure is not property rejection')
        support.require(lines[index].lstrip().startswith('assert!(!live_native(') and
                        message.startswith('assertion failed: !live_native('),
                        'failure did not reach the actual rejection assertion')
        sites[name] = dict(source=file, line=int(line), column=int(column), message=message,
                           source_sha256=support.sha(source))
    return dict(native_assertions_checked=True, property_rejected=bool(failures),
                exact_failures=sorted(failures), assertion_sites=sites)


def check_only(root):
    head, inventory = support.source_guard(root)
    original = (root / support.SOURCE).read_text()
    rows = {}
    for view in FAILURES:
        if view == 'restored':
            body = support.restore(support.mutate(original, 'ignore-startup'), 'ignore-startup')
        else:
            body = support.mutate(original, view)
        rows[view] = dict(sha256=hashlib.sha256(body.encode()).hexdigest(), tests=sorted(test_names()),
                          expected_failure_tests=sorted(FAILURES[view]), compiled=False)
    support.require(rows['original']['sha256'] == rows['restored']['sha256'], 'restoration differs')
    return dict(kind='source-live-native-source-admission-only', complete_source_preparation=True,
                source_head=head, source_base=support.BASE, views=rows,
                native_assertions_executed=False, ning_acceptance=False, shared_acceptance=False,
                full_v3=False)


def run(root, output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='source-live-native-development', complete=False, commands=[], views={},
                  ning_acceptance=False, shared_acceptance=False, full_v3=False)
    def command(project, folder, view, label, argv, env):
        log = folder / (label + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1800).returncode
        record['commands'].append(dict(view=view, label=label, argv=argv, cwd=str(project),
            exit_code=code, seconds=time.monotonic()-begin, log=str(log), log_sha256=support.sha(log)))
        (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
        return code, log

    try:
        support.require(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'native',
                        'native execution belongs to the existing native-host suite')
        base = support.module('source_live_original_snapshot', root /
                              'verification/experiments/stage1-protocol/schema-leaves/formal.py')
        registry = support.module('source_live_original_registry', root /
                                  'verification/experiments/stage1-protocol/envelope-writer/native.py')
        registry.HERE = Path('.')
        legacy = support.module('source_live_original_native_artifacts', root /
                                'verification/experiments/stage1-protocol/source-origin/native.py')
        support.require(1 <= len(os.sched_getaffinity(0)) <= 4, 'native CPU allocation differs')
        head, _ = support.source_guard(root)
        inputs = base.snapshot(root)
        modes = {n: support.mode(root / n) for n in inputs}
        libraries = registry.registry()
        tools = {}
        for name in ('rustc', 'cargo', 'rustfmt', 'clippy-driver'):
            path = Path(subprocess.check_output(['rustup', 'which', name], cwd=root, text=True).strip())
            tools[name] = dict(path=str(path), sha256=support.sha(path))
        original = (root / support.SOURCE).read_text()
        fragment = (HERE / 'native-tests.rs.in').read_text()
        support.require(fragment not in original, 'native tests already installed')
        record.update(source_head=head, source_base=support.BASE, inputs=inputs,
                      input_modes=modes, library_registry=libraries, tools=tools,
                      assertion_inventory=sorted(test_names()), cpus=sorted(os.sched_getaffinity(0)))
        last_wrong = None
        for view, failures in FAILURES.items():
            folder = output / view
            project = folder / 'source'
            project.mkdir(parents=True, mode=0o700)
            expected = dict(inputs)
            for name, digest in inputs.items():
                target = project / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(root / name, target)
                support.require(support.sha(target) == digest, 'input changed during copy')
            if view == 'restored':
                support.require(last_wrong is not None, 'actual last wrong source missing')
                body = support.restore(last_wrong, 'ignore-startup')
            else:
                body = support.mutate(original, view)
                if view == 'ignore-startup':
                    last_wrong = body
            source = project / support.SOURCE
            source.write_text(body + '\n' + fragment)
            expected[support.SOURCE] = support.sha(source)
            target = folder / 'target'
            support.require(not target.exists(), 'variant target already exists')
            env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR='never', PYTHONDONTWRITEBYTECODE='1')
            code, _ = command(project, folder, view, 'format',
                ['rustfmt', '--edition', '2024', '--check', support.SOURCE], env)
            support.require(code == 0, 'format failure: ' + view)
            code, build = command(project, folder, view, 'build',
                ['cargo', 'test', '-p', 'sandproof-controller-service', '--lib', '--no-run',
                 '--locked', '--offline', '--message-format=json'], env)
            support.require(code == 0, 'compile failure is not property rejection: ' + view)
            built = legacy.artifacts(build, project, target)
            program, = [x['executable'] for x in built if x['target']['name'] == 'sandproof_controller_service']
            support.require(program is not None, 'actual native test program missing')
            executable = Path(program)
            support.require(executable.is_file() and not executable.is_symlink() and
                            executable.resolve().is_relative_to(target.resolve()), 'test program is outside fresh target')
            with executable.open('rb') as stream:
                support.require(stream.read(4) == b'\x7fELF', 'actual Linux native ELF program missing')
            executable_sha256 = support.sha(executable)
            code, listing = command(project, folder, view, 'list', [program, '--list'], env)
            support.require(code == 0, 'actual program inventory failed')
            names = {x.removesuffix(': test').split('::')[-1] for x in listing.read_text().splitlines()
                     if x.endswith(': test') and PREFIX in x}
            support.require(names == test_names(), 'actual complete test inventory differs')
            code, log = command(project, folder, view, 'tests', [program, PREFIX, '--test-threads=1'], env)
            entry = assertions(log, failures, project, code)
            code, _ = command(project, folder, view, 'clippy',
                ['cargo', 'clippy', '-p', 'sandproof-controller-service', '--lib', '--tests',
                 '--locked', '--offline', '--', '-D', 'warnings'], env)
            support.require(code == 0, 'strict Clippy failed: ' + view)
            support.require(support.sha(executable) == executable_sha256, 'actual executed program changed')
            support.require(all(support.sha(project / n) == digest and support.mode(project / n) == modes[n]
                                for n, digest in expected.items()), 'prepared source or modes changed')
            entry.update(complete=True, source_inputs=expected, source_input_modes=modes,
                business_source_sha256=hashlib.sha256(body.encode()).hexdigest(),
                test_names=sorted(test_names()), target_initially_absent=True,
                actual_executable=dict(path=program, sha256=executable_sha256),
                artifacts=[dict(message=x, files={n: support.sha(Path(n)) for n in x['filenames']
                                                 if Path(n).is_file()}) for x in built])
            record['views'][view] = entry
        final_head, _ = support.source_guard(root)
        support.require(final_head == head, 'source revision changed during execution')
        support.require(base.snapshot(root) == inputs and registry.registry() == libraries and
                        all(support.mode(root / n) == m for n, m in modes.items()), 'root input/library changed')
        support.require(all(support.sha(Path(x['path'])) == x['sha256'] for x in tools.values()), 'native tool changed')
        support.require(all(support.sha(Path(x['log'])) == x['log_sha256'] for x in record['commands']), 'original log changed')
        support.require((output / 'original/source' / support.SOURCE).read_bytes() ==
                        (output / 'restored/source' / support.SOURCE).read_bytes(), 'restored actual Rust differs')
        record.update(complete=True, input_tool_library_and_log_postguards_complete=True)
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - started
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=support.ROOT)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    root, output = args.repo.resolve(), args.output.resolve()
    if args.check_only:
        output.mkdir(parents=True, mode=0o700)
        result = check_only(root)
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    else:
        cpus = sorted(os.sched_getaffinity(0))
        if len(cpus) > 4:
            os.sched_setaffinity(0, set(cpus[:4]))
        result = run(root, output)
    print(json.dumps({n: result.get(n) for n in ('complete', 'complete_source_preparation', 'seconds', 'error')}))
    raise SystemExit(0 if result.get('complete') or result.get('complete_source_preparation') else 1)
