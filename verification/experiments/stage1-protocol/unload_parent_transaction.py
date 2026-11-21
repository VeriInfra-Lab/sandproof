"""Check the first Unload transaction against a single current source archive."""
import argparse
import json
import os
from pathlib import Path
import re
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
    'revision': 'RevisionParentCall',
    'parent-link': 'UnloadParentLink',
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
from check_revision_pair import check_pair, git, source_identity
import finish_capture
import frontier_capture
from prepare import materialize_current
import prepare_revision_call
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
        compile_log = command(result, env, project, view, 'compile',
                              [*cargo, '--no-run', '--message-format=json'])
        artifacts = [json.loads(line) for line in compile_log.splitlines() if line.startswith('{')]
        binary, = [item for item in artifacts
                   if item.get('reason') == 'compiler-artifact'
                   and item['target']['name'] == 'sandproof_controller_service'
                   and item['target']['src_path'] == str(project / 'controller-service/src/lib.rs')
                   and item['profile']['test'] and item['executable']]
        executable = Path(binary['executable'])
        if binary['fresh'] is not False or not executable.is_relative_to(output / 'target'):
            raise ValueError('parent transaction test did not freshly compile the current Rust variant: ' + name)
        executable_sha256 = digest(executable)
        expected_failure = REJECTIONS.get(name)
        names = (expected_failure[0],) if expected_failure else TESTS
        for test in names:
            text = command(result, env, project, view, test,
                           [executable, TEST_ROOT + test, '--exact', '--nocapture', '--test-threads=1'],
                           expected=101 if expected_failure else 0)
            if expected_failure:
                if (expected_failure[1] not in text or 'test result: FAILED. 0 passed; 1 failed;' not in text
                        or f'\n    {TEST_ROOT}{test}\n' not in text):
                    raise ValueError('wrong parent did not reach the original rejection: ' + test)
            elif f'test {TEST_ROOT}{test} ... ok' not in text or 'test result: ok. 1 passed; 0 failed;' not in text:
                raise ValueError('missing original parent result: ' + test)
        if digest(executable) != executable_sha256 or business.read_text() != bodies[name]:
            raise ValueError('parent transaction test executable or source changed: ' + name)
        result['native_views'][name] = dict(tests=list(names), rust_sha256=digest(business),
                                            fixture_sha256=digest(target),
                                            executable_sha256=executable_sha256)


def run_formal(output, result, identity, receipt_path, native_archive, native_manifest):
    registered = load(ROOT / HERE / 'registered.py')
    runtime = load(ROOT / HERE / 'formal_runtime.py')
    checker = load(ROOT / HERE / 'formal.py')
    receipt = json.loads(receipt_path.read_bytes())
    validate_native_result(receipt, identity)
    archive = load(ROOT / HERE / 'archive.py').verify(native_archive, native_manifest)
    if archive['commit'] != identity['head'] or archive['tree'] != identity['tree']:
        raise ValueError('parent transaction archive belongs to a different candidate')
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
    revisions = {name: captures / ('revision-' + name) for name in ('base-revision', 'parent')}
    for mode, folder in {'world-begin': begin, **revisions}.items():
        command(result, env, ROOT, captures, folder.name,
                [sys.executable, PACKAGE / 'capture.py', '--mode', mode,
                 '--source-commit', identity['head'], '--lock', lock, '--output', folder])
    report = json.loads((ROOT / HERE / 'unload-startup-source/capture-profile.json').read_bytes())
    parent = dict(report['parent'], namespace=prepare_revision_call.PARENT,
                  subdir=prepare_revision_call.PARENT)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    pair = check_pair(revisions['base-revision'], revisions['parent'], identity, parent, fixed['target'])
    write_json(captures / 'revision-pair.json', pair)
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
        elif case == 'revision':
            prepare_revision_call.prepare_call(
                revisions['base-revision'] / 'generated' / prepare_revision_call.BASE,
                revisions['parent'] / 'generated' / prepare_revision_call.PARENT,
                packages, mcp, project, identity=identity)
        elif case == 'parent-link':
            materialize_current(project, output / 'revision' / 'project', identity)
        elif case.startswith('prefix'):
            wrong = case.endswith('reordered')
            prepare_unload_frontier.materialize(frontiers['reordered' if wrong else 'prefix'],
                frontiers['parent'], begin, project, packages, mcp, wrong, identity=identity)
        else:
            mode = 'reordered' if case.endswith('reordered') else 'tail'
            prepare_unload_finish.materialize(finishes[mode], finishes['parent'],
                frontiers['prefix'], frontiers['parent'], begin, project, packages, mcp, mode, identity=identity)
        plan = json.loads((project / 'plan.json').read_bytes())
        if case in ('revision', 'parent-link'):
            roots, source = plan['roots'], plan['source_commit']
            expected_roots = [root, 'ParentRejects'] if case == 'revision' else [root]
            files = {name: row['sha256'] for name, row in plan['modules'].items()}
        else:
            roots, source, files = [plan['root']], plan['source'], plan['files']
            expected_roots = [root]
        if roots != expected_roots or source != identity['head']:
            raise ValueError('wrong materialized root or commit: ' + case)
        if any(digest(project / name) != checksum for name, checksum in files.items()):
            raise ValueError('materialized proof differs from its checked inputs')
        shutil.copyfile(ROOT/'tools/proof_services.py', project/'tools/proof_services.py')
        (project / 'artifacts').mkdir(exist_ok=True)

        def run(cwd, logs, label, argv):
            command(result, env, cwd, logs, label, argv)

        run(project, folder, 'build', ['lake', 'build', *roots])
        repeated = case == 'parent-link'
        actual = checker.actual_files(project, sorted(files), folder, env, run, repeat_across_pages=repeated)
        registered._saved_physical_view(actual, project, folder / 'physical-files', result['commands'],
                                        repeat_across_pages=repeated)
        result['cases'][case] = dict(actual, complete=True)
    check_parent_audits(ROOT, result, output)


def validate_native_result(receipt, identity):
    if (receipt.get('kind') != KIND or receipt.get('complete') is not True
            or receipt.get('commit') != identity['head'] or receipt.get('tree') != identity['tree']
            or receipt.get('source_sha256') != identity['source_archive_sha256']):
        raise ValueError('missing current parent transaction native evidence')
    views = receipt.get('native_views', {})
    if list(views) != list(NATIVE_VIEWS) or any(
            views['original'].get(key) != views['restored'].get(key)
            for key in ('rust_sha256', 'fixture_sha256', 'tests')):
        raise ValueError('missing parent transaction variants or original restoration')
    for name, view in views.items():
        tests = [REJECTIONS[name][0]] if name in REJECTIONS else list(TESTS)
        if view['tests'] != tests or any(not re.fullmatch(r'[0-9a-f]{64}', view.get(key, ''))
                                         for key in ('rust_sha256', 'fixture_sha256', 'executable_sha256')):
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
            files = [registered._original_file(evidence, name)
                     for name in ('result.json', 'source.tar', 'source-manifest.json')]
            run_formal(output, result, identity, *files)
        assert_current(result)
        result['complete'] = True
    finally:
        write_json(output / 'result.json', result)


if __name__ == '__main__':
    main()
