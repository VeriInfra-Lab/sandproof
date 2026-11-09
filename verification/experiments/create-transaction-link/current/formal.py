"""Check the current actual recording caller and its established providers."""
import argparse
import contextlib
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
STAGE1 = HERE.parent.parent/'stage1-protocol'
sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import native_case, sha, validate_native, write_json

SCOPE = 'stage1-v3-transaction-source'
LABEL = 'stage1-create-transaction-link'
CALLER = Path('verification/aeneas/actual-record/SandproofActualRecord')


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def require(condition, message):
    if not condition:
        raise ValueError(message)


def run(output, lock, lock_fd, scope=SCOPE):
    require(os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal', 'formal host role required')
    require(1 <= len(os.sched_getaffinity(0)) <= 4, 'four CPU proof allocation required')
    require(shutil.disk_usage(output.parent).free >= 80*2**30, 'original 80 GiB proof entry floor')
    cache = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])
    validate_native(cache, scope=scope)
    native_case('functional', ROOT, None, scope=scope)
    native = native_case(LABEL, ROOT, None, scope=scope)
    output.mkdir(mode=0o700)
    started = time.monotonic()
    result = dict(kind='actual-record-transaction-source-component', complete=False, commands=[],
                  full_V3=False, begin_verified=False, automatic_drop_verified=False, V5_verified=False)
    project = output/'project'
    project.mkdir(mode=0o700)
    artifacts = project/'artifacts'
    data = json.loads((HERE/'materialization.json').read_text())
    binding = json.loads((HERE/'rust-source-binding.json').read_text())
    profile = json.loads((HERE/'caller-profile.json').read_text())
    archive = load('transaction_formal_archive', STAGE1/'archive.py')
    registry = load('transaction_formal_registered', STAGE1/'registered.py')
    services = load('transaction_formal_services', ROOT/'tools/proof_services.py')
    batch = load('transaction_formal_batch', ROOT/'tools/proof_batch.py')
    fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
    tool_dir = Path(os.environ.get('SANDPROOF_PROOF_TOOL_ROOT', ROOT/'.tools')).resolve(strict=True)
    charon = tool_dir/'aeneas/charon/bin/charon'
    aeneas = tool_dir/'aeneas/bin/aeneas'
    tools = {charon: fixed['charon']['binary_sha256'], aeneas: fixed['aeneas']['binary_sha256']}
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
               CARGO_INCREMENTAL='0', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(name, None)

    def command(label, argv):
        require(shutil.disk_usage(project).free >= 64*2**30, 'original 64 GiB proof floor')
        log = artifacts/(label+'.log')
        argv = list(map(str, argv))
        with log.open('xb') as stream:
            child = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                   stdout=stream, stderr=subprocess.STDOUT, timeout=900)
        result['commands'].append(dict(label=label, argv=argv, cwd=str(project),
            exit_code=child.returncode, log=str(log), log_sha256=sha(log)))
        require(child.returncode == 0, 'first actual tool failure: '+label)

    try:
        lock_stream = lock.open('r+') if lock_fd is None else os.fdopen(os.dup(lock_fd), 'r+')
        with lock_stream:
            info, expected = os.fstat(lock_stream.fileno()), lock.stat()
            require((info.st_dev, info.st_ino) == (expected.st_dev, expected.st_ino),
                    'proof coordination lock differs')
            fcntl.flock(lock_stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            source = archive.build(ROOT, output/'source.tar', output/'source-manifest.json')
            require(archive.verify(output/'source.tar', output/'source-manifest.json') == source,
                    'formal source archive identity differs')
            for name, digest in binding['files'].items():
                require(source['files'][name]['sha256'] == digest, 'original Rust source link differs: '+name)
            require(all(sha(path) == digest for path, digest in tools.items()), 'original extraction tools differ')
            with tarfile.open(output/'source.tar') as tar:
                tar.extractall(project, filter='data')
            artifacts.mkdir()
            (project/'.tools').symlink_to(tool_dir, target_is_directory=True)
            packages = Path(os.environ.get('SANDPROOF_FORMAL_PACKAGES', ROOT/'.lake/packages')).resolve()
            require(packages.is_dir(), 'original fixed Lake packages required')
            (project/'.lake').mkdir()
            (project/'.lake/packages').symlink_to(packages, target_is_directory=True)
            env.setdefault('CARGO_TARGET_DIR', str(output/'extract-target'))
            shutil.copyfile(HERE/'lakefile.toml.in', project/'lakefile.toml')
            all_files = {}
            for entry in data['modules'].values():
                name, digest = entry['path'], entry['sha256']
                provider = ROOT/entry['source']
                require(sha(provider) == digest, 'reviewed Lean provider differs: '+name)
                destination = project/name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(provider, destination)
                all_files[name] = digest
            if (project/'lake-manifest.json').exists():
                require(sha(project/'lake-manifest.json') == sha(ROOT/'lake-manifest.json'),
                        'fixed Lake manifest changed')
            root = project/profile['append_only']['path']
            original = root.read_bytes()
            anchor = HERE/'caller-root.rs.in'
            require(sha(anchor) == profile['append_only']['sha256'], 'actual capture root differs')
            raw = artifacts/'actual-record.llbc'
            argv = [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default']
            for flag, key in (('--include', 'include'), ('--opaque', 'opaque'),
                              ('--exclude', 'exclude'), ('--start-from', 'roots')):
                for value in profile[key]:
                    argv += [flag, value]
            try:
                root.write_bytes(original+anchor.read_bytes())
                command('charon', [*argv, '--dest-file', raw, '--', '--manifest-path',
                    'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']])
            finally:
                root.write_bytes(original)
            require(raw.stat().st_size <= 64*2**20, 'original capture size guard')
            ir = json.loads(raw.read_text())
            options = ir['translated']['options']
            require(not ir['has_errors'] and options['start_from'] == profile['roots']
                    and all(options[key] == profile[key] for key in ('include','opaque','exclude'))
                    and not any(options[key] for key in ('skip_borrowck','no_typecheck','monomorphize')),
                    'actual scope, typing or borrow mode changed')
            generated = artifacts/'generated'
            generated.mkdir()
            command('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                '-split-files', '-emit-json', '-namespace', 'SandproofActualRecordRust',
                '-subdir', 'SandproofActualRecord', '-dest', generated, raw])
            for name in ('Types.lean', 'Funs.lean'):
                actual = generated/'SandproofActualRecord'/name
                require(sha(actual) == all_files[str(CALLER/name)],
                        'current actual generated file differs: '+name)
            seed_paths = [data['modules'][name]['path'] for name in data['roots']]
            require(batch.closure(project, seed_paths, active_view='lakefile.toml') == all_files,
                    'actual current import closure differs')
            command('build', ['lake', 'build', *data['roots']])
            files = {name: all_files[name] for name in data['checked_files']}
            for index, name in enumerate(files):
                command('kernel-'+str(index), ['lake', 'env', 'lean', name])
            services.ROOT, services.FILES = project, tuple(files)
            service_stdout = artifacts/'actual-services.stdout'
            with service_stdout.open('x') as out, contextlib.redirect_stdout(out):
                services.main()
            physical, = artifacts.glob('physical-service-*')
            physical = physical.rename(artifacts/'physical-files')
            audits = [audit for name in files for audit in re.findall(
                r'^#audit_controller_proof (\S+)$', (project/name).read_text(), re.M)]
            actual = dict(files=files, audits=audits, service_log_sha256=sha(physical/'service.log'))
            registry._saved_physical_view(actual, project, physical, result['commands'])
            current, _ = archive.committed_inputs(ROOT)
            require(current == {key: source[key] for key in ('schema','commit','tree','files')}
                    and all(sha(project/name) == digest for name,digest in all_files.items())
                    and all(sha(path) == digest for path,digest in tools.items())
                    and root.read_bytes() == original
                    and all(sha(Path(row['log'])) == row['log_sha256'] for row in result['commands']),
                    'source, tool or proof evidence changed during checking')
            print(service_stdout.read_text(), end='', flush=True)
            result.update(complete=True, cases={'actual': dict(complete=True, **actual)},
                original_source_restored=True, source_head=source['commit'], source_tree=source['tree'],
                source_archive_sha256=source['archive_sha256'], native_key=native['key'],
                checked_project=str(project), llbc_sha256=sha(raw), all_files=all_files,
                current_caller_generated_matched=True)
    except Exception as error:
        result['error'] = repr(error)
        raise
    finally:
        result['seconds'] = time.monotonic()-started
        write_json(output/'result.json', result)
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--lock-fd', type=int)
    parser.add_argument('--scope', choices=(SCOPE, 'stage1-protocol'), default=SCOPE)
    args = parser.parse_args()
    run(args.output.resolve(), args.lock.resolve(), args.lock_fd, args.scope)
