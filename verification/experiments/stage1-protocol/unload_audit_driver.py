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

from unload_audits import AUDIT, HERE, KIND, LABEL, PACKAGES, digest, frozen_inputs, package_audits
import unload_current_capture

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


def prepare_current(output, source, native_archive, runtime, run):
    parent = ROOT / HERE / 'unload-parent-link'
    sys.path.insert(0, str(parent))
    from check_revision_pair import check_pair, source_identity
    from prepare import materialize_current
    import prepare_revision_call

    identity = source_identity(source['commit'], source['commit'])
    if identity['tree'] != source['tree']:
        raise ValueError('current Unload checkout differs from the native archive')
    artifacts = output/'artifacts'
    leaf = unload_current_capture.run(ROOT, native_archive, artifacts/'current-captures',
                                      runtime.tool_root(ROOT)/'aeneas', run, digest)
    lock = artifacts/'revision.lock'
    lock.touch()
    revisions = {mode: artifacts/('revision-' + mode) for mode in ('base-revision', 'parent')}
    for mode, folder in revisions.items():
        run(dict(os.environ), ROOT, artifacts, folder.name,
            [sys.executable, parent/'capture.py', '--mode', mode, '--source-commit', source['commit'],
             '--lock', lock, '--output', folder])
    descriptor = json.loads((ROOT/HERE/'unload-startup-source/capture-profile.json').read_bytes())['parent']
    descriptor.update(namespace=prepare_revision_call.PARENT, subdir=prepare_revision_call.PARENT)
    fixed = json.loads((ROOT/'verification/foundations-environment.json').read_bytes())
    pair = check_pair(revisions['base-revision'], revisions['parent'], identity, descriptor, fixed['target'])
    write_json(artifacts/'revision-pair.json', pair)
    revision, project = output/'revision-project', output/'current-project'
    prepare_revision_call.prepare_call(
        revisions['base-revision']/'generated'/prepare_revision_call.BASE,
        revisions['parent']/'generated'/prepare_revision_call.PARENT,
        runtime.package_root(ROOT), runtime.tool_root(ROOT)/'lean-mcp', revision, identity=identity)
    materialize_current(project, revision, identity)
    shutil.copyfile(ROOT/'tools/proof_services.py', project/'tools/proof_services.py')
    (project/'artifacts').mkdir(exist_ok=True)
    return (project, leaf, identity['source_archive_sha256'])


def run_formal(output, result):
    registered = load(ROOT / HERE / 'registered.py')
    native = native_case(LABEL, ROOT, None, scope='stage1-protocol')
    receipt = registered._original_file(native, 'result.json')
    native_result = json.loads(receipt.read_text())
    archive = load(ROOT / HERE / 'archive.py')
    native_archive = registered._original_file(native, 'source.tar')
    source = archive.verify(native_archive,
                            registered._original_file(native, 'source-manifest.json'))
    if (native_result.get('kind') != KIND or native_result.get('complete') is not True
            or native_result.get('commit') != source['commit'] or native_result.get('tree') != source['tree']
            or native_result.get('frozen_inputs') != frozen_inputs(ROOT)
            or set(native_result.get('tests', {})) != {
                'startup', 'continued', 'options-encoding', 'options-invalid',
                'wrong-0', 'wrong-1', 'restored-0', 'restored-1'}):
        raise ValueError('missing original unload native assertion evidence')
    result['native_result_sha256'] = digest(receipt)
    verify_current(output, result, source, native_archive, registered)


def verify_current(output, result, source, native_archive, registered):
    runtime = load(ROOT / HERE / 'formal_runtime.py')
    checker = load(ROOT / HERE / 'formal.py')
    templates = registered.audit_templates(ROOT)
    env = dict(os.environ, LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')

    def capture(env, cwd, logs, label, argv):
        command(result, env, cwd, logs, label, argv)

    def run(cwd, logs, label, argv):
        command(result, env, cwd, logs, label, argv)

    project, leaves, source_sha256 = prepare_current(output, source, native_archive, runtime, capture)
    result.update(commit=source['commit'], tree=source['tree'], source_sha256=source_sha256,
                  current_captures=leaves)
    plan = json.loads((project/'plan.json').read_bytes())
    if (plan['roots'] != ['UnloadParentLink'] or plan['source_commit'] != source['commit']
            or any(digest(project/name) != item['sha256'] for name, item in plan['modules'].items())):
        raise ValueError('current Unload proof plan differs from its checked inputs')
    run(project, output, 'build', ['lake', 'build', 'UnloadParentLink'])

    for package in PACKAGES:
        folder = output / package
        folder.mkdir()
        audit_files = package_audits(templates, package)
        audited = {}
        for name, expected in audit_files.items():
            relative = Path(name).relative_to(HERE / package / 'proof-inputs').with_suffix('')
            if AUDIT.findall((project / relative).read_text()) != expected:
                raise ValueError('materialized unload audit differs: ' + name)
            audited[str(relative)] = expected

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
