"""Check the first Unload transaction against a single current source archive."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sys
import tarfile

from unload_audit_driver import HERE, ROOT, command, digest, load, native_case, write_json
from unload_audits import PARENT_LABEL, check_parent_audits


LABEL = PARENT_LABEL
KIND = LABEL
PACKAGE = ROOT / HERE / 'unload-parent-link'
CASES = {
    'begin': 'UnloadWorldBegin',
    'prefix': 'UnloadFrontier',
    'prefix-reordered': 'UnloadFrontierReordered',
    'finish': 'UnloadFinish',
    'finish-reordered': 'UnloadFinishReordered',
}
AUTH_TEST = 'proof_first_unload_prepare_orders_inbox_authentication_and_source_decode'
COMMIT_TEST = 'proof_first_prepare_after_commit_error_preserves_original_durable_work'
TESTS = (AUTH_TEST, 'proof_first_unload_prepare_save_error_follows_construction_without_durable_work',
         COMMIT_TEST, 'proof_first_prepare_commit_uncertain_can_hide_either_sqlite_outcome')
TEST_ROOT = 'controller::lifecycle_inbox::source_work::tests::'
REJECTIONS = {
    'prefix-reordered': (AUTH_TEST, 'authentication must precede source decode: Err(Sqlite(InvalidColumnType(1, "sha256", Text)))'),
    'finish-reordered': (COMMIT_TEST, 'AfterCommit failure must leave the original work durable'),
}
NATIVE_VIEWS = ('original', 'prefix', 'prefix-reordered', 'finish', 'finish-reordered', 'restored')

sys.path.insert(0, str(PACKAGE))
from check_revision_pair import git, source_identity
import finish_capture
import frontier_capture
import prepare_unload_finish
import prepare_unload_frontier
import prepare_world_begin
import probe_prepare_iterators


def assert_current(result):
    head = git('rev-parse', 'HEAD').decode().strip()
    if git('status', '--porcelain', '-z'):
        raise ValueError('parent transaction requires a clean source checkout')
    identity = source_identity(head, head)
    result.update(commit=head, tree=identity['tree'], source_sha256=identity['source_archive_sha256'])
    return identity


def run_native(output, result):
    archive = load(ROOT / HERE / 'archive.py')
    source = archive.build(ROOT, output / 'source.tar', output / 'source-manifest.json')
    if archive.verify(output / 'source.tar', output / 'source-manifest.json') != source:
        raise ValueError('parent transaction native source archive differs')
    project = output / 'source'
    project.mkdir()
    with tarfile.open(output / 'source.tar') as stream:
        stream.extractall(project, filter='data')
    for name, expected in probe_prepare_iterators.prepare.SOURCE_INPUTS.items():
        if digest(project / name) != expected:
            raise ValueError('parent transaction native input differs: ' + name)
    business = project / probe_prepare_iterators.prepare.SOURCE
    original = business.read_text()
    prefix = probe_prepare_iterators.rewrite(original, 'json-boundary')
    bodies = dict(original=original, prefix=frontier_capture.factor(prefix),
                  finish=finish_capture.factor(original), restored=original,
                  **{'prefix-reordered': frontier_capture.factor(prefix, reordered=True),
                     'finish-reordered': finish_capture.factor(original, reordered=True)})
    target = project / 'controller-service/src/controller/lifecycle_inbox/source_work/tests.rs'
    with target.open('ab') as stream:
        shared = project / HERE / 'unload-prepare-transaction'
        for file in ('native-tests.rs.in', 'begin-contention.rs.in'):
            stream.write(b'\n' + (shared / file).read_bytes())
        for file in ('current-inputs/native-first-errors.rs.in', 'finish/native-finish.rs.in'):
            stream.write(b'\n' + (project / PACKAGE.relative_to(ROOT) / file).read_bytes())
    env = dict(os.environ, CARGO_TARGET_DIR=str(output / 'target'), CARGO_NET_OFFLINE='true',
               CARGO_INCREMENTAL='0', CARGO_TERM_COLOR='never')
    cargo = ['cargo', 'test', '--locked', '--offline', '-p', 'sandproof-controller-service',
             '--features', 'rusqlite/hooks', '--lib']
    result['native_views'] = {}
    for name in NATIVE_VIEWS:
        view = output / name
        view.mkdir()
        business.write_text(bodies[name])
        expected_failure = REJECTIONS.get(name)
        if expected_failure:
            command(result, env, project, view, 'compile', [*cargo, '--no-run'])
            names = (expected_failure[0],)
        else:
            names = TESTS
        for test in names:
            text = command(result, env, project, view, test,
                           [*cargo, TEST_ROOT + test, '--', '--exact', '--nocapture', '--test-threads=1'],
                           expected=101 if expected_failure else 0)
            if expected_failure:
                if (expected_failure[1] not in text or 'test result: FAILED. 0 passed; 1 failed;' not in text
                        or f'\n    {TEST_ROOT}{test}\n' not in text):
                    raise ValueError('wrong parent did not reach the original rejection: ' + test)
            elif f'test {TEST_ROOT}{test} ... ok' not in text or 'test result: ok. 1 passed; 0 failed;' not in text:
                raise ValueError('missing original parent result: ' + test)
        result['native_views'][name] = dict(tests=list(names), rust_sha256=digest(business),
                                            fixture_sha256=digest(target))


def run_formal(output, result, identity, receipt_path):
    registered = load(ROOT / HERE / 'registered.py')
    runtime = load(ROOT / HERE / 'formal_runtime.py')
    checker = load(ROOT / HERE / 'formal.py')
    receipt = json.loads(receipt_path.read_bytes())
    validate_native_result(receipt, identity)
    result['native_result_sha256'] = digest(receipt_path)
    artifacts = output / 'artifacts'
    captures = artifacts / 'captures'
    captures.mkdir(parents=True)
    lock = captures / 'component.lock'
    lock.touch()
    tools = runtime.tool_root(ROOT) / 'aeneas'
    packages, mcp = runtime.package_root(ROOT), runtime.tool_root(ROOT) / 'lean-mcp'
    begin = captures / 'begin'
    env = dict(os.environ, LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    command(result, env, ROOT, captures, 'begin',
            [sys.executable, PACKAGE / 'capture.py', '--mode', 'world-begin',
             '--source-commit', identity['head'], '--lock', lock, '--output', begin])
    frontiers = {name: captures / ('frontier-' + name) for name in ('prefix', 'parent', 'reordered')}
    for name, folder in frontiers.items():
        if probe_prepare_iterators.probe('json-boundary', folder, tools, lock, False,
                                         identity['head'], name, identity=identity):
            raise ValueError('incomplete actual frontier extraction: ' + name)
    finishes = {name: captures / ('finish-' + name) for name in ('tail', 'parent', 'reordered')}
    for name, folder in finishes.items():
        finish_capture.capture(name, folder, tools, lock, identity=identity)

    for view, capture, report_name in (
        ('prefix', frontiers['prefix'], 'probe-result.json'),
        ('prefix-reordered', frontiers['reordered'], 'probe-result.json'),
        ('finish', finishes['tail'], 'result.json'),
        ('finish-reordered', finishes['reordered'], 'result.json'),
    ):
        raw = json.loads((capture / report_name).read_bytes())
        if raw['candidate_source_sha256'] != receipt['native_views'][view]['rust_sha256']:
            raise ValueError('formal and native Rust variants differ: ' + view)

    for case, root in CASES.items():
        folder = output / case
        folder.mkdir()
        project = folder / 'project'
        if case == 'begin':
            prepare_world_begin.prepare(begin, project, packages, mcp, identity=identity)
        elif case.startswith('prefix'):
            wrong = case.endswith('reordered')
            prepare_unload_frontier.materialize(frontiers['reordered' if wrong else 'prefix'],
                frontiers['parent'], begin, project, packages, mcp, wrong, identity=identity)
        else:
            mode = 'reordered' if case.endswith('reordered') else 'tail'
            prepare_unload_finish.materialize(finishes[mode], finishes['parent'],
                frontiers['prefix'], frontiers['parent'], begin, project, packages, mcp, mode, identity=identity)
        plan = json.loads((project / 'plan.json').read_bytes())
        if plan['root'] != root or plan['source'] != identity['head']:
            raise ValueError('wrong materialized root or commit: ' + case)
        if any(digest(project / name) != checksum for name, checksum in plan['files'].items()):
            raise ValueError('materialized proof differs from its checked inputs')
        shutil.copyfile(ROOT/'tools/proof_services.py', project/'tools/proof_services.py')
        (project / 'artifacts').mkdir()

        def run(cwd, logs, label, argv):
            command(result, env, cwd, logs, label, argv)

        run(project, folder, 'build', ['lake', 'build', root])
        actual = checker.actual_files(project, sorted(plan['files']), folder, env, run)
        registered._saved_physical_view(actual, project, folder / 'physical-files', result['commands'])
        result['cases'][case] = dict(actual, complete=True)
    check_parent_audits(ROOT, result)


def validate_native_result(receipt, identity):
    if (receipt.get('kind') != KIND or receipt.get('complete') is not True
            or receipt.get('commit') != identity['head'] or receipt.get('tree') != identity['tree']):
        raise ValueError('missing current parent transaction native evidence')
    views = receipt.get('native_views', {})
    if list(views) != list(NATIVE_VIEWS) or views['original'] != views['restored']:
        raise ValueError('missing parent transaction variants or original restoration')
    for name, view in views.items():
        tests = [REJECTIONS[name][0]] if name in REJECTIONS else list(TESTS)
        if view['tests'] != tests:
            raise ValueError('parent transaction native assertions differ: ' + name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('native', 'formal'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != args.mode:
        raise ValueError('parent transaction driver requires its original host role')
    if args.mode == 'formal':
        registered = load(ROOT / HERE / 'registered.py')
        registered.inherited_lock()
        if len(os.sched_getaffinity(0)) != 4:
            raise ValueError('parent transaction formal work requires four admitted CPUs')
    os.umask(0o077)
    output = args.output.resolve()
    output.mkdir(mode=0o700)
    result = dict(kind=KIND, complete=False, cases={}, commands=[])
    try:
        identity = assert_current(result)
        if args.mode == 'native':
            run_native(output, result)
        else:
            evidence = native_case(LABEL, ROOT, None, scope='stage1-protocol')
            run_formal(output, result, identity, registered._original_file(evidence, 'result.json'))
        assert_current(result)
        result['complete'] = True
    finally:
        write_json(output / 'result.json', result)


if __name__ == '__main__':
    main()
