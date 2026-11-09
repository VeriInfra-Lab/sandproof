"""Run the nine original source-row assertions in the native suite."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tarfile
import time

HERE = Path(__file__).resolve().parent
STAGE1 = HERE.parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import sha, write_json


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(output):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native':
        raise ValueError('source-row assertions require the native suite role')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    output.mkdir(parents=True, mode=0o700)
    view = output/'original'
    project, target = view/'source', view/'target'
    project.mkdir(parents=True, mode=0o700)
    result = dict(kind='original-source-rows-native-component', complete=False,
                  commands=[], cases=[], cpus=cpus)
    started = time.monotonic()
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS=str(len(cpus)),
               CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR='never',
               CARGO_INCREMENTAL='0', PYTHONDONTWRITEBYTECODE='1')

    def command(label, argv):
        log = view/(label+'.log')
        begin = time.monotonic()
        argv = list(map(str, argv))
        with log.open('xb') as stream:
            child = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                   stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        result['commands'].append(dict(view='original', label=label, argv=argv,
            cwd=str(project), exit_code=child.returncode, seconds=time.monotonic()-begin,
            log=str(log), log_sha256=sha(log)))
        child.check_returncode()
        return log

    try:
        archive = load('source_rows_archive', STAGE1/'archive.py')
        legacy = load('source_rows_artifacts', STAGE1/'source-origin/native.py')
        registry = load('source_rows_registry', STAGE1/'envelope-writer/native.py')
        registry.HERE = Path('.')
        require = legacy.require
        plan = json.loads((HERE/'native-admission.json').read_text())
        native_plan = HERE/'native-plan.json'
        require(sha(native_plan) == plan['native_plan_sha256'], 'fixed nine-case plan differs')
        cases = json.loads(native_plan.read_text())['cases']
        names = [case['filter'] for case in cases]
        require(len(names) == len(set(names)) == 9 and all(
            case['target'] == 'lib' and case['exact'] is True and case['count'] == 1
            for case in cases), 'original exact assertion inventory differs')

        source = archive.build(ROOT, output/'source.tar', output/'source-manifest.json')
        archive.verify(output/'source.tar', output/'source-manifest.json')
        result.update(source_head=source['commit'], source_tree=source['tree'],
                      source_archive_sha256=source['archive_sha256'], assertion_inventory=names)
        for name, expected in plan['source_inputs'].items():
            require(source['files'][name]['sha256'] == expected, 'reviewed source differs: '+name)
        with tarfile.open(output/'source.tar') as tar:
            tar.extractall(project, filter='data')
        inputs = {name: dict(row) for name, row in source['files'].items()}
        libraries = registry.registry()
        tools = {}
        for name in ('rustc', 'cargo', 'rustfmt', 'clippy-driver'):
            path = Path(subprocess.check_output(['rustup', 'which', name], cwd=project,
                                                env=env, text=True).strip())
            tools[name] = dict(path=str(path), sha256=sha(path))
        result.update(library_registry=libraries, tools=tools)

        touched = []
        for fixture in plan['fixtures']:
            name = fixture['source']
            path = project/name
            fragment = project/fixture['fragment']
            require(sha(fragment) == fixture['fragment_sha256'], 'original fixture differs: '+name)
            body = path.read_bytes()
            require(all(('fn '+test+'(').encode() not in body for test in fixture['tests']),
                    'original fixture already installed: '+name)
            path.write_bytes(body+fragment.read_bytes())
            touched.append(path)
        fmt = ['rustfmt', '--edition', '2024', '--config', 'skip_children=true']
        command('private-test-format', [*fmt, *touched])
        for fixture in plan['fixtures']:
            name = fixture['source']
            actual = sha(project/name)
            require(actual == fixture['formatted_sha256'], 'formatted test bytes differ: '+name)
            inputs[name]['sha256'] = actual
        command('format', [*fmt, '--check', project/plan['capture_source'], *touched])

        require(not target.exists(), 'original source-row target must be fresh')
        cargo = ['-p', 'sandproof-controller-service', '--lib', '--locked', '--offline']
        build = command('build', ['cargo', 'test', *cargo, '--no-run', '--message-format=json'])
        artifacts = legacy.artifacts(build, project, target)
        program, = [row['executable'] for row in artifacts
                    if row['target']['name'] == 'sandproof_controller_service']
        require(isinstance(program, str), 'actual test binary missing')
        executable = Path(program)
        require(executable.is_file() and not executable.is_symlink()
                and executable.resolve().is_relative_to(target.resolve()),
                'actual test binary outside fresh target')
        with executable.open('rb') as stream:
            require(stream.read(4) == b'\x7fELF', 'actual Linux ELF required')
        executable_sha = sha(executable)
        result.update(actual_executable=dict(path=program, sha256=executable_sha),
                      artifacts=[dict(message=row, files={n: sha(Path(n)) for n in row['filenames']
                                 if Path(n).is_file()}) for row in artifacts])
        listing = command('list', [program, '--list']).read_text().splitlines()
        for index, name in enumerate(names):
            require(listing.count(name+': test') == 1, 'exact assertion missing: '+name)
            label = 'source-row-case-'+str(index)
            log = command(label, [program, name, '--test-threads=1', '--exact'])
            text = log.read_text()
            require(re.findall(r'^test \S+ \.\.\. .*$', text, re.M) == ['test '+name+' ... ok']
                    and len(re.findall(r'^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured;',
                                       text, re.M)) == 1, 'exact assertion did not pass once: '+name)
            result['cases'].append(dict(filter=name, label=label, passed=1, ignored=0))
        command('clippy', ['cargo', 'clippy', *cargo, '--tests', '--', '-D', 'warnings'])

        def matches(name, row):
            path = project/name
            info = path.lstat()
            return (stat.S_ISREG(info.st_mode) and sha(path) == row['sha256']
                    and bool(info.st_mode & stat.S_IXUSR) == (row['mode'] == '100755'))

        current, _ = archive.committed_inputs(ROOT)
        require(current == {k: source[k] for k in ('schema', 'commit', 'tree', 'files')}
                and all(matches(name, row) for name, row in inputs.items())
                and registry.registry() == libraries and sha(executable) == executable_sha
                and all(sha(Path(row['path'])) == row['sha256'] for row in tools.values())
                and all(sha(Path(row['log'])) == row['log_sha256'] for row in result['commands']),
                'source, libraries, executable, tools or logs changed during native checks')
        require(archive.verify(output/'source.tar', output/'source-manifest.json') == source,
                'original source archive changed during native checks')
        result.update(complete=True, passed_cases=len(result['cases']), source_inputs=inputs,
                      input_tool_library_and_log_postguards_complete=True)
    except Exception as error:
        result['error'] = repr(error)
        raise
    finally:
        result['seconds'] = time.monotonic()-started
        write_json(output/'result.json', result)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    run(args.output.resolve())
