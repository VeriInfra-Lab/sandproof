"""Exercise original/adopted/wrong/restored Rust against one frozen original SourceWork."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


support = module('source_question_native_support', HERE / 'support.py')
sha, mode, require = support.sha, support.mode, support.require
SOURCE = support.SOURCE
TESTSOURCE = 'controller-service/src/controller/lifecycle_inbox/source_work/tests.rs'
PREFIX = 'proof_source_question_contract_'
PIPELINE = PREFIX + 'pipeline'
EXPORT = 'proof_source_question_export_fixture'
HELPER_TESTS = {PREFIX + n for n in ('full', 'binding', 'resource', 'limits', 'source',
                                    'node_bootstrap', 'empty_panics', 'single_panics')}
FAILURES = {'baseline': set(), 'correct': set(),
            'first-resource': {PREFIX + n for n in ('full', 'resource', 'single_panics', 'pipeline')},
            'options-binding': {PREFIX + n for n in ('full', 'binding', 'pipeline')},
            'disk-as-material': {PREFIX + n for n in ('full', 'limits', 'pipeline')},
            'restored': set()}


def artifacts(log, project, target):
    rows = []
    for line in log.read_text().splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if item.get('reason') == 'compiler-artifact' and item['target']['name'] in (
                'sandproof_protocol', 'sandproof_controller_service'):
            rows.append(item)
    require(len(rows) == 2 and all(x['fresh'] is False for x in rows),
            'actual protocol/service were not freshly built')
    for item in rows:
        package = 'sandbox-protocol' if item['target']['name'] == 'sandproof_protocol' else 'controller-service'
        require(item['manifest_path'] == str(project / package / 'Cargo.toml') and
                item['target']['src_path'] == str(project / package / 'src/lib.rs') and
                item['profile']['test'] == (package == 'controller-service'),
                'wrong manifest/source/program')
    protocol, = [x for x in rows if x['target']['name'] == 'sandproof_protocol']
    linked = list(target.glob('debug/deps/libsandproof_protocol-*.rlib'))
    require(len(linked) == 1 and str(linked[0]) in protocol['filenames'], 'wrong linked protocol library')
    return rows


def assertions(log, tests, failures, project, code):
    text = log.read_text()
    starts = list(re.finditer(r'^test (\S+) \.\.\. ', text, re.M))
    require(len(starts) == len(tests), 'missing actual native tests')
    stop = min(x for x in (text.find('\nfailures:'), text.find('\ntest result:')) if x >= 0)
    outcomes = []
    for i, start in enumerate(starts):
        end = starts[i + 1].start() if i + 1 < len(starts) else stop
        status = re.search(r'(?:^|\s)(ok|FAILED|ignored)\Z', text[start.end():end].rstrip())
        require(status is not None, 'actual test status missing')
        outcomes.append((start.group(1).split('::')[-1], status.group(1)))
    require({n for n, _ in outcomes} == tests and
            {n for n, status in outcomes if status == 'FAILED'} == failures,
            'same contract assertion set or exact failure set differs')
    require(code == (101 if failures else 0) and
            f'{len(tests)-len(failures)} passed; {len(failures)} failed; 0 ignored;' in text,
            'assertions not reached')
    sites = {}
    for name in failures:
        block = text.split('::' + name + ' stdout ----', 1)[1].split('\n---- ', 1)[0]
        found = re.search(r'panicked at ([^\n:]+):(\d+):(\d+):\n(.*?)(?=\nnote:|\nstack backtrace:|\n\nfailures:|\Z)', block, re.S)
        require(found is not None, 'no original property assertion')
        file, line, column, message = found.groups()
        require(file == SOURCE, 'failure outside the actual contract test')
        owners = re.findall(r'fn (proof_source_question_contract_\w+)\(',
                            '\n'.join((project / file).read_text().splitlines()[:int(line)-1]))
        require(owners and owners[-1] == name, 'fixture failure instead of property rejection')
        expected = ('assertion failed: outcome.is_err()' if name.endswith('single_panics') else
                    'assertion `left == right` failed')
        require(message.startswith(expected), 'failure did not reach the same field/index contract')
        sites[name] = dict(source=file, line=int(line), column=int(column), message=message,
                           source_sha256=sha(project / file))
    return dict(native_assertions_checked=True, exact_failures=sorted(failures),
                property_rejected=bool(failures), assertion_sites=sites)


def run(output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='actual-source-question-native-development', complete=False, commands=[], views={},
                  production_source_adopted_in_branch=True, general_old_new_equivalence_proved=False,
                  ning_acceptance=False, shared_acceptance=False, full_v3=False)
    fixture = output / 'original-source-work.json'

    def command(project, folder, view, label, argv, env):
        log = folder / (label + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1800).returncode
        record['commands'].append(dict(view=view, label=label, argv=argv, cwd=str(project), exit_code=code,
            seconds=time.monotonic()-begin, log=str(log), log_sha256=sha(log)))
        (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
        return code, log

    try:
        require(1 <= len(os.sched_getaffinity(0)) <= 4, 'native CPU allocation differs')
        head = support.source_guard()
        inputs = support.base.snapshot(ROOT)
        modes = {n: mode(ROOT / n) for n in inputs}
        libraries = support.registry.registry()
        tools = {}
        for name in ('rustc', 'cargo', 'rustfmt', 'clippy-driver'):
            path = Path(subprocess.check_output(['rustup', 'which', name], text=True).strip())
            tools[name] = dict(path=str(path), sha256=sha(path))
        record.update(inputs=inputs, input_modes=modes, library_registry=libraries, tools=tools,
                      source_head=head, source_base=support.BASE)
        original = subprocess.check_output(['git', 'show', support.BASE + ':' + SOURCE], cwd=ROOT)
        fixture_sha = None
        for view, failures in FAILURES.items():
            folder = output / view
            project = folder / 'source'
            project.mkdir(parents=True, mode=0o700)
            expected = dict(inputs)
            for name, wanted in inputs.items():
                path = project / name
                path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, path)
                require(sha(path) == wanted, 'input changed during copy')
            body = original if view == 'baseline' else support.mutate(
                (ROOT / SOURCE).read_text(), 'correct' if view == 'restored' else view).encode()
            probe = b'' if view == 'baseline' else (HERE / 'native-tests.rs.in').read_bytes()
            (project / SOURCE).write_bytes(body + probe + (HERE / 'pipeline-tests.rs.in').read_bytes())
            path = project / TESTSOURCE
            path.write_bytes(path.read_bytes() + (HERE / 'export-tests.rs.in').read_bytes())
            for name in (SOURCE, TESTSOURCE):
                expected[name] = sha(project / name)
            target = folder / 'target'
            require(not target.exists(), 'variant target already exists')
            env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR='never', PYTHONDONTWRITEBYTECODE='1',
                SANDPROOF_SOURCE_QUESTION_FIXTURE=str(fixture))
            code, _ = command(project, folder, view, 'format',
                ['rustfmt', '--edition', '2024', '--check', SOURCE, TESTSOURCE], env)
            require(code == 0, 'format failure: ' + view)
            code, build = command(project, folder, view, 'build',
                ['cargo', 'test', '-p', 'sandproof-controller-service', '--lib', '--no-run',
                 '--locked', '--offline', '--message-format=json'], env)
            require(code == 0, 'compile failure is not property rejection: ' + view)
            built = artifacts(build, project, target)
            program, = [x['executable'] for x in built if x['target']['name'] == 'sandproof_controller_service']
            code, listing = command(project, folder, view, 'list', [program, '--list'], env)
            require(code == 0, 'test inventory failed')
            tests = {PIPELINE} | (set() if view == 'baseline' else HELPER_TESTS)
            names = {x.removesuffix(': test').split('::')[-1] for x in listing.read_text().splitlines()
                     if x.endswith(': test') and PREFIX in x}
            require(names == tests, 'actual complete test inventory differs')
            if view == 'baseline':
                require(not fixture.exists(), 'original fixture already populated')
                code, log = command(project, folder, view, 'export', [program,
                    'controller::lifecycle_inbox::source_work::tests::' + EXPORT,
                    '--exact', '--test-threads=1'], env)
                require(code == 0 and '1 passed; 0 failed; 0 ignored;' in log.read_text() and
                        fixture.is_file(), 'original production fixture was not produced')
                fixture_sha = sha(fixture)
                record['fixture'] = dict(path=str(fixture), sha256=fixture_sha, size=fixture.stat().st_size,
                    origin='original9c50 production question and original protected fixture', export_log=str(log))
            code, log = command(project, folder, view, 'tests', [program, PREFIX, '--test-threads=1'], env)
            entry = assertions(log, tests, failures, project, code)
            code, _ = command(project, folder, view, 'clippy',
                ['cargo', 'clippy', '-p', 'sandproof-controller-service', '--lib', '--tests',
                 '--locked', '--offline', '--', '-D', 'warnings'], env)
            require(code == 0, 'strict Clippy failed: ' + view)
            require(sha(fixture) == fixture_sha, 'the original fixture changed')
            require(all(sha(project / n) == digest and mode(project / n) == modes[n]
                        for n, digest in expected.items()), 'native source or mode changed')
            entry.update(complete=True, test_names=sorted(tests), source_inputs=expected, source_input_modes=modes,
                business_source_sha256=hashlib.sha256(body).hexdigest(),
                target_initially_absent=True, actual_question_pipeline_checked=True,
                artifacts=[dict(message=x, files={n: sha(Path(n)) for n in x['filenames'] if Path(n).is_file()})
                           for x in built])
            record['views'][view] = entry
        require(support.base.snapshot(ROOT) == inputs and support.registry.registry() == libraries and
                all(mode(ROOT / n) == m for n, m in modes.items()), 'root source/modes/libraries changed')
        require(all(sha(Path(x['path'])) == x['sha256'] for x in tools.values()), 'native tool changed')
        require(all(sha(Path(x['log'])) == x['log_sha256'] for x in record['commands']), 'original log changed')
        require((output / 'correct/source' / SOURCE).read_bytes() ==
                (output / 'restored/source' / SOURCE).read_bytes(), 'restored Rust differs')
        record.update(complete=True, source_tool_library_fixture_and_log_postguards_complete=True)
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - started
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    cpus = sorted(os.sched_getaffinity(0))
    if len(cpus) > 4:
        os.sched_setaffinity(0, set(cpus[:4]))
    result = run(args.output.resolve())
    print(json.dumps({n: result.get(n) for n in ('complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
