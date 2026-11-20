"""Recheck the current original-source-rows slice inside the existing formal suite."""
import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import resource
import shutil
import signal
import subprocess
import sys
import tarfile
import time

HERE = Path(__file__).resolve().parent
STAGE1 = HERE.parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import digest, native_case, sha, validate_native, write_json
from tools.proof_batch import closure

SCOPE = 'stage1-v3-transaction-source'
LABEL = 'stage1-original-source-rows'
MODULE = 'SandproofOriginalSourceRows'
PROOF_ROOTS = ('SourceRowEvidence', 'SourceCompleteEvidence', 'SourceObservedRows')
GIB = 2**30


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(path.read_text())


def rel(name):
    path = PurePosixPath(name)
    require(bool(path.parts) and not path.is_absolute() and '..' not in path.parts
            and name == path.as_posix() and '\\' not in name, 'unsafe input path: '+name)
    return Path(*path.parts)


def checked(path, expected):
    require(path.is_file() and not path.is_symlink() and sha(path) == expected,
            'missing or changed finite input: '+str(path))
    return path


def materialize(source, destination):
    if destination.exists():
        checked(destination, sha(source))
    else:
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)


def run(output, scope, lock_path, lock_fd):
    require(__debug__ and os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') == 'formal',
            'component needs the original formal role and active Python assertions')
    from importlib.util import module_from_spec, spec_from_file_location

    def load(name, path):
        spec = spec_from_file_location(name, path)
        module = module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    registered = load('rows_registered', STAGE1/'registered.py')
    checker = load('rows_actual_files', STAGE1/'formal.py')
    archive = load('rows_committed_archive', STAGE1/'archive.py')
    inherited_fd, lock = registered.inherited_lock()
    require(lock_path is None or lock_path.resolve(strict=True) == Path(lock['path']).resolve(strict=True),
            'requested lock differs from the inherited coordination lock')
    require(lock_fd is None or lock_fd == inherited_fd, 'requested lock descriptor differs')
    cpus = sorted(os.sched_getaffinity(0))
    require(len(cpus) == 4, 'original source-row job requires its four admitted CPUs')
    output.mkdir(parents=True, mode=0o700)
    project = output/'actual'/'source'
    project.mkdir(parents=True, mode=0o700)
    target = Path(os.environ['CARGO_TARGET_DIR']).resolve(strict=True)
    require(target.is_dir() and not any(target.iterdir())
            and target.stat().st_dev == output.stat().st_dev,
            'source-row job requires its fresh target on the admitted filesystem')
    limit = resource.getrlimit(resource.RLIMIT_AS)
    cap = min([256*GIB, *(x for x in limit if x != resource.RLIM_INFINITY)])
    resource.setrlimit(resource.RLIMIT_AS, (cap, cap))
    begin = time.monotonic()
    result = dict(kind='original-source-rows-current-formal-component', complete=False,
                  public_scope=scope, commands=[], cpus=cpus, coordination_lock=lock,
                  peak_target_bytes=0)
    capturing = False
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS='4',
               CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0',
               RUSTFLAGS='--remap-path-prefix='+str(project)+'=.',
               LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536', PYTHONDONTWRITEBYTECODE='1')
    # The existing physical-file helper inherits the process environment.
    os.environ.update(LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536',
                      PYTHONDONTWRITEBYTECODE='1')

    def guard(start=False):
        fs = os.statvfs(output)
        require(fs.f_bavail*fs.f_frsize >= (80 if start else 64)*GIB, 'disk headroom')
        require(time.monotonic()-begin < (180 if capturing else 600), 'elapsed budget')
        if capturing:
            allocated = int(subprocess.check_output(['du', '-x', '-s', '-B1', str(target)],
                                                    text=True).split()[0])
            result['peak_target_bytes'] = max(result['peak_target_bytes'], allocated)
            require(allocated <= 4*GIB, 'capture target exceeds 4 GiB')

    def command(cwd, folder, label, argv):
        guard()
        result['phase'] = label
        argv = list(map(str, argv))
        log = folder/(label+'.log')
        started = time.monotonic()
        with log.open('xb') as stream:
            child = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                     stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                while child.poll() is None:
                    guard()
                    if capturing:
                        observed = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', log.read_text(errors='replace'))
                        require('error: Type error after transformations:' not in observed
                                and not (label == 'aeneas' and re.search(r'\[Error\s*\]', observed)),
                                'first conclusive original capture diagnostic: '+label)
                    time.sleep(0.25 if capturing else 1)
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait()
                result['commands'].append(dict(label=label, argv=argv, cwd=str(cwd),
                    exit_code=child.returncode, seconds=time.monotonic()-started,
                    log_sha256=sha(log)))
        guard()
        require(child.returncode == 0, 'original command failed: '+label)

    try:
        guard(start=True)
        spec = read(HERE/'execution-inputs.json')
        require(spec['schema'] == 1, 'unknown source-row execution inventory')
        history = HERE/'original-capture'
        for name, expected in spec['capture_inputs'].items():
            checked(history/rel(name), expected)
        cfg = read(history/'precheck-config.json')
        require(len(cfg['roots']) == 2 and len(cfg['include']) == 48 and len(cfg['opaque']) == 4,
                'source-row selection differs')
        provider_root = Path(os.environ['SANDPROOF_SOURCE_ROWS_PROVIDER_ROOT']).resolve(strict=True)
        provider_history = provider_root/'std-mutex-persistent-tmp'
        provider_proof = provider_history/'v3-source-provider-kernel-20'
        provider_receipt = read(history/'proof-link-binding.json')['independent_bridge_evidence']
        checked(provider_proof/'result.json', provider_receipt['result_sha256'])
        checked(provider_proof/'actual-files/physical-files/service.log', provider_receipt['service_log_sha256'])
        require((provider_history/'v3-original-source-driver-cut-precheck-16/original_source_generic.llbc').is_file()
                and (provider_history/'v3-source-provider-kernel-18/source').is_dir(),
                'missing original identifier provider reference files')

        evidence = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']).resolve(strict=True)
        validate_native(evidence, scope=scope)
        native_case('functional', ROOT, None, scope=scope)
        receipt = native_case(LABEL, ROOT, None, scope=scope)
        native = evidence/'commands'/digest(receipt['key'])/'evidence'
        manifest = native/'source-manifest.json'
        source = archive.verify(native/'source.tar', manifest)
        current, _ = archive.committed_inputs(ROOT)
        identity = {key: source[key] for key in ('schema', 'commit', 'tree', 'files')}
        require(current == identity, 'native original archive differs from current committed candidate')
        native_result = read(native/'result.json')
        require(native_result['complete'] is True and native_result['passed_cases'] == 9
                and native_result['source_head'] == source['commit']
                and native_result['source_tree'] == source['tree']
                and native_result['source_archive_sha256'] == source['archive_sha256'],
                'current source-row native case did not finish on its exact original')
        result.update(source_head=source['commit'], source_tree=source['tree'],
                      native_receipt_sha256=sha(native.parent/'receipt.json'),
                      native_source_archive_sha256=source['archive_sha256'])
        with tarfile.open(native/'source.tar') as tar:
            tar.extractall(project, filter='data')
        artifacts = project/'artifacts'
        artifacts.mkdir()
        shutil.copyfile(manifest, artifacts/'native-source-manifest.json')
        source_cut = cfg['trusted_owned_mechanical_driver']
        checked(project/rel(source_cut['source_file']), source_cut['whole_file_sha256'])

        toolroot = Path(os.environ['SANDPROOF_PROOF_TOOL_ROOT']).resolve(strict=True)
        fixed = read(project/'verification/foundations-environment.json')
        charon, aeneas = toolroot/'aeneas/charon/bin/charon', toolroot/'aeneas/bin/aeneas'
        checked(charon.resolve(strict=True), fixed['charon']['binary_sha256'])
        checked(aeneas.resolve(strict=True), fixed['aeneas']['binary_sha256'])
        meta = read(history/'name-metadata-plan.json')
        lib = checked(project/rel(meta['source']), meta['original_sha256'])
        original = lib.read_bytes()
        view = checked(history/'metadata-view-lib.rs', meta['view_sha256']).read_bytes()
        registration = b'#![feature(register_tool)]\n#![register_tool(charon)]\n'
        require(view.startswith(registration) and len(meta['renames']) == 2, 'name metadata changed')
        restored = view.removeprefix(registration)
        for rename in meta['renames']:
            header = rename['source_header'].encode()
            tag = ('#[charon::rename("'+rename['name']+'")]\n').encode()
            require(restored.count(tag+header) == 1, 'ambiguous original From metadata')
            restored = restored.replace(tag+header, header, 1)
        require(restored == original, 'extraction metadata changed Rust source bytes')

        llbc = artifacts/'original_source_generic.llbc'
        generated = artifacts/'generated'
        selectors = [item for flag, key in (('--start-from', 'roots'), ('--include', 'include'),
                     ('--opaque', 'opaque')) for value in cfg[key] for item in (flag, value)]
        capturing = True
        lib.write_bytes(view)
        try:
            command(project, output, 'charon', [charon, 'cargo', '--preset=aeneas',
                '--error-on-warnings', '--sysroot', 'default', *selectors, '--dest-file', llbc,
                '--', '--manifest-path', 'controller-service/Cargo.toml', '--lib', '--locked',
                '--offline', '--target', 'x86_64-unknown-linux-gnu'])
        finally:
            require(lib.read_bytes() == view, 'private metadata view changed during extraction')
            lib.write_bytes(original)
        checked(lib, meta['original_sha256'])
        raw = read(llbc)
        admitted = load('rows_ir_admission', history/'admit-original-source.py').audit(raw, cfg, artifacts)
        load('rows_actual_source', history/'actual-source-audit.py').check(raw, cfg, artifacts)
        load('rows_existing_provider', history/'provider-link-audit.py').check(
            raw, project, history, artifacts, provider_root)
        command(project, output, 'aeneas', [aeneas, '-backend', 'lean', '-abort-on-error',
            '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', cfg['namespace'],
            '-subdir', cfg['subdir'], '-dest', generated, llbc])
        translation = read(generated/'translation.json')['functions']
        for name in cfg['roots']:
            rows = [row for row in translation if row['rust_name'] == name]
            require(len(rows) == 1 and rows[0]['is_local'] is True and rows[0]['is_opaque'] is False,
                    'actual translated root missing: '+name)
        owned = ('sandproof_controller_service::', 'sandbox_controller_core::', 'sandproof_protocol::')
        require({row['rust_name'] for row in translation
                 if row['is_opaque'] and row['rust_name'].startswith(owned)}
                == set(admitted['planned_owned_library_opaque']), 'owned translation cut set differs')
        result.update(llbc_sha256=sha(llbc), generated={str(p.relative_to(generated)): sha(p)
                      for p in sorted(generated.rglob('*')) if p.is_file()})
        for name, expected in spec['external_templates'].items():
            checked(generated/MODULE/rel(name), expected)
        capturing = False

        before = {name: row['sha256'] for name, row in source['files'].items()}
        for name, row in spec['materialize'].items():
            materialize(checked(project/rel(row['source']), row['sha256']), project/rel(name))
        for name in ('Types.lean', 'Funs.lean'):
            materialize(generated/MODULE/name, project/rel(spec['generated_directory'])/name)
        lake = checked(project/rel(spec['lake_view']['source']), spec['lake_view']['sha256'])
        shutil.copyfile(lake, project/'lakefile.toml')
        before['lakefile.toml'] = sha(project/'lakefile.toml')
        (project/'.tools').symlink_to(toolroot, target_is_directory=True)
        (project/'.lake').mkdir()
        packages = Path(os.environ['SANDPROOF_FORMAL_PACKAGES']).resolve(strict=True)
        (project/'.lake/packages').symlink_to(packages, target_is_directory=True)

        seeds = [name+'.lean' for name in PROOF_ROOTS]
        files = closure(project, seeds, active_view='lakefile.toml')
        require(set(files) == set(spec['closure_files']), 'current source-row import inventory differs')
        result['complete_local_closure'] = files
        command(project, output, 'build-roots', ['lake', 'build', *PROOF_ROOTS])
        physical = output/'actual'/'actual-files'
        physical.mkdir()
        actual = checker.actual_files(project, sorted(files), physical, env, command)
        registered._saved_physical_view(actual, project, physical/'physical-files', result['commands'])
        require(sorted(actual['audits']) == sorted(spec['audits']), 'source-row audit inventory differs')
        require(files == closure(project, seeds, active_view='lakefile.toml')
                and archive.committed_inputs(ROOT)[0] == identity, 'current source or proof closure changed')
        for name, expected in before.items():
            checked(project/rel(name), expected)
        guard()
        result.update(complete=True, cases={'actual': dict(actual, complete=True)})
    except BaseException as error:
        result['error'] = repr(error)
        raise
    finally:
        result['seconds'] = time.monotonic()-begin
        write_json(output/'result.json', result)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--lock', type=Path)
    parser.add_argument('--lock-fd', type=int)
    parser.add_argument('--scope', choices=(SCOPE, 'stage1-protocol'), default=SCOPE)
    args = parser.parse_args()
    os.umask(0o077)
    run(args.output.resolve(), args.scope, args.lock, args.lock_fd)
