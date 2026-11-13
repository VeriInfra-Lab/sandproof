"""Execute the original unload assertions and frozen proof views in Stage1."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile

from unload_audits import AUDIT, HERE, KIND, LABEL, PACKAGES, capture_hashes, digest, frozen_inputs, package_audits

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import native_case, write_json

SOURCE_WORK = Path('controller-service/src/controller/lifecycle_inbox/source_work')
HOST = Path('controller-service/src/host_contract.rs')
TEST_PREFIX = 'controller::lifecycle_inbox::source_work::tests::'
STARTUP_TEST = TEST_PREFIX + 'proof_source_flow_original_request_reaches_same_source_question'
CONTINUED_TEST = TEST_PREFIX + 'real_continued_cursor_keeps_receipt_bytes_and_stops_at_first_error'
OPTIONS_TESTS = tuple('host_contract::proof_create_options::' + name for name in (
    'actual_create_options_keeps_template_and_full_encoding',
    'actual_create_options_enforces_field_rules_and_byte_limit'))
MUTATIONS = (
    (SOURCE_WORK / 'full/continuation.rs', b'Err(error) => break (Err(error), next_state),',
     b'Err(_) => state = next_state,', CONTINUED_TEST, 'assertion failed: matches!(read(), Err(JournalError::Json(_)))'),
    (HOST, b"key.as_bytes().contains(&b'=')", b"key.as_bytes().contains(&b'~')",
     OPTIONS_TESTS[1], 'Err(AdmissionError::InvalidCreateOptions)'),
)


def load(path):
    spec = importlib.util.spec_from_file_location(path.stem, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def command(result, env, cwd, logs, label, argv, *, expected=0):
    log = logs / (label + '.log')
    argv = list(map(str, argv))
    with log.open('xb') as stream:
        process = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                 stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
    result['commands'].append(dict(argv=argv, cwd=str(cwd), exit_code=process.returncode,
                                   log=str(log), log_sha256=digest(log)))
    if process.returncode != expected:
        raise ValueError('unload command failed: ' + str(log))
    return log.read_text()


def run_native(output, result):
    archive = load(ROOT / HERE / 'archive.py')
    source = archive.build(ROOT, output / 'source.tar', output / 'source-manifest.json')
    if archive.verify(output / 'source.tar', output / 'source-manifest.json') != source:
        raise ValueError('original unload native source archive differs')
    project = output / 'source'
    project.mkdir()
    with tarfile.open(output / 'source.tar') as stream:
        stream.extractall(project, filter='data')
    for package, path in (('unload-startup-source', SOURCE_WORK / 'tests.rs'),
                          ('unload-continued-observation', SOURCE_WORK / 'tests.rs'),
                          ('unload-stored-options', HOST)):
        with (project / path).open('ab') as stream:
            stream.write((project / HERE / package / 'native-tests.rs.in').read_bytes())
    env = dict(os.environ, CARGO_TARGET_DIR=str(output / 'target'), CARGO_NET_OFFLINE='true',
               CARGO_INCREMENTAL='0', CARGO_TERM_COLOR='never')
    cargo = ['cargo', 'test', '-p', 'sandproof-controller-service', '--lib', '--locked', '--offline']

    def exact(label, test, failure=None):
        stdout = command(result, env, project, output, label,
                         [*cargo, test, '--', '--exact', '--nocapture', '--test-threads=1'],
                         expected=101 if failure else 0)
        status = 'FAILED' if failure else 'ok'
        counts = '0 passed; 1 failed' if failure else '1 passed; 0 failed'
        reported = re.search(r'^\s*' + re.escape(test) + r'\s*$', stdout, re.M) if failure else f'test {test} ... ok' in stdout
        if (not reported or f'test result: {status}. {counts}; 0 ignored;' not in stdout
                or (failure and failure not in stdout)):
            raise ValueError('original unload assertion did not reach its required outcome: ' + test)
        result['tests'][label] = test

    result.update(commit=source['commit'], tree=source['tree'], tests={})
    for label, name in zip(('startup', 'continued', 'options-encoding', 'options-invalid'),
                           (STARTUP_TEST, CONTINUED_TEST, *OPTIONS_TESTS)):
        exact(label, name)
    for index, (path, before, after, test, failure) in enumerate(MUTATIONS):
        original = (project / path).read_bytes()
        if original.count(before) != 1:
            raise ValueError('original unload mutation is missing or ambiguous: ' + str(path))
        wrong = original.replace(before, after, 1)
        (output / f'wrong-{index}.rs').write_bytes(wrong)
        (project / path).write_bytes(wrong)
        try:
            exact(f'wrong-{index}', test, failure)
        finally:
            (project / path).write_bytes(original)
        exact(f'restored-{index}', test)


def copy_captures(output):
    setting = os.environ.get('SANDPROOF_UNLOAD_CAPTURE_EVIDENCE')
    if not setting or not Path(setting).is_absolute():
        raise ValueError('SANDPROOF_UNLOAD_CAPTURE_EVIDENCE must name the original unload LLBC directory')
    root = Path(setting).resolve(strict=True)
    for package in PACKAGES:
        folder = output / package
        folder.mkdir()
        for name, expected in capture_hashes(ROOT, package).items():
            relative = Path(package) / (name + '.llbc')
            source = root / relative
            if source.resolve() != source or digest(source) != expected:
                raise ValueError('original unload capture differs: ' + str(relative))
            target = output / relative
            shutil.copyfile(source, target)
            if digest(target) != expected:
                raise ValueError('original unload capture changed while copying: ' + str(relative))


def prepare_project(package, project, runtime):
    path = ROOT / HERE / package
    if package == 'unload-startup-source':
        load(path / 'materialize.py').materialize(project)
    elif package == 'unload-base-provenance':
        load(path / 'materialize.py').materialize(ROOT, project)
    else:
        load(ROOT / HERE / 'unload-continued-observation/materialize.py').materialize(project, path)
    (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
    (project / '.lake').mkdir()
    (project / '.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
    for name in ('tools/proof_services.py', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        destination = project / name
        destination.parent.mkdir(exist_ok=True)
        shutil.copyfile(ROOT / name, destination)
    (project / 'artifacts').mkdir()


def run_formal(output, result):
    registered = load(ROOT / HERE / 'registered.py')
    native = native_case(LABEL, ROOT, None, scope='stage1-protocol')
    receipt = registered._original_file(native, 'result.json')
    native_result = json.loads(receipt.read_text())
    archive = load(ROOT / HERE / 'archive.py')
    source = archive.verify(registered._original_file(native, 'source.tar'),
                            registered._original_file(native, 'source-manifest.json'))
    if (native_result.get('kind') != KIND or native_result.get('complete') is not True
            or native_result.get('commit') != source['commit'] or native_result.get('tree') != source['tree']
            or native_result.get('frozen_inputs') != frozen_inputs(ROOT)
            or set(native_result.get('tests', {})) != {
                'startup', 'continued', 'options-encoding', 'options-invalid',
                'wrong-0', 'wrong-1', 'restored-0', 'restored-1'}):
        raise ValueError('missing original unload native assertion evidence')
    result['native_result_sha256'] = digest(receipt)
    copy_captures(output / 'artifacts')
    runtime = load(ROOT / HERE / 'formal_runtime.py')
    checker = load(ROOT / HERE / 'formal.py')
    templates = registered.audit_templates(ROOT)
    env = dict(os.environ, LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    for package in PACKAGES:
        folder = output / package
        folder.mkdir()
        project = folder / 'project'
        prepare_project(package, project, runtime)
        audit_files = package_audits(templates, package)
        audited = {}
        for name, expected in audit_files.items():
            relative = Path(name).relative_to(HERE / package / 'proof-inputs').with_suffix('')
            if AUDIT.findall((project / relative).read_text()) != expected:
                raise ValueError('materialized unload audit differs: ' + name)
            audited[str(relative)] = expected

        def run(cwd, logs, label, argv):
            command(result, env, cwd, logs, label, argv)

        if package == 'unload-startup-source':
            modules = ['.'.join(Path(name).with_suffix('').parts) for name in audited]
            run(project, folder, 'build', ['lake', 'build', *modules])
        elif package == 'unload-base-provenance':
            run(project, folder, 'build', ['bash', ROOT / HERE / package / 'check-materialized.sh', project])
            objects = project / '.lake/build/lib/lean'
            objects.parent.mkdir(parents=True, exist_ok=True)
            objects.symlink_to(project / 'objects', target_is_directory=True)
        else:
            for index, module in enumerate((project / 'build-order.txt').read_text().splitlines()):
                source = Path(module + '.lean')
                if source.is_absolute() or '..' in source.parts:
                    raise ValueError('invalid original unload module: ' + module)
                obj = project / '.lake/build/lib/lean' / (module + '.olean')
                obj.parent.mkdir(parents=True, exist_ok=True)
                run(project, folder, f'build-{index}',
                    ['lake', 'env', 'lean', '--root=' + str(project), '-o', obj, source])
        actual = checker.actual_files(project, sorted(audited), folder, env, run)
        registered._saved_physical_view(actual, project, folder / 'physical-files', result['commands'])
        result['cases'][package] = dict(actual, complete=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('native', 'formal'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != args.mode:
        raise ValueError('unload audit driver requires its original host role')
    if args.mode == 'formal':
        lock, _ = load(ROOT / HERE / 'registered.py').inherited_lock()
        if not os.get_inheritable(lock) or len(os.sched_getaffinity(0)) != 4:
            raise ValueError('unload formal driver requires its inherited lock and four admitted CPUs')
    os.umask(0o077)
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True)
    (output / 'artifacts').mkdir()
    result = dict(kind=KIND, complete=False, cases={}, commands=[])
    try:
        inputs = frozen_inputs(ROOT)
        (run_native if args.mode == 'native' else run_formal)(output, result)
        if inputs != frozen_inputs(ROOT):
            raise ValueError('original unload inputs changed during execution')
        result.update(complete=True, frozen_inputs=inputs)
    except BaseException as error:
        result['error'] = repr(error)
        raise
    finally:
        write_json(output / 'result.json', result)


if __name__ == '__main__':
    main()
