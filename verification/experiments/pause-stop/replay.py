"""Re-extract the real Commit/Stop worker route and audit its entire finite closure.

Run only through the approved formal host wrapper. The caller supplies a
private component lock; its outer executor already holds formal-heavy.lock.
"""
import argparse
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import time
import tomllib
import traceback

HERE = Path('verification/experiments/pause-stop')


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


native = load('pause_stop_same_native_mutator', Path(__file__).with_name('native.py'))
checks = load('pause_stop_formal_inventory', Path(__file__).with_name('inventory.py'))


def run(args):
    os.umask(0o077)
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    started = time.monotonic()
    record = dict(scope='pause-stop', head=args.head,
                  archive_sha256=args.archive_sha256, complete=False,
                  commands=[], variants=[], whole_V4=False, whole_system=False,
                  legacy_four_field_migration=False, native_tests_run=False, VM_run=False)

    def save():
        record['seconds'] = time.monotonic()-started
        (out/'result.json').write_text(json.dumps(record, indent=2)+'\n')

    try:
        if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
            raise ValueError('Pause Stop replay requires the explicit formal host role')
        if args.archive.is_symlink() or native.sha(args.archive) != args.archive_sha256:
            raise ValueError('Original source archive changed or is linked')
        source = out/'source'
        source.mkdir(mode=0o700)
        inputs = {}
        with tarfile.open(args.archive) as archive:
            for member in archive.getmembers():
                path = Path(member.name)
                if (path.is_absolute() or '..' in path.parts or path.as_posix() != member.name
                        or '\\' in member.name or member.name in inputs or not member.isfile()
                        or member.mode not in (0o644, 0o755)):
                    raise ValueError('Source archive must contain unique regular committed files')
                inputs[member.name] = dict(sha256=hashlib.sha256(archive.extractfile(member).read()).hexdigest(),
                                           executable=bool(member.mode & 0o111))
            archive.extractall(source, filter='data')
        for name in ('replay.py', 'native.py', 'inventory.py'):
            if native.sha(source/HERE/name) != native.sha(Path(__file__).with_name(name)):
                raise ValueError('Executed pause Stop driver differs from archived source: '+name)
        files, manifest = checks.inventory(source)
        record.update(inventory=manifest, inventory_sha256=native.sha(source/HERE/'inventory.json'))
        tools = args.tools.resolve(strict=True)
        packages = args.packages.resolve(strict=True)
        if not tools.is_dir() or not packages.is_dir():
            raise ValueError('Original fixed tools and package directories are required')
        fixed = checks.literal_constant(source/'verification/experiments/capture-handoff/replay.py', 'TOOLS')
        externals = checks.literal_constant(source/'verification/experiments/capture-handoff/caller_replay.py', 'EXTERNALS')
        if not isinstance(fixed, dict) or not fixed or not isinstance(externals, set) or not externals:
            raise ValueError('Original tool or external inventory changed format')
        for path, expected in fixed.items():
            if native.sha(tools/path) != expected:
                raise ValueError('Fixed proof tool changed: '+path)
        record.update(tools=fixed, reviewed_externals=sorted(externals))
        (source/'.tools').symlink_to(tools, target_is_directory=True)
        (source/'.lake').mkdir()
        (source/'.lake/packages').symlink_to(packages, target_is_directory=True)
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(out/'formal-target'))

        def command(label, argv, root=source, current_env=env, forward=False, data=None):
            argv = list(map(str, argv))
            log = out/(label+'.log')
            begin = time.monotonic()
            with log.open('xb') as stream:
                standard_input = {'input': data} if data is not None else {'stdin': subprocess.DEVNULL}
                process = subprocess.run(argv, cwd=root, env=current_env, **standard_input,
                                         stdout=stream, stderr=subprocess.STDOUT)
            record['commands'].append(dict(label=label, argv=argv, cwd=str(root),
                exit_code=process.returncode, seconds=time.monotonic()-begin,
                log_sha256=native.sha(log)))
            save()
            if forward:
                sys.stdout.buffer.write(log.read_bytes())
                sys.stdout.buffer.flush()
            if process.returncode:
                raise ValueError('Command failure is not a property rejection: '+label)

        def compare(destination, module, committed):
            for relative in (module+'/Types.lean', module+'/Funs.lean', 'translation.json'):
                if checks.regular(destination/'lean'/relative).read_bytes() != checks.regular(source/committed/relative).read_bytes():
                    raise ValueError('Current generated file differs: '+committed+'/'+relative)

        def outputs(root):
            return {str(path.relative_to(root)): native.sha(checks.regular(path))
                    for path in sorted(root.rglob('*')) if path.is_file() or path.is_symlink()}

        with args.lock.open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            correct = out/'caller'
            command('generate-caller', ['bash', 'verification/experiments/capture-handoff/regenerate.sh', 'caller'],
                    current_env=env | {'SANDPROOF_PROOF_TOOL_ROOT': str(tools),
                                       'SANDPROOF_CAPTURE_OUTPUT': str(correct)})
            record['real_callers'] = checks.verify_generation(correct, 'CaptureCaller',
                'CaptureCallerRust.sandbox_controller_core', externals)
            compare(correct, 'CaptureCaller', checks.LIBRARIES[0][1])
            record['correct_outputs'] = outputs(correct)
            if len(native.CASES) != 1 or native.CASES[0][0] != 'ignore-handoff':
                raise ValueError('Formal replay must use the single original handoff mutation')
            wrong = out/'wrong-source-permit'
            shutil.copytree(source, wrong, ignore=shutil.ignore_patterns('.lake', '.tools', 'target'))
            mutated = native.mutate(wrong, native.CASES[0])
            destination = out/'wrong-permit'
            destination.mkdir(mode=0o700)
            wrong_env = env | {'RUSTFLAGS': '--remap-path-prefix='+str(wrong)+'=.',
                               'CARGO_TARGET_DIR': str(out/'target-wrong-permit')}
            argv = [tools/'aeneas/charon/bin/charon', 'cargo', '--preset=aeneas',
                    '--error-on-warnings', '--sysroot', 'default']
            for name in checks.ROOTS:
                argv += ['--start-from', 'crate::'+name]
            argv += ['--include', 'sandbox_controller_core', '--include', 'core::option::*::and_then',
                     '--dest-file', destination/'capture_handoff.llbc', '--', '--manifest-path',
                     'Cargo.toml', '--lib', '--locked', '--offline']
            command('charon-ignore-handoff', argv, wrong/'controller-core', wrong_env)
            command('aeneas-ignore-handoff', [tools/'aeneas/bin/aeneas', '-backend', 'lean',
                '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json', '-namespace',
                'PauseHandoffWrongRust.sandbox_controller_core', '-subdir', 'PauseHandoffWrong', '-dest',
                destination/'lean', destination/'capture_handoff.llbc'], wrong, wrong_env)
            selected = checks.verify_generation(destination, 'PauseHandoffWrong',
                'PauseHandoffWrongRust.sandbox_controller_core', externals)
            compare(destination, 'PauseHandoffWrong', checks.LIBRARIES[1][1])
            correct_external = source/checks.LIBRARIES[0][1]/'CaptureCaller/FunsExternal.lean'
            wrong_external = source/checks.LIBRARIES[1][1]/'PauseHandoffWrong/FunsExternal.lean'
            rebound = checks.regular(correct_external).read_text().replace(
                'CaptureCaller.Types', 'PauseHandoffWrong.Types').replace(
                'CaptureCallerRust.sandbox_controller_core', 'PauseHandoffWrongRust.sandbox_controller_core')
            if checks.regular(wrong_external).read_text() != rebound:
                raise ValueError('Wrong external models are not the original mechanical rebinding')
            record['variants'].append(dict(role='ignore-handoff', source_sha256=mutated,
                actual_functions=selected, outputs=outputs(destination)))
            config = source/'lakefile.toml'
            text = config.read_text()
            registrations = tomllib.loads(text).get('lean_lib', [])
            for module, path in checks.LIBRARIES:
                existing = [entry for entry in registrations if entry.get('name') == module]
                if existing and existing != [dict(name=module, srcDir=path)]:
                    raise ValueError('Ambiguous pause Stop library registration')
                if not existing:
                    text += '\n[[lean_lib]]\nname = "'+module+'"\nsrcDir = "'+path+'"\n'
            config.write_text(text)  # Only this fresh private archive view.
            record['private_lakefile_sha256'] = native.sha(config)
            sys.path.insert(0, str(source))
            from tools.proof_batch import closure
            dependencies = closure(source, seeds=list(files), active_view='lakefile.toml')
            for name in dependencies:
                if name not in files:
                    files[name] = checks.audits(checks.regular(source/name).read_text())
            record.update(local_import_closure=dependencies, expected_audits=files)
            command('build-pause-stop', ['lake', 'build', 'CaptureCaller.Funs', 'PauseHandoffWrong.Funs',
                checks.AUDIT_MODULE], forward=True)
            command('audit-ignore-handoff', ['lake', 'env', 'lean',
                str(checks.WRONG_FILE)], forward=True)
            requests = [dict(tool='lean_diagnostic_messages', arguments=dict(file_path=name, timeout_s=120),
                expected_sha256=native.sha(source/name)) for name in files]+[dict(tool='exit')]
            data = ''.join(json.dumps(row)+'\n' for row in requests).encode()
            (out/'requests.jsonl').write_bytes(data)
            command('service', [source/'.tools/lean-mcp/bin/python', 'tools/lean_session.py'], data=data)
            record['actual_results'] = checks.check_service(source, files, (out/'service.log').read_text(), native.sha)
            record.update(checked_file_count=len(files), audit_command_count=sum(map(len, files.values())))
            for name, expected in inputs.items():
                for root, overrides in ((source, {'lakefile.toml': record['private_lakefile_sha256']}),
                                        (wrong, {str(native.SOURCE): mutated})):
                    path = root/name
                    info = path.lstat()
                    if (not stat.S_ISREG(info.st_mode) or native.sha(path) != overrides.get(name, expected['sha256'])
                            or bool(info.st_mode & 0o111) != expected['executable']):
                        raise ValueError('Archived formal input changed during checking: '+str(path))
            for path, expected in fixed.items():
                if native.sha(tools/path) != expected:
                    raise ValueError('Fixed proof tool changed during checking: '+path)
            if native.sha(args.archive) != args.archive_sha256:
                raise ValueError('Original source archive changed during checking')
            record['complete'] = True
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    finally:
        save()
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('archive', 'output', 'lock', 'tools', 'packages'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--archive-sha256', required=True)
    parser.add_argument('--head', required=True)
    result = run(parser.parse_args())
    print(json.dumps({key: result.get(key) for key in ('scope', 'head', 'complete', 'seconds', 'error')}), flush=True)
    raise SystemExit(0 if result['complete'] else 1)


if __name__ == '__main__':
    main()
