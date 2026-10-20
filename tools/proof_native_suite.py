"""Run all proof-batch native obligations on ning; consume them on a6000."""
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
    rules, rust_inputs, sha, write_json, tree_digest, locked, repository_inputs, load_success)

JSON_NEGATIVE_DRIVERS = ('check_json_request_negative.py',
                         'check_json_identifiers_negative.py',
                         'check_json_counter_negative.py')


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def runtime():
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


def run_suite(output):
    output.mkdir(parents=True,exist_ok=True,mode=0o700)
    with locked(output/'batch-lock'):
        return run_locked_suite(output)


def run_locked_suite(output):
    trusted_checkout(ROOT)
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=True)
    # A permissive caller TMPDIR would cause genuine protected-file rejection.
    # Use a private child of Linux's root-owned sticky /tmp; no shared modes or
    # production checks are changed. Keep its identity in the private evidence.
    system_tmp=Path('/tmp');info=system_tmp.lstat()
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid!=0
            or not info.st_mode & stat.S_ISVTX):
        raise ValueError('native fixture requires the normal root-owned sticky /tmp')
    private_tmp=Path(tempfile.mkdtemp(prefix='sandproof-native-',dir=system_tmp))
    os.environ['TMPDIR']=str(private_tmp);tempfile.tempdir=None
    write_json(output/'temporary-root.json',dict(path=str(private_tmp),uid=private_tmp.stat().st_uid,
                                               mode=oct(private_tmp.stat().st_mode&0o777)))
    os.environ['SANDPROOF_NATIVE_EVIDENCE'] = str(output)
    os.environ['SANDPROOF_ACCEPTANCE_ROLE'] = 'native'
    initial = dict(rust=rust_inputs(ROOT), rules=rules(), runtime=runtime(),repository=repository_inputs(ROOT))
    write_json(output / 'runtime.json', initial['runtime'])
    started = time.monotonic()
    work = Path(tempfile.mkdtemp(prefix='run-', dir=output))
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
    native_case('functional', ROOT, work / 'functional', lambda: functional(work))
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
                           ('condvar','check_condvar_components.py')]:
        target = work / name
        native_case(name, ROOT, target, lambda n=filename, p=target:
                    call(['tools/' + n, '--output', p]))
    target = work / 'schema-domains'
    native_case('schema-domains', ROOT, target, lambda p=target:
        call(['verification/experiments/schema-domains/native.py', '--output', p]))
    target = work / 'loop-callbacks'
    native_case('loop-callbacks', ROOT, target, lambda p=target:
        call(['tools/check_loop_callbacks_components.py', '--output', p]))
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
    if dict(rust=rust_inputs(ROOT), rules=rules(), runtime=runtime(),repository=repository_inputs(ROOT)) != initial:
        raise ValueError('source, rules or native tools changed during the suite')
    trusted_checkout(ROOT)
    commands={};uses=[]
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
    result = dict(schema=1, complete=True, exit_code=0, rules=initial['rules'],
                  rust=initial['rust'], runtime=initial['runtime'], commands=commands,uses=uses,
                  repository_sha256=digest(initial['repository']))
    write_json(output / 'complete.json', result)
    print(json.dumps(dict(native_suite_complete=True, seconds=round(time.monotonic()-started,3),
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
    req = sub.add_parser('require'); req.add_argument('case', choices=['functional','task','revocation','owned-message','notify','joinset','std-mutex','condvar','sqlite-correct','schema-domains','loop-callbacks','sqlite-statements'])
    cmd = sub.add_parser('command'); cmd.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args(argv)
    if args.mode == 'run':
        run_suite(args.output.resolve())
    elif args.mode == 'require':
        native_case(args.case, ROOT, None)
    else:
        if not args.command:
            parser.error('missing native command')
        command(args.command, Path.cwd())


if __name__ == '__main__':
    main()
