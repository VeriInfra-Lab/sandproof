"""Re-extract the real Stop dispatch/receipt route and audit its entire finite closure.

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

HERE = Path('verification/experiments/pause-receipt')


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


native = load('pause_receipt_same_native_mutator', Path(__file__).with_name('native.py'))
checks = load('pause_receipt_formal_inventory', Path(__file__).with_name('inventory.py'))
support = load('pause_receipt_archived_projects', Path(__file__).with_name('support.py'))


def run(args):
    os.umask(0o077)
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    started = time.monotonic()
    record = dict(scope='pause-receipt', head=args.head,
                  archive_sha256=args.archive_sha256, complete=False,
                  commands=[], variants=[], whole_V4=False, whole_system=False,
                  legacy_four_field_migration=False, native_tests_run=False, VM_run=False)

    def save():
        record['seconds'] = time.monotonic()-started
        (out/'result.json').write_text(json.dumps(record, indent=2)+'\n')

    try:
        if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
            raise ValueError('Pause receipt replay requires the explicit formal host role')
        if args.archive.is_symlink() or native.sha(args.archive) != args.archive_sha256:
            raise ValueError('Original source archive changed or is linked')
        source = args.correct_project.absolute() if args.correct_project else out/'source'
        if args.correct_project and (source==out or source.is_relative_to(out) or out.is_relative_to(source)):
            raise ValueError('Receipt output and retained correct project must be separate roots')
        inputs, contents, modes = {}, {}, {}
        with tarfile.open(args.archive) as archive:
            for member in archive.getmembers():
                path = Path(member.name)
                if (path.is_absolute() or '..' in path.parts or path.as_posix() != member.name
                        or '\\' in member.name or member.name in inputs or not member.isfile()
                        or member.mode not in (0o644, 0o755)):
                    raise ValueError('Source archive must contain unique regular committed files')
                data=archive.extractfile(member).read()
                inputs[member.name]=dict(sha256=hashlib.sha256(data).hexdigest(),executable=bool(member.mode&0o111))
                contents[member.name]=data
                modes[member.name]=member.mode
        record['prepared_projects']={'correct':support.prepare_project(source,contents,modes,
            existing=args.correct_project is not None,history=out/'previous-correct-inputs')}
        # The old math scratch installed both negative draft pages in C.
        # Preserve only these explicitly known, unarchived drafts outside the
        # positive view; never clean caches or discover a smaller proof scope.
        for runtime in checks.WRONG_MODULES:
            path=source/runtime
            if path.exists() and runtime not in contents:
                if path.is_symlink() or not path.is_file():
                    raise ValueError('Unexpected nonregular old negative draft')
                saved=out/'previous-correct-inputs'/runtime
                saved.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
                shutil.move(path,saved)
        for name in ('replay.py', 'native.py', 'inventory.py', 'support.py'):
            if native.sha(source/HERE/name) != native.sha(Path(__file__).with_name(name)):
                raise ValueError('Executed pause receipt driver differs from archived source: '+name)
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
        support.link_seed(source/'.tools',tools)
        (source/'.lake').mkdir(exist_ok=True)
        support.link_seed(source/'.lake/packages',packages)
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0',LEAN_NUM_THREADS='4',
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
            record['capture_reuse']={'correct':support.admit_capture(args.correct_capture,correct,source,role='correct',roots=checks.ROOTS,native=native,checks=checks)}
            record['real_callers'] = checks.verify_generation(correct, 'CaptureCaller',
                'CaptureCallerRust.sandbox_controller_core', externals)
            compare(correct, 'CaptureCaller', checks.LIBRARIES[0][1])
            record['correct_outputs'] = outputs(correct)
            if len(native.CASES) != 1 or native.CASES[0][0] != 'skip-succeeded-evidence':
                raise ValueError('Formal replay must use the single original first-Succeeded evidence mutation')
            wrong=args.wrong_project.absolute() if args.wrong_project else out/'wrong-source-outcome'
            if (wrong==source or wrong.is_relative_to(source)or source.is_relative_to(wrong)
                    or out.is_relative_to(wrong)or wrong.is_relative_to(out)and args.wrong_project is not None):
                raise ValueError('Receipt source views must have separate operator-owned roots')
            wrong_contents=dict(contents)
            private_generated={}
            for relative in ('CaptureCaller/Types.lean','CaptureCaller/Funs.lean','translation.json'):
                actual=checks.CORRECT_LIBRARY+'/'+relative
                canonical=checks.WRONG_LIBRARY+'/'+relative
                wrong_contents[actual]=contents[canonical]
                private_generated[actual]=hashlib.sha256(contents[canonical]).hexdigest()
            record['prepared_projects']['wrong']=support.prepare_project(wrong,wrong_contents,modes,
                existing=args.wrong_project is not None,history=out/'previous-wrong-inputs')
            mutated=native.mutate(wrong,native.CASES[0])
            destination=out/'wrong-outcome'
            record['capture_reuse']['wrong']=support.admit_capture(args.wrong_capture,destination,wrong,role='wrong',roots=checks.ROOTS,native=native,checks=checks)
            selected = checks.verify_generation(destination, 'CaptureCaller',
                'CaptureCallerRust.sandbox_controller_core', externals)
            compare(destination, 'CaptureCaller', checks.WRONG_LIBRARY)
            correct_external = source/checks.LIBRARIES[0][1]/'CaptureCaller/FunsExternal.lean'
            wrong_external = source/checks.WRONG_LIBRARY/'CaptureCaller/FunsExternal.lean'
            if checks.regular(wrong_external).read_bytes()!=checks.regular(correct_external).read_bytes():
                raise ValueError('Wrong receipt must use the identical original eleven library models')
            record['variants'].append(dict(role='skip-succeeded-evidence', source_sha256=mutated,
                actual_functions=selected, outputs=outputs(destination)))
            # Same Rust type identity in separate projects; no World conversion.
            for runtime, template in checks.WRONG_MODULES.items():
                target=wrong/runtime
                if target.is_symlink() or target.exists()and not target.is_file():
                    raise ValueError('Wrong runtime proof must be a regular private file')
                expected=checks.regular(source/template).read_bytes()
                if target.exists()and target.read_bytes()!=expected:
                    saved=out/'previous-private-proofs'/runtime
                    saved.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
                    saved.write_bytes(target.read_bytes())
                target.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
                target.write_bytes(expected)
            registrations=tomllib.loads((wrong/'lakefile.toml').read_text()).get('lean_lib',[])
            old=[entry for entry in registrations if entry.get('name')=='CaptureCaller']
            if old!=[dict(name='CaptureCaller',srcDir=checks.CORRECT_LIBRARY)]:
                raise ValueError('Original correct CaptureCaller library must be unambiguous')
            record['wrong_private_generated_overrides']=private_generated
            support.link_seed(wrong/'.tools',tools)
            (wrong/'.lake').mkdir(exist_ok=True)
            support.link_seed(wrong/'.lake/packages',packages)
            shared=[checks.BASE/(name+'.lean')for name in ('Conditions','Model')]
            if any(native.sha(source/name)!=native.sha(wrong/name)for name in shared):
                raise ValueError('Correct and wrong receipt must use the identical property and World model')
            record['same_property_model']={str(name):native.sha(source/name)for name in shared}
            sys.path.insert(0,str(source))
            from tools.proof_batch import closure
            views=[('correct',source,files,checks.AUDIT_MODULE),
                   ('wrong',wrong,checks.wrong_inventory(source,manifest),checks.WRONG_AUDIT_MODULE)]
            record['views']={}
            for role,project,view_files,audit_module in views:
                dependencies=closure(project,seeds=list(view_files),active_view='lakefile.toml')
                if role=='wrong' and any(str(checks.BASE/(leaf+'.lean'))in dependencies for leaf in ('Admission','Acceptance','Audit')):
                    raise ValueError('Wrong view must not import correct receipt admission/safety conclusions')
                for name in dependencies:
                    if name not in view_files:
                        view_files[name]=checks.audits(checks.regular(project/name).read_text())
                snapshots={name:native.sha(project/name)for name in view_files}
                command('build-'+role+'-receipt',['lake','build','CaptureCaller.Funs',audit_module],
                        root=project,forward=True)
                if role=='wrong':
                    command('kernel-wrong-receipt',['lake','env','lean',str(checks.BASE/'WrongAudit.lean')],
                            root=project,forward=True)
                requests=[dict(tool='lean_diagnostic_messages',arguments=dict(file_path=name,timeout_s=120),
                    expected_sha256=native.sha(project/name))for name in view_files]+[dict(tool='exit')]
                data=''.join(json.dumps(row)+'\n'for row in requests).encode()
                (out/(role+'-requests.jsonl')).write_bytes(data)
                command(role+'-service',[project/'.tools/lean-mcp/bin/python','tools/lean_session.py'],root=project,data=data)
                actual=checks.check_service(project,view_files,(out/(role+'-service.log')).read_text(),native.sha)
                if any(native.sha(project/name)!=expected for name,expected in snapshots.items()):
                    raise ValueError('Receipt proof input changed during checking: '+role)
                record['views'][role]=dict(complete=True,files=snapshots,local_import_closure=dependencies,
                    expected_audits=view_files,actual_results=actual,checked_file_count=len(view_files),
                    audit_command_count=sum(map(len,view_files.values())))
                save()
            for name,expected in inputs.items():
                for project,overrides in ((source,{}),(wrong,{
                    str(native.SOURCE):mutated,**private_generated})):
                    path=project/name
                    info=path.lstat()
                    if (not stat.S_ISREG(info.st_mode)or native.sha(path)!=overrides.get(name,expected['sha256'])
                            or bool(info.st_mode&0o111)!=expected['executable']):
                        raise ValueError('Archived receipt input changed: '+str(path))
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
    parser.add_argument('--correct-project',type=Path)
    parser.add_argument('--wrong-project',type=Path)
    parser.add_argument('--correct-capture',type=Path,required=True)
    parser.add_argument('--wrong-capture',type=Path,required=True)
    parser.add_argument('--archive-sha256', required=True)
    parser.add_argument('--head', required=True)
    result = run(parser.parse_args())
    print(json.dumps({key: result.get(key) for key in ('scope', 'head', 'complete', 'seconds', 'error')}), flush=True)
    raise SystemExit(0 if result['complete'] else 1)


if __name__ == '__main__':
    main()
