"""Run proof-batch native obligations on the test host for the proof host."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import stat

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools.acceptance_receipts import (ROOT, digest, native_case, native_run,
    rules, rust_inputs, sha, write_json, tree_digest, locked, repository_inputs, load_success,
    contract_inputs, source_archive_inputs, NATIVE_SCHEMA, NATIVE_SCOPES,
    check_native_scope, check_native_components)
from tools.ci_retention import NativeBuildRun

JSON_NEGATIVE_DRIVERS = ('check_json_request_negative.py',
                         'check_json_identifiers_negative.py',
                         'check_json_counter_negative.py')


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def runtime(*, temporary_parent=None):
    result = {}
    for name, arguments in [('rustc', ['-vV']), ('cargo', ['-V']),
                            ('rustfmt', ['--version']), ('clippy-driver', ['--version']),
                            ('cc', ['--version'])]:
        path = shutil.which(name)
        if path is None:
            raise ValueError('missing native tool: ' + name)
        # rustup proxies are not the actual compiler binaries.
        if name in ('rustc', 'cargo', 'rustfmt', 'clippy-driver'):
            path = subprocess.check_output(['rustup', 'which', name], text=True).strip()
        result[name] = dict(sha256=sha(path), version=subprocess.check_output([path, *arguments], text=True))
    frozen = json.loads((ROOT / 'verification/first-chain-linux-environment.json').read_text())
    if result['rustc']['version'].splitlines()[0] != frozen['production_compiler']:
        raise ValueError('native compiler differs from frozen production compiler')
    result['target_cfg'] = subprocess.check_output(['rustc', '--print', 'cfg', '--target', frozen['target']], text=True)
    redis = Path(os.environ['SANDPROOF_REDIS_SERVER']).resolve()
    result['redis'] = dict(sha256=sha(redis), version=subprocess.check_output([redis, '--version'], text=True))
    result['python'] = sys.version
    result['platform'] = sys.platform
    result['uid'] = os.getuid()
    result['gid'] = os.getgid()
    if temporary_parent is not None:
        result['temporary_parent'] = temporary_parent_identity(temporary_parent)
    cargo_home=Path(os.environ.get('CARGO_HOME',str(Path.home()/'.cargo')))
    result['registry']=tree_digest(cargo_home/'registry/src')
    result['cargo_config']={str(p):sha(p) for p in
        (cargo_home/'config',cargo_home/'config.toml') if p.is_file()}
    sysroot=Path(subprocess.check_output(['rustc','--print','sysroot'],text=True).strip())
    result['rust_libraries']=tree_digest(sysroot/'lib')
    return result


def command(command, source=ROOT, label='command', expected=0, env=None):
    fd, name = tempfile.mkstemp(prefix='native-log-', dir=os.environ['SANDPROOF_NATIVE_EVIDENCE'])
    os.close(fd)
    path = Path(name)
    native_run(label, command, source, path, env, expected)
    sys.stdout.write(path.read_text())
    path.unlink()


def trusted_checkout(root):
    names=subprocess.check_output(['git','ls-files','-z'],cwd=root).decode().split('\0')
    directories={root,*root.parents}
    for name in names:
        if name:
            directory=(root/name).parent
            while directory.is_relative_to(root):
                directories.add(directory)
                if directory==root:break
                directory=directory.parent
    for path in directories:
        info=path.lstat()
        writes_ok=not info.st_mode&0o022 or (info.st_uid==0 and info.st_mode&stat.S_ISVTX)
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0,os.getuid()) or not writes_ok):
            raise ValueError('untrusted checkout ancestry; prepare a private checkout with umask 077: '+str(path))



def temporary_parent_identity(parent):
    info = parent.lstat()
    return dict(path=str(parent), device=info.st_dev, inode=info.st_ino,
                uid=info.st_uid, gid=info.st_gid, mode=oct(info.st_mode & 0o7777))


def private_temporary_root():
    """Keep the configured filesystem and the original trusted-parent policy."""
    configured = os.environ.get('TMPDIR')
    parent = Path('/tmp') if configured is None else Path(configured)
    if configured == '' or not parent.is_absolute() or '..' in parent.parts:
        raise ValueError('native temporary parent must be an absolute nonempty path')
    for path in (parent, *parent.parents):
        info = path.lstat()
        writes_ok = not info.st_mode & 0o022 or (info.st_uid == 0 and info.st_mode & stat.S_ISVTX)
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid not in (0, os.getuid()) or not writes_ok):
            raise ValueError('untrusted native temporary ancestry: ' + str(path))
    if parent == Path('/tmp'):
        info = parent.lstat()
        if info.st_uid != 0 or not info.st_mode & stat.S_ISVTX:
            raise ValueError('native fixture requires the normal root-owned sticky /tmp')
    return Path(tempfile.mkdtemp(prefix='sandproof-native-', dir=parent))


def run_suite(output, *, scope='all'):
    check_native_scope(scope)
    output.mkdir(parents=True,exist_ok=True,mode=0o700)
    with locked(output/'batch-lock'):
        return run_locked_suite(output, scope=scope)


def run_locked_suite(output, *, scope='all'):
    check_native_scope(scope)
    trusted_checkout(ROOT)
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=True)
    # A private child keeps protected-file tests strict while retaining the
    # operator's configured filesystem (the CoW cases require qualified XFS).
    private_tmp = private_temporary_root()
    os.environ['TMPDIR']=str(private_tmp);tempfile.tempdir=None
    write_json(output/'temporary-root.json',dict(path=str(private_tmp),parent=str(private_tmp.parent),
        uid=private_tmp.stat().st_uid,mode=oct(private_tmp.stat().st_mode&0o777)))
    os.environ['SANDPROOF_NATIVE_EVIDENCE'] = str(output)
    os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'native'
    initial = dict(rust=rust_inputs(ROOT), rules=rules(), runtime=runtime(temporary_parent=Path(os.environ['TMPDIR']).parent),repository=repository_inputs(ROOT),
                   contracts=contract_inputs(ROOT),source_archives=source_archive_inputs())
    write_json(output / 'runtime.json', initial['runtime'])
    started = time.monotonic()
    work = Path(tempfile.mkdtemp(prefix='run-', dir=output))
    builds = NativeBuildRun(work, subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
    previous=work/'previous-complete.json'
    if (output/'complete.json').exists():
        previous.write_bytes((output/'complete.json').read_bytes())
    (output / 'complete.json').unlink(missing_ok=True)
    os.environ['SANDPROOF_NATIVE_PREVIOUS']=str(previous)
    os.environ['SANDPROOF_NATIVE_BATCH']=str(time.time_ns())+'-'+str(os.getpid())
    os.environ['SANDPROOF_NATIVE_USED']=str(work/'commands-used.jsonl')
    def call(args):
        subprocess.run([sys.executable, *map(str, args)], cwd=ROOT, check=True)
    # Full Linux functional gate is still required, including every tool test,
    # no-default and diagnostics features, doctests, strict Clippy and docs.
    native_case('functional', ROOT, work / 'functional', lambda: functional(work), scope=scope)
    if scope == 'stage1-protocol':
        run_stage1_native(work, scope=scope)
        return publish_suite(output, work, initial, started, scope=scope, builds=builds)
    if scope == 'capture-caller':
        run_capture_caller_native(work, scope=scope)
        return publish_suite(output, work, initial, started, scope=scope, builds=builds)
    if scope == 'pause-publication':
        run_pause_publication_native(work, scope=scope)
        return publish_suite(output, work, initial, started, scope=scope, builds=builds)
    if scope == 'pause-stop':
        run_pause_stop_native(work, scope=scope)
        return publish_suite(output, work, initial, started, scope=scope, builds=builds)
    # These are the exact commands previously embedded in proof shell entries.
    # Identical commands/inputs execute once even when several modules ask.
    for cmd in [['cargo','fmt','--all','--','--check'],['cargo','test','--workspace'],
                ['cargo','clippy','--workspace','--all-targets','--','-D','warnings']]:
        command(cmd)
    for name in ('controller','foundations','json-request','json-identifiers',
                 'network','first-chain-linux','data-drain','task-contract','json-counter'):
        text = (ROOT / f'scripts/verify-{name}.sh').read_text().replace('\\\n', '')
        for line in text.splitlines():
            prefix = 'bash scripts/acceptance-native.sh '
            if line.startswith(prefix):
                command(shlex.split(line[len(prefix):]))
    for name, filename in [('task','check_task_contract_components.py'),
                           ('revocation','check_revocation_components.py'),
                           ('owned-message','check_owned_message_components.py'),
                           ('notify','check_notify_components.py'),
                           ('joinset','check_joinset_components.py'),
                           ('std-mutex','check_std_mutex_components.py'),
                           ('condvar','check_condvar_components.py'),
                           ('tokio-rwlock','check_tokio_rwlock_components.py'),
                           ('callback-state','check_callback_state_components.py')]:
        target = work / name
        native_case(name, ROOT, target, lambda n=filename, p=target:
                    call(['tools/' + n, '--output', p]))
    for name, driver in [('btree-set-binding','tools/check_btree_set_binding_components.py'),
                         ('tokio-mutex','verification/experiments/tokio-mutex/native.py')]:
        target=work/name
        native_case(name, ROOT, target, lambda d=driver,p=target: call([d,'--output',p]))
    target=work/'protected-directory'
    library_archive=Path(os.environ['SANDPROOF_LIBRARY_SOURCE_ARCHIVE']).resolve(strict=True)
    def protected_directory_native():
        call(['verification/experiments/protected-directory/check.py'])
        call(['verification/experiments/protected-directory/test_checks.py'])
        call(['verification/experiments/protected-directory/native.py','--archive',library_archive,'--output',target])
    native_case('protected-directory',ROOT,target,protected_directory_native)
    run_capture_caller_native(work)
    run_pause_publication_native(work)
    run_pause_stop_native(work)
    run_stage1_native(work)
    target = work / 'schema-domains'
    native_case('schema-domains', ROOT, target, lambda p=target:
        call(['verification/experiments/schema-domains/native.py', '--output', p]))
    target = work / 'loop-callbacks'
    native_case('loop-callbacks', ROOT, target, lambda p=target:
        call(['tools/check_loop_callbacks_components.py', '--output', p]))
    target = work / 'bounded-writer'
    writer_archive=Path(os.environ['SANDPROOF_WRITER_SOURCE_ARCHIVE']).resolve(strict=True)
    native_case('bounded-writer', ROOT, target, lambda p=target:
        call(['verification/experiments/bounded-writer/native.py', '--source-archive', writer_archive, '--output', p]))
    target = work / 'child-observation'
    child_archive=Path(os.environ['SANDPROOF_CHILD_OBSERVATION_SOURCE_ARCHIVE']).resolve(strict=True)
    native_case('child-observation', ROOT, target, lambda p=target:
        call(['verification/experiments/child-observation/native.py', '--archive', child_archive, '--output', p]))
    target = work / 'sqlite-statements'
    archive = Path(os.environ['SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE']).resolve(strict=True)
    native_case('sqlite-statements', ROOT, target, lambda p=target:
        call(['verification/experiments/sqlite-statements/native.py',
              '--source-archive', archive, '--output', p]))
    target = work / 'sqlite-correct'
    native_case('sqlite-correct', ROOT, target, lambda:
        call(['verification/experiments/sqlite-commit/native.py', '--source-base', ROOT,
              '--destination', target]))
    for driver in JSON_NEGATIVE_DRIVERS:
        call(['tools/' + driver, '--native-only', '--output', work / driver.removesuffix('.py')])
    call(['verification/experiments/tunnel-contract/replay.py', '--native-only',
          '--source-base', ROOT, '--destination', work / 'tunnel', '--tool-root', ROOT / '.tools'])
    for kind in ('sqlite-commit','redis-result'):
        call([f'verification/experiments/{kind}/negative.py', '--native-only',
              '--source-base', ROOT, '--destination', work / (kind+'-negative'),
              '--correct-replay', work / 'unused', '--tool-root', ROOT / '.tools'])
    call(['verification/experiments/redis-result/replay.py', '--native-only',
          '--source-base', ROOT, '--destination', work / 'redis-correct', '--tool-root', ROOT / '.tools'])
    source = work / 'redis-correct/source'
    command(['cargo','clippy','-p','sandproof-center-store','--all-targets','--features','redis-tests',
             '--locked','--offline','--','-D','warnings'], source)
    command(['cargo','test','-p','sandproof-center-store','--features','redis-tests','--locked','--offline'],
            source, env=dict(os.environ, RUST_TEST_THREADS='1'))
    from tools.tls_pin_reproduction import reproduce
    reproduce(ROOT, ROOT / '.tools', work / 'tls', native_only=True)
    return publish_suite(output, work, initial, started, scope=scope, builds=builds)


def run_capture_caller_native(work, *, scope='all'):
    target = work / 'capture-caller'
    def original_component():
        subprocess.run([sys.executable, 'tools/check_capture_caller_components.py',
                        'native', '--output', str(target)], cwd=ROOT, check=True)
    native_case('capture-caller', ROOT, target, original_component, scope=scope)


def run_pause_publication_native(work, *, scope='all'):
    target = work / 'pause-publication'
    def original_component():
        subprocess.run([sys.executable, 'tools/check_pause_publication_components.py',
                        'native', '--output', str(target)], cwd=ROOT, check=True)
    native_case('pause-publication', ROOT, target, original_component, scope=scope)


def run_pause_stop_native(work, *, scope='all'):
    target = work / 'pause-stop'
    def original_component():
        subprocess.run([sys.executable, 'tools/check_pause_stop_components.py',
                        'native', '--output', str(target)], cwd=ROOT, check=True)
    native_case('pause-stop', ROOT, target, original_component, scope=scope)


def run_stage1_native(work, *, scope='all'):
    target = work / 'stage1-protocol'
    def retained():
        subprocess.run([sys.executable,
            'verification/experiments/stage1-protocol/component.py',
            '--native-only', '--output', str(target)], cwd=ROOT, check=True)
    native_case('stage1-protocol', ROOT, target, retained, scope=scope)
    registered = load('stage1_registered_components',
                      ROOT / 'verification/experiments/stage1-protocol/registered.py')
    registered.run_native(ROOT, work / 'stage1-v3')


def publish_suite(output, work, initial, started, *, scope='all', builds=None):
    check_native_scope(scope)
    if dict(rust=rust_inputs(ROOT), rules=rules(), runtime=runtime(temporary_parent=Path(os.environ['TMPDIR']).parent),repository=repository_inputs(ROOT),
            contracts=contract_inputs(ROOT),source_archives=source_archive_inputs()) != initial:
        raise ValueError('source, rules or native tools changed during the suite')
    trusted_checkout(ROOT)
    commands={};uses=[];keys={}
    for line in (work/'commands-used.jsonl').read_text().splitlines():
        row=json.loads(line);identity=row['identity'];value=row['sha256']
        uses.append(row)
        if identity in commands and commands[identity]!=value:
            raise ValueError('native evidence changed between uses')
        entry=output/'commands'/identity
        receipt=json.loads((entry/'receipt.json').read_text())
        if sha(entry/'receipt.json')!=value or load_success(entry,receipt['key']) is None:
            raise ValueError('native evidence lost before complete publication')
        commands[identity]=value
        keys[identity]=receipt['key']
    check_native_components(keys.values(), scope=scope)
    result = dict(schema=NATIVE_SCHEMA, scope=scope, scope_complete=True,
                  full_native_suite_complete=scope=='all',
                  complete=True, exit_code=0, rules=initial['rules'],
                  rust=initial['rust'], runtime=initial['runtime'], commands=commands,uses=uses,
                  contracts=initial['contracts'],source_archives=initial['source_archives'],
                  repository_sha256=digest(initial['repository']))
    # Keep the original completion plus a precise target inventory. Publication
    # never moves/deletes build bytes or guesses resource cleanup from exit 0.
    original = work / 'native-complete.json'
    write_json(original, result)
    if builds is not None:
        builds.finish(original)
    write_json(output / 'complete.json', result)
    print(json.dumps(dict(scope=scope, scope_complete=True, full_native_suite_complete=scope=='all',
                         native_suite_complete=scope=='all', seconds=round(time.monotonic()-started,3),
                         commands=len(commands), evidence=str(output))))


def functional(work):
    target = work / 'functional'
    target.mkdir()
    with (target / 'check-function.log').open('wb') as log:
        subprocess.run(['bash','scripts/check-function.sh'], cwd=ROOT, stdout=log,
                       stderr=subprocess.STDOUT, check=True,
                       env=dict(os.environ, SANDPROOF_CHECK_EVENTS=str(target/'events.jsonl')))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='mode', required=True)
    run = sub.add_parser('run'); run.add_argument('--output', type=Path, required=True)
    run.add_argument('--scope', choices=NATIVE_SCOPES, default='all')
    registered = load('stage1_registered_components',
                      ROOT / 'verification/experiments/stage1-protocol/registered.py')
    req = sub.add_parser('require'); req.add_argument('case', choices=['functional','task','revocation','owned-message','notify','joinset','std-mutex','condvar','tokio-rwlock','sqlite-correct','schema-domains','loop-callbacks','sqlite-statements','child-observation','callback-state','bounded-writer','btree-set-binding','protected-directory','tokio-mutex','capture-caller','pause-publication','pause-stop','stage1-protocol', *registered.native_labels()])
    req.add_argument('--scope', choices=NATIVE_SCOPES, default='all')
    cmd = sub.add_parser('command'); cmd.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    if args.mode == 'run':
        run_suite(args.output.resolve(), scope=args.scope)
    elif args.mode == 'require':
        native_case(args.case, ROOT, None, scope=args.scope)
    else:
        if not args.command:
            parser.error('missing native command')
        command(args.command, Path.cwd())


if __name__ == '__main__':
    main()
