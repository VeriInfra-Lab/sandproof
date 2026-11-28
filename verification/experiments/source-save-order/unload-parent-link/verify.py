"""Verify the source-save ordering slice against its fixed Rust archive."""

import argparse
import fcntl
import json
import os
import runpy
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
STAGE1 = ROOT / 'verification/experiments/stage1-protocol'
sys.path.extend((str(STAGE1 / 'unload-parent-link'), str(ROOT)))

from prepare_source_save_order import ROOTS, materialize
from prepare_started_transaction import RUST_FILES as TRANSACTION_RUST_FILES
from probe_prepare_iterators import GIB
import source_save_capture as concrete
import source_save_order as typed
from tools.acceptance_receipts import sha, write_json
from tools.benchmark_ci import remove_private_tree
from tools.ci_retention import reject_mounts


MODES = ('original', 'factored', 'reordered')
FIXTURE = HERE / 'save-codec/native-save-order.rs.in'
TEST_SOURCE = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests.rs')
TEST_GROUP = 'controller::lifecycle_inbox::source_work::tests::'
TEST_CASE = TEST_GROUP + 'proof_source_save_validation_precedes_any_row_update'
REJECTION = 'the saved-work conflict must precede the source row update:'
ENVIRONMENT = ROOT / 'verification/foundations-environment.json'
AUDIT_COUNTS = {'factored': 13, 'reordered': 3}
NATIVE_KIND = 'source-save-order-native'
SOURCE_INPUTS = (
    *TRANSACTION_RUST_FILES,
    'Cargo.toml',
    'rust-toolchain.toml',
    'controller-service/Cargo.toml',
    'controller-service/src/lib.rs',
    'controller-service/src/controller/lifecycle_inbox/source_work.rs',
    str(concrete.CODEC),
    'sandbox-protocol',
)


def command(project, output, label, argv, *, env=None):
    argv = list(map(str, argv))
    log = output / (label + '.log')
    with log.open('xb') as stream:
        proc = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                              stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
    return dict(argv=argv, exit_code=proc.returncode, log_sha256=sha(log))


def original_archive(target):
    actual_tree = subprocess.check_output(
        ['git', 'rev-parse', concrete.SOURCE+'^{tree}'], cwd=ROOT, text=True).strip()
    if actual_tree != concrete.TREE:
        raise ValueError('registered source-save tree changed')
    with target.open('xb') as stream:
        subprocess.run(['git', 'archive', concrete.SOURCE], cwd=ROOT, stdout=stream, check=True)
    if sha(target) != concrete.ARCHIVE:
        raise ValueError('registered source-save archive bytes changed')


def candidate():
    head, tree = subprocess.check_output(
        ['git', 'rev-parse', 'HEAD', 'HEAD^{tree}'], cwd=ROOT, text=True).splitlines()
    changes = subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
    if changes:
        raise ValueError('a clean proof checkout is required')
    anchor_inputs = subprocess.check_output(
        ['git', 'diff', '--name-only', concrete.SOURCE, head, '--', *SOURCE_INPUTS],
        cwd=ROOT, text=True).splitlines()
    if anchor_inputs:
        raise ValueError('the source-save input differs from its proof anchor: '+', '.join(anchor_inputs))
    return dict(review_commit=head, review_tree=tree, source_commit=concrete.SOURCE,
                source_tree=concrete.TREE, source_archive_sha256=concrete.ARCHIVE,
                fixture_sha256=sha(FIXTURE), entry_sha256=sha(Path(__file__)))


def check_assertions(mode, status, log):
    if mode == 'reordered':
        required = (REJECTION, 'reached source row update',
                    f'    {TEST_CASE}', 'test result: FAILED. 0 passed; 1 failed; 0 ignored;')
        passed = status == 101 and all(text in log for text in required)
    else:
        passed = status == 0 and f'test {TEST_CASE} ... ok' in log and 'test result: ok.' in log
    if not passed:
        raise ValueError('original SQLite assertion did not produce the expected result: ' + mode)


def compiled_test(output, project, target):
    with (output/'compile.log').open() as log:
        artifacts = [json.loads(line) for line in log if line.startswith('{')]
    item, = [row for row in artifacts if row.get('reason') == 'compiler-artifact'
              and row['target']['name'] == 'sandproof_controller_service'
              and row['profile']['test'] and row.get('executable')]
    path = Path(item['executable'])
    if (item['fresh'] is not False or path.is_symlink()
            or item['target']['src_path'] != str(project/'controller-service/src/lib.rs')
            or item['manifest_path'] != str(project/'controller-service/Cargo.toml')
            or not path.resolve(strict=True).is_relative_to(target)):
        raise ValueError('compiled test does not identify the actual source view')
    return path


def native_view(mode, archive, work, output, env):
    project = work / 'source'
    project.mkdir(mode=0o700, parents=True)
    with tarfile.open(archive) as source:
        source.extractall(project, filter='data')
    codec = concrete.stage(project, mode) if mode == 'original' else typed.stage(project, mode)
    with (project/TEST_SOURCE).open('ab') as stream:
        stream.write(b'\n' + FIXTURE.read_bytes())
    output.mkdir(mode=0o700)
    sources = {}
    for name in (concrete.CODEC, TEST_SOURCE):
        destination = output / name.name
        shutil.copyfile(project/name, destination)
        sources[name.name] = sha(destination)
    target = work / 'target'
    env = dict(env, CARGO_TARGET_DIR=str(target))
    compilation = command(project, output, 'compile',
                          ['cargo', 'test', '--locked', '--offline', '-p', 'sandproof-controller-service',
                           '--features', 'rusqlite/hooks', '--lib', '--no-run', '--message-format=json'],
                          env=env)
    if compilation['exit_code'] != 0:
        raise ValueError('actual source view failed to compile: ' + mode)
    binary = compiled_test(output, project, target)
    binary_sha256 = sha(binary)
    selection = [TEST_GROUP] if mode == 'factored' else ['--exact', TEST_CASE]
    assertion = command(project, output, 'assertions', [binary, *selection, '--nocapture'], env=env)
    check_assertions(mode, assertion['exit_code'], (output/'assertions.log').read_text())
    if sha(binary) != binary_sha256 or any(sha(project/name) != sources[name.name]
                                          for name in (concrete.CODEC, TEST_SOURCE)):
        raise ValueError('compiled executable or source changed during the test')
    return dict(compilation=compilation, assertions=assertion, sources=sources,
                actual_executable_sha256=binary_sha256, staged_codec_sha256=codec,
                property_rejected=mode == 'reordered')


def run_native(output, work_parent):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'native' or sys.platform != 'linux':
        raise ValueError('this SQLite slice requires the Linux native role')
    source = candidate()
    output.mkdir(mode=0o700)
    archive = output/'source.tar'
    original_archive(archive)
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0', CARGO_TERM_COLOR='never')
    rustc = subprocess.check_output(['rustc', '-vV'], cwd=ROOT, env=env, text=True)
    if rustc.splitlines()[0] != json.loads(ENVIRONMENT.read_bytes())['production_compiler']:
        raise ValueError('registered production compiler required')
    result = dict(kind=NATIVE_KIND, complete=False, **source, rustc=rustc, views={})
    work_parent.mkdir(mode=0o700)
    identity = work_parent.stat()
    try:
        for mode in MODES:
            result['views'][mode] = native_view(mode, archive, work_parent/mode, output/mode, env)
        if source != candidate():
            raise ValueError('proof checkout changed during native checks')
        if len({view['actual_executable_sha256'] for view in result['views'].values()}) != len(MODES):
            raise ValueError('each test view must identify a distinct real executable')
        reject_mounts(work_parent)
        remove_private_tree(work_parent, [identity.st_dev, identity.st_ino])
        result['complete'] = True
    finally:
        write_json(output/'result.json', result)


def native_evidence(folder):
    receipt = folder/'result.json'
    result = json.loads(receipt.read_bytes())
    if (result['kind'] != NATIVE_KIND or result['complete'] is not True
            or tuple(result['views']) != MODES
            or any(result.get(k) != v for k, v in candidate().items())
            or sha(folder/'source.tar') != concrete.ARCHIVE):
        raise ValueError('complete native evidence from the same candidate is required')
    with tarfile.open(folder/'source.tar') as archive:
        initial_tests = archive.extractfile(str(TEST_SOURCE))
        if initial_tests is None:
            raise ValueError('the exact original Rust test source is missing')
        fixture_bytes = initial_tests.read() + b'\n' + FIXTURE.read_bytes()
    for mode, row in result['views'].items():
        view = folder/mode
        expected_codec = concrete.DIGESTS['original'] if mode == 'original' else typed.DIGESTS[mode]
        if (row['staged_codec_sha256'] != expected_codec or row['compilation']['exit_code'] != 0
                or row['property_rejected'] is not (mode == 'reordered')
                or sha(view/'compile.log') != row['compilation']['log_sha256']
                or sha(view/'assertions.log') != row['assertions']['log_sha256']
                or sha(view/concrete.CODEC.name) != expected_codec
                or (view/TEST_SOURCE.name).read_bytes() != fixture_bytes
                or set(row['sources']) != {concrete.CODEC.name, TEST_SOURCE.name}
                or any(sha(view/name) != digest for name, digest in row['sources'].items())):
            raise ValueError('original native evidence differs: ' + mode)
        check_assertions(mode, row['assertions']['exit_code'], (view/'assertions.log').read_text())
    return sha(receipt)


def run_formal(output, native, tools, packages, mcp, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or len(os.sched_getaffinity(0)) < 4
            or shutil.disk_usage(output.parent).free < 80 * GIB):
        raise ValueError('require formal role, four available CPUs and 80 GiB of workspace space')
    seal = native_evidence(native)
    identity = candidate()
    output.mkdir(mode=0o700)
    checkout_env = dict(os.environ, LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    capture_env = dict(checkout_env, PYTHONPATH=str(STAGE1/'unload-parent-link'))
    formal = runpy.run_path(str(STAGE1/'formal.py'))
    result = dict(kind='source-save-order-formal', complete=False, **identity,
                  native_receipt_sha256=seal, captures={}, views={})
    try:
        # The original drivers each hold the supplied lock and leave the raw
        # archive, actual LLBC, generated Lean and command logs intact.
        for filename, modes in [('source_save_capture.py', typed.MODES),
                                ('source_save_order.py', (*typed.MODES, 'parent'))]:
            group = output / Path(filename).stem
            group.mkdir(mode=0o700)
            for mode in modes:
                receipt = command(ROOT, group, mode,
                                  [sys.executable, HERE/filename, '--mode', mode, '--tools', tools,
                                   '--lock', lock, '--output', group/mode], env=capture_env)
                if receipt['exit_code'] != 0:
                    raise ValueError('actual source-save capture failed: ' + filename + ' ' + mode)
                result['captures'][filename+' '+mode] = receipt
        captures = output/'source_save_order'
        os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
        for mode, library in ROOTS.items():
            project, evidence = output/('lean-'+mode), output/('audit-'+mode)
            materialize(captures/mode, captures/'parent', captures/'factored',
                        project, packages, mcp, mode)
            shutil.copy2(ROOT/'tools/proof_services.py', project/'tools/proof_services.py')
            (project/'artifacts').mkdir(exist_ok=True)
            evidence.mkdir(mode=0o700)
            plan = json.loads((project/'plan.json').read_bytes())
            if len(plan['files']) != 17:
                raise ValueError('expected the original 17 materialized Lean files: ' + mode)
            commands = {}

            def check(project, destination, label, argv):
                receipt = command(project, destination, label, argv, env=checkout_env)
                commands[label] = receipt
                if receipt['exit_code'] != 0:
                    raise ValueError('actual Lean command failed: ' + label)

            check(project, evidence, 'lake', ['flock', '-n', lock, 'lake', 'build', library])
            # The original service itself does not take the host lock. Keep
            # it for the whole physical-file pass.
            with lock.open('r+') as guard:
                fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
                actual = formal['actual_files'](project, tuple(plan['files']), evidence, checkout_env, check)
            if actual['files'] != plan['files'] or len(actual['audits']) != AUDIT_COUNTS[mode]:
                raise ValueError('original Lean file or audit inventory changed: ' + mode)
            result['views'][mode] = dict(actual=actual, commands=commands, plan_sha256=sha(project/'plan.json'))
        if seal != native_evidence(native) or identity != candidate():
            raise ValueError('native or source evidence changed during formal checks')
        result['complete'] = True
    finally:
        write_json(output/'result.json', result)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    choices = cli.add_subparsers(dest='role', required=True)
    native = choices.add_parser('native')
    native.add_argument('--work', type=Path, required=True)
    formal = choices.add_parser('formal')
    for name in ('native', 'tools', 'lake-packages', 'lean-mcp', 'lock'):
        formal.add_argument('--'+name, type=Path, required=True)
    cli.add_argument('--output', type=Path, required=True)
    opts = cli.parse_args()
    os.umask(0o077)
    if opts.role == 'native':
        run_native(opts.output.resolve(), opts.work.resolve())
    else:
        run_formal(opts.output.resolve(), opts.native.resolve(strict=True), opts.tools.resolve(strict=True),
                   opts.lake_packages.resolve(strict=True), opts.lean_mcp.resolve(strict=True),
                   opts.lock.resolve(strict=True))


if __name__ == '__main__':
    main()
