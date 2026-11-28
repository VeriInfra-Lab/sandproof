"""Finite native checks of the actual Stop dispatch/receipt call chain.

The execution, binary/source admission and restored-case checks follow the
existing pause-stop component. Only this component's fixture and one bounded
mutation differ; no business algorithm is replaced by a verification model.
"""
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

HERE = Path('verification/experiments/pause-dispatch')
SOURCE = Path('controller-core/src/effect.rs')
TEST_PARENT = Path('controller-core/src/capture/tests.rs')
TEST_PREFIX = 'capture::tests::pause_dispatch_proof::'
TEST_HOOK = '''
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../verification/experiments/pause-dispatch/tests.rs"
));
'''
BOUND_ASSERTION = "successful dispatch must mark the same complete Stop and preserve the remaining World"
BOUND_WRITE = 'next.effects.put(id, EffectStatus::Dispatched);'
WRONG_WRITE = 'next.effects\n        .put(misbound_stop(id), EffectStatus::Dispatched);'
WRONG_HELPER = '''
fn misbound_stop(id: EffectId) -> EffectId {
    match id.logical.command {
        Command::StopVm {
            sandbox, epoch, vm, ..
        } => EffectId {
            logical: LogicalEffectId {
                command: Command::StopVm {
                    sandbox,
                    epoch,
                    generation: 0,
                    vm,
                },
            },
            attempt: id.attempt,
        },
        _ => id,
    }
}

'''
CASES = [('wrong-stop-binding', str(SOURCE), BOUND_WRITE, WRONG_WRITE, (
    'dispatch_keeps_the_fresh_commit_stop_binding',
    'commit_and_dispatch_keep_an_existing_nonzero_attempt',
    'unknown_replay_keeps_the_same_current_nonzero_attempt',
    'actual_stop_receipt_connects_registration_to_publication',
    'receipt_replays_conflicts_and_stale_attempts_are_distinct',
))]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def mutate(root, case):
    _, relative, before, after, _ = case
    path = Path(root)/relative
    text = path.read_text()
    begin = text.index('pub fn dispatch_effect(')
    end = text.index('\n#[must_use]\npub fn acknowledge_effect(', begin)
    if text.count(before) != 1 or text[begin:end].count(before) != 1 or 'fn misbound_stop(' in text:
        raise ValueError('Mutation must name the unique real dispatch effect-row write')
    text = text[:begin] + text[begin:end].replace(before, after, 1) + text[end:]
    marker = '#[must_use]\npub fn dispatch_effect('
    if text.count(marker) != 1:
        raise ValueError('Mutation must preserve the unique actual dispatch entry')
    path.write_text(text.replace(marker, WRONG_HELPER.lstrip('\n')+marker, 1))
    return sha(path)


def run(args):
    if args.archive.is_symlink() or sha(args.archive) != args.archive_sha256:
        raise ValueError('Original source archive changed or is linked')
    if not os.environ.get('CARGO_HOME'):
        raise ValueError('Set the approved offline CARGO_HOME explicitly')
    os.umask(0o077)
    out = args.output.resolve(); out.mkdir(mode=0o700)
    original = out/'source'; original.mkdir(mode=0o700)
    inputs = {}
    with tarfile.open(args.archive) as archive:
        for member in archive.getmembers():
            path = Path(member.name)
            if (path.is_absolute() or '..' in path.parts or member.name in inputs
                    or not member.isfile() or member.mode not in (0o644, 0o755)):
                raise ValueError('Source archive must contain unique regular committed files')
            inputs[member.name] = dict(sha256=hashlib.sha256(archive.extractfile(member).read()).hexdigest(),
                                       executable=bool(member.mode & 0o111))
        archive.extractall(original, filter='data')
    if sha(original/HERE/'native.py') != sha(Path(__file__).resolve()):
        raise ValueError('Native driver differs from its archived source')
    fixture = original/HERE/'tests.rs'
    tests = {TEST_PREFIX+name for name in re.findall(r'#\[test\]\s+fn\s+(\w+)', fixture.read_text())}
    if not tests or not {TEST_PREFIX+name for case in CASES for name in case[4]} <= tests:
        raise ValueError('Original fixture does not contain the selected assertions')
    started = time.monotonic()
    result = dict(kind='pause-dispatch-native', head=args.head,
                  archive_sha256=args.archive_sha256, complete=False, commands=[], variants=[],
                  fixture_sha256=sha(fixture), tests=sorted(tests), original_inputs=inputs,
                  whole_V4=False, whole_system=False, VM_run=False)

    def save():
        result['seconds'] = time.monotonic()-started
        (out/'result.json').write_text(json.dumps(result, indent=2)+'\n')

    def command(label, argv, root, env, expected=0):
        begin = time.monotonic()
        timeout = None
        try:
            done = subprocess.run(argv, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=1800)
        except subprocess.TimeoutExpired as error:
            timeout = error
            done = subprocess.CompletedProcess(argv, None, error.stdout or b'', error.stderr or b'')
        for stream in ('stdout', 'stderr'):
            (out/(label+'.'+stream)).write_bytes(getattr(done, stream))
        row = dict(label=label, argv=list(map(str, argv)), cwd=str(root), exit_code=done.returncode,
                   seconds=time.monotonic()-begin,
                   timed_out=timeout is not None,
                   stdout_sha256=sha(out/(label+'.stdout')), stderr_sha256=sha(out/(label+'.stderr')))
        result['commands'].append(row); save()
        if timeout is not None:
            raise timeout
        if done.returncode != expected:
            raise ValueError('Unexpected command result: '+label)
        return done, row

    def checked_binary(data, root, target):
        found = []
        for line in data.splitlines():
            try:
                entry = json.loads(line)
            except (ValueError, UnicodeError):
                continue
            if entry.get('reason') != 'compiler-artifact' or not entry.get('executable'):
                continue
            package = entry['package_id'].rsplit('#', 1)[0]
            executable = Path(entry['executable'])
            if (entry['manifest_path'] != str(root/'controller-core/Cargo.toml')
                    or entry['target']['name'] != 'sandbox_controller_core'
                    or entry['target']['kind'] != ['lib']
                    or entry['target']['src_path'] != str(root/'controller-core/src/lib.rs')
                    or entry['profile']['test'] is not True or entry['fresh'] is not False
                    or not package.startswith('path+file:')
                    or Path(unquote(urlparse(package[5:]).path)) != root/'controller-core'
                    or executable.is_symlink() or not executable.is_file()
                    or not executable.resolve().is_relative_to(target.resolve())):
                raise ValueError('Core test artifact must be fresh and bound to this real source')
            found.append(dict(path=str(executable), sha256=sha(executable),
                              fresh=False, message=entry))
        if len(found) != 1:
            raise ValueError('Expected exactly one freshly compiled core unit-test executable')
        return found[0]

    def snapshot_matches(project, expected):
        for name, value in expected.items():
            path = project/name; info = path.lstat()
            if (not stat.S_ISREG(info.st_mode) or sha(path) != value['sha256']
                    or bool(info.st_mode & 0o111) != value['executable']):
                raise ValueError('Checked source bytes or mode changed: '+name)

    try:
        with args.lock.open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            for name in ('correct', *(case[0] for case in CASES), 'restored'):
                folder = out/name; project = folder/'source'
                folder.mkdir(mode=0o700); shutil.copytree(original, project)
                expected = {key: dict(value) for key, value in inputs.items()}
                parent = project/TEST_PARENT
                if 'pause_dispatch_proof' in parent.read_text() or TEST_HOOK in parent.read_text():
                    raise ValueError('Original test module already contains the private fixture')
                with parent.open('a') as stream:
                    stream.write(TEST_HOOK)
                expected[str(TEST_PARENT)]['sha256'] = sha(parent)
                case = next((case for case in CASES if case[0] == name), None)
                if case is not None:
                    digest = mutate(project, case)
                    expected[case[1]]['sha256'] = digest
                target = folder/'target'; temporary = folder/'tmp'; temporary.mkdir(mode=0o700)
                env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true',
                           CARGO_INCREMENTAL='0', CARGO_BUILD_JOBS='4',
                           RUST_TEST_THREADS='1', TMPDIR=str(temporary))
                command(name+'-format', ['cargo', 'fmt', '--all', '--', '--check'], project, env)
                command(name+'-fixture-format', ['rustfmt', '--edition', '2024', '--check',
                                                 str(HERE/'tests.rs')], project, env)
                build, _ = command(name+'-build', ['cargo', 'test', '--locked', '--offline', '-p',
                    'sandbox-controller-core', '--all-features', '--lib', '--no-run',
                    '--message-format=json'], project, env)
                binary = checked_binary(build.stdout, project, target)
                expected_failures = {TEST_PREFIX+name for name in case[4]} if case else set()
                execution, row = command(name+'-assertions', [binary['path'], TEST_PREFIX,
                    '--test-threads=1'], project, env, 101 if case else 0)
                text = execution.stdout.decode(errors='replace')
                observed = {match[0] for match in re.findall(r'^test (\S+) \.\.\. (ok|FAILED)$', text, re.M)}
                failures = {match[0] for match in re.findall(r'^test (\S+) \.\.\. (FAILED)$', text, re.M)}
                if (observed != tests or failures != expected_failures
                        or f'{len(tests)-len(failures)} passed; {len(failures)} failed; 0 ignored;' not in text
                        or (case and (b'panicked at' not in execution.stdout+execution.stderr
                                     or text.count(BOUND_ASSERTION) != len(expected_failures)))):
                    raise ValueError('Original exact pause-dispatch assertions did not match')
                row.update(observed_tests=sorted(observed), observed_failures=sorted(failures),
                           expected_failures=sorted(expected_failures), property_rejected=bool(case),
                           binary=binary)
                if sha(binary['path']) != binary['sha256']:
                    raise ValueError('Fresh test executable changed while running')
                command(name+'-clippy', ['cargo', 'clippy', '--locked', '--offline', '-p',
                    'sandbox-controller-core', '--all-features', '--all-targets', '--', '-D', 'warnings'], project, env)
                snapshot_matches(project, expected)
                variant = dict(name=name, complete=True, binary=binary,
                               observed_failures=sorted(failures), input_changes={key:value for key,value in expected.items()
                                   if value != inputs[key]})
                if case is None:
                    result[name] = variant
                else:
                    variant.update(source=case[1], source_sha256=sha(project/case[1]),
                                   build_exit=0, assertion_exit=101, expected_failure=case[4])
                    result['variants'].append(variant)
                save()
            snapshot_matches(original, inputs)
            if sha(args.archive) != args.archive_sha256:
                raise ValueError('Original source archive changed during native checks')
            result['complete'] = True
    except BaseException as error:
        result.update(error=repr(error), traceback=traceback.format_exc())
    finally:
        save()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--archive-sha256', required=True)
    parser.add_argument('--head', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    args = parser.parse_args()
    result = run(args)
    print(json.dumps({key:result.get(key) for key in ('head','complete','seconds','error')}))
    raise SystemExit(0 if result['complete'] else 1)


if __name__ == '__main__':
    main()
