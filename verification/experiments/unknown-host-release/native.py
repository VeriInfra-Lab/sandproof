"""Native checks of the actual journal release wrapper; no VM or Lean run."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tarfile
import time
import traceback
from urllib.parse import unquote, urlparse

HERE = Path('verification/experiments/unknown-host-release')
SOURCE = Path('controller-service/src/controller/bounded.rs')
FIXTURE = Path('controller-service/src/controller/lifecycle_inbox/source_work/tests/slot_reuse.rs')
BEFORE = '        release(&context).map_err(|_| JournalError::UnresolvedAction)?;\n'
AFTER = '        let _ = release(&context);\n'
PROPERTY = ('controller::lifecycle_inbox::source_work::tests::slot_reuse::'
            'two_owner_two_cycle_resume_keeps_its_released_position_across_reopen')
CONTROLS = (
    'controller::bounded_tests::host_release_never_runs_before_committed_intent_and_lost_ack_is_replayable',
    'controller::bounded_tests::commit_uncertainty_reloads_exact_candidate_not_the_assumed_release_result',
)
TESTS = (PROPERTY, *CONTROLS)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def mutate(project):
    path = project / SOURCE
    text = path.read_text()
    start = text.index('    pub fn release_host_slot(')
    end = text.index('\n    // Same two original transactions', start)
    if text.count(BEFORE) != 1 or text[start:end].count(BEFORE) != 1:
        raise ValueError('Expected the unique original release-error propagation')
    path.write_text(text.replace(BEFORE, AFTER, 1))


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    marker = ('    assert!(\n        journal\n            .release_host_slot(\n'
              '                moved,\n')
    if text.count(marker) != 1:
        raise ValueError('Expected the unchanged original unknown-release assertion')
    return text[:text.index(marker)].count('\n') + 1


def checked_execution(stdout, stderr, code, name, rejected, line):
    text = stdout.decode(errors='replace')
    status = 'FAILED' if rejected else 'ok'
    if (code != (101 if rejected else 0)
            or re.findall(r'^test (\S+) \.\.\. (ok|FAILED|ignored)$', text, re.M) != [(name, status)]
            or 'running 1 test\n' not in text
            or f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; 0 ignored;' not in text):
        raise ValueError('The exact original assertion did not produce the required result')
    if rejected:
        combined = stdout + stderr
        location = (str(FIXTURE) + ':' + str(line) + ':').encode()
        if (b'panicked at ' not in combined or location not in combined
                or b'assertion failed:' not in combined
                or b'release_host_slot(moved,' not in combined):
            raise ValueError('Wrong implementation failed outside the original rejection assertion')


def checked_binary(data, project, target):
    found = []
    for line in data.splitlines():
        try:
            row = json.loads(line)
        except (ValueError, UnicodeError):
            continue
        if row.get('reason') != 'compiler-artifact' or not row.get('executable'):
            continue
        package = row['package_id'].rsplit('#', 1)[0]
        executable = Path(row['executable'])
        if (row['manifest_path'] != str(project / 'controller-service/Cargo.toml')
                or row['target']['src_path'] != str(project / 'controller-service/src/lib.rs')
                or row['target']['name'] != 'sandproof_controller_service'
                or row['target']['kind'] != ['lib'] or row['profile']['test'] is not True
                or row['fresh'] is not False or not package.startswith('path+file:')
                or Path(unquote(urlparse(package[5:]).path)) != project / 'controller-service'
                or executable.is_symlink() or not executable.is_file()
                or not executable.resolve().is_relative_to(target.resolve())):
            raise ValueError('Require a fresh actual service libtest from this source and target')
        found.append(dict(path=str(executable), sha256=sha(executable), artifact=row))
    if len(found) != 1:
        raise ValueError('Expected exactly one freshly compiled actual service libtest')
    return found[0]


def unpack(archive, project, tree):
    inputs = {}
    seen = set()
    directories = {str(parent) for name in tree for parent in Path(name).parents
                   if str(parent) != '.'}
    with tarfile.open(archive) as stream:
        for item in stream.getmembers():
            path = Path(item.name)
            if path.is_absolute() or '..' in path.parts or item.name in seen:
                raise ValueError('Archive contains an unsafe or repeated member')
            seen.add(item.name)
            if item.isdir():
                if item.name not in directories or item.mode != 0o755:
                    raise ValueError('Archive contains an uncommitted or writable directory')
                continue
            if (not item.isfile() or item.mode not in (0o644, 0o755)
                    or item.name not in tree):
                raise ValueError('Archive must contain only unique regular committed files')
            data = stream.extractfile(item).read()
            blob = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
            mode = '100755' if item.mode == 0o755 else '100644'
            if tree[item.name] != (mode, blob):
                raise ValueError('Archive differs from the exact committed tree: ' + item.name)
            inputs[item.name] = dict(sha256=hashlib.sha256(data).hexdigest(), mode=item.mode)
        if set(inputs) != set(tree):
            raise ValueError('Archive omits committed input files')
        stream.extractall(project, filter='data')
    return inputs


def check_inputs(project, inputs, changed=None):
    for name, expected in inputs.items():
        path = project / name
        info = path.lstat()
        digest = changed if name == str(SOURCE) and changed else expected['sha256']
        if (not stat.S_ISREG(info.st_mode) or sha(path) != digest
                or stat.S_IMODE(info.st_mode) != expected['mode']):
            raise ValueError('Source inputs changed during the check: ' + name)


def run(args):
    os.umask(0o077)
    root = Path(__file__).resolve().parents[3]
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
    if args.head != head or subprocess.check_output(['git', 'status', '--porcelain'], cwd=root):
        raise ValueError('Execute the committed driver from the exact clean candidate')
    if args.archive.is_symlink() or sha(args.archive) != args.archive_sha256:
        raise ValueError('Original source archive changed or is linked')
    if not os.environ.get('CARGO_HOME'):
        raise ValueError('Set the original approved offline CARGO_HOME explicitly')
    if os.environ.get('RUST_TEST_THREADS'):
        raise ValueError('Keep the default libtest parallelism')
    if any(os.environ.get(name) for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS')):
        raise ValueError('Do not override the original compiler flags')
    for name in ('CARGO_PROFILE_DEV_DEBUG_ASSERTIONS', 'CARGO_PROFILE_DEV_OVERFLOW_CHECKS',
                 'CARGO_PROFILE_TEST_DEBUG_ASSERTIONS', 'CARGO_PROFILE_TEST_OVERFLOW_CHECKS'):
        if name in os.environ and os.environ[name].lower() not in ('true', '1'):
            raise ValueError('Keep debug assertions and overflow checks enabled')
    work = args.work_root.resolve(strict=True)
    info = args.work_root.lstat()
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
            or stat.S_IMODE(info.st_mode) != 0o700 or any(work.iterdir())):
        raise ValueError('Use an empty owner-only payload allocated by the original retention tool')
    out = args.output.resolve()
    target = args.target.resolve(strict=True)
    if (out.is_relative_to(work) or target.is_relative_to(work)
            or out == target or out.is_relative_to(target) or target.is_relative_to(out)):
        raise ValueError('Keep evidence, disposable sources and the original cache separate')
    out.mkdir(mode=0o700)
    started = time.monotonic()
    result = dict(kind='unknown-host-release-native', head=head, complete=False, commands=[],
                  variants=[], archive_sha256=args.archive_sha256, tests=list(TESTS),
                  KVM=False, Lean=False, whole_six_variants=False,
                  mutation=dict(path=str(SOURCE), before=BEFORE, after=AFTER),
                  environment={key: os.environ.get(key) for key in ('CARGO_HOME', 'CARGO_BUILD_JOBS',
                      'RUST_TEST_THREADS', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS')})

    def save():
        result['seconds'] = time.monotonic() - started
        (out / 'result.json').write_text(json.dumps(result, indent=2) + '\n')

    def command(label, argv, project, env):
        begin = time.monotonic()
        try:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=1800)
        except subprocess.TimeoutExpired as error:
            (out / (label + '.stdout')).write_bytes(error.stdout or b'')
            (out / (label + '.stderr')).write_bytes(error.stderr or b'')
            result['commands'].append(dict(label=label, argv=argv, timed_out=True,
                                           seconds=time.monotonic() - begin))
            save()
            raise
        (out / (label + '.stdout')).write_bytes(done.stdout)
        (out / (label + '.stderr')).write_bytes(done.stderr)
        row = dict(label=label, argv=argv, cwd=str(project), exit_code=done.returncode, timed_out=False,
                   seconds=time.monotonic() - begin,
                   stdout_sha256=sha(out / (label + '.stdout')),
                   stderr_sha256=sha(out / (label + '.stderr')))
        result['commands'].append(row)
        save()
        return done

    try:
        tree = {}
        for row in subprocess.check_output(['git', 'ls-tree', '-rz', head], cwd=root).split(b'\0'):
            if not row:
                continue
            meta, path = row.split(b'\t', 1)
            mode, kind, blob = meta.decode().split()
            if kind != 'blob' or mode not in ('100644', '100755'):
                raise ValueError('Only regular committed source files are supported')
            tree[path.decode()] = (mode, blob)
        original = work / 'original'
        original.mkdir(mode=0o700)
        inputs = unpack(args.archive, original, tree)
        if sha(original / HERE / 'native.py') != sha(Path(__file__)):
            raise ValueError('Driver differs from archived candidate')
        result['inputs'] = inputs
        lock_fd = os.open(args.lock, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
        with os.fdopen(lock_fd, 'r+') as lock:
            lock_info = os.fstat(lock.fileno())
            if (not stat.S_ISREG(lock_info.st_mode) or lock_info.st_uid != os.getuid()
                    or stat.S_IMODE(lock_info.st_mode) != 0o600 or lock_info.st_nlink != 1):
                raise ValueError('Use the original owner-only heavy-task lock')
            fcntl.flock(lock, fcntl.LOCK_EX)
            result['lock_identity'] = [lock_info.st_dev, lock_info.st_ino]
            for tool in ('cargo', 'rustc'):
                version = command(tool + '-version', [tool, '--version'], original, dict(os.environ))
                if version.returncode or not version.stdout.startswith((tool + ' 1.97.0 ').encode()):
                    raise ValueError('Use the fixed Rust/Cargo 1.97.0 toolchain')
            for variant in ('correct', 'ignore-host-error', 'restored'):
                project = work / variant
                shutil.copytree(original, project)
                if variant == 'ignore-host-error':
                    mutate(project)
                changed = sha(project / SOURCE) if variant == 'ignore-host-error' else None
                check_inputs(project, inputs, changed)
                env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
                           CARGO_INCREMENTAL='0')
                build = command(variant + '-build', ['cargo', 'test', '--locked', '--offline',
                    '-p', 'sandproof-controller-service', '--lib', '--no-run', '--message-format=json'],
                    project, env)
                if build.returncode != 0:
                    raise ValueError('Compilation failed, not a property rejection: ' + variant)
                binary = checked_binary(build.stdout, project, target)
                line = rejection_line(project)
                for index, name in enumerate(TESTS):
                    execution = command(variant + '-test-' + str(index),
                                        [binary['path'], name, '--exact'], project, env)
                    checked_execution(execution.stdout, execution.stderr, execution.returncode,
                                      name, variant == 'ignore-host-error' and name == PROPERTY, line)
                if sha(binary['path']) != binary['sha256']:
                    raise ValueError('Actual libtest changed during execution')
                check_inputs(project, inputs, changed)
                result['variants'].append(dict(name=variant, binary=binary,
                    source_sha256=sha(project / SOURCE), fixture_sha256=sha(project / FIXTURE),
                    rejected=variant == 'ignore-host-error', expected_assertion_line=line))
                save()
        check_inputs(original, inputs)
        if sha(args.archive) != args.archive_sha256:
            raise ValueError('Archive changed during execution')
        result['complete'] = True
    except BaseException as error:
        result.update(error=repr(error), traceback=traceback.format_exc())
    finally:
        save()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('archive', 'work-root', 'target', 'output', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--archive-sha256', required=True)
    parser.add_argument('--head', required=True)
    result = run(parser.parse_args())
    print(json.dumps({key: result.get(key) for key in ('head', 'complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)


if __name__ == '__main__':
    main()
