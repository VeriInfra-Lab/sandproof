"""Bounded development recapture; does not issue native or public acceptance."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tarfile
import time

import prepare_transaction_capture
import revision_capture
import world_begin_capture

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
CURRENT = {
    'prepare-transaction': (prepare_transaction_capture, 'prepare_admission'),
    'world-begin': (world_begin_capture, 'begin_admission'),
}
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--mode", choices=("source", "parent", "base-revision", *CURRENT), required=True)
parser.add_argument("--lock", type=Path, required=True)
parser.add_argument("--source-commit", help="exact Git commit for a current source capture")
args = parser.parse_args()
if args.source_commit is not None and (
        args.mode == 'source' or re.fullmatch(r'[0-9a-f]{40}', args.source_commit) is None):
    parser.error('--source-commit requires a full commit and a current source capture')
if (args.mode == 'base-revision' or args.mode in CURRENT) and args.source_commit is None:
    parser.error(args.mode + ' requires --source-commit')
OUT = args.output.resolve()
MODE = args.mode
TOOLS = (ROOT / '.tools/aeneas').resolve(strict=True)
LOCK = args.lock.resolve(strict=True)
GIB = 2**30


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_source_inputs(root, inputs):
    for name, expected in inputs.items():
        path = root / name
        if path.is_symlink() or sha(path) != expected:
            raise ValueError(f'reviewed current source differs: {name}')


def main():
    os.umask(0o077)
    OUT.mkdir()
    source = OUT / 'source'
    source.mkdir()
    result = dict(development_only=True, native_acceptance=False, mode=MODE, commands=[], complete=False)
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
               CARGO_TARGET_DIR=str(OUT / 'target'))
    for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(key, None)
    try:
        result['head'] = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        subprocess.run(['git', 'diff', '--quiet', 'HEAD'], cwd=ROOT, check=True)
        admission, admission_key = CURRENT.get(MODE, (None, 'revision_admission'))
        inputs = admission.SOURCE_INPUTS if admission else {
            revision_capture.REVISION_SOURCE: revision_capture.REVISION_SOURCE_SHA256,
        }
        if args.source_commit is not None:
            if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
                raise ValueError('current source capture belongs to the existing proof role')
            if subprocess.check_output(['git', 'status', '--porcelain', '-z'], cwd=ROOT):
                raise ValueError('current source capture driver must be clean')
            result['driver_head'] = result['head']
            result['head'] = subprocess.check_output(
                ['git', 'rev-parse', '--verify', args.source_commit + '^{commit}'],
                cwd=ROOT, text=True,
            ).strip()
            if result['head'] != args.source_commit:
                raise ValueError('current revision source is not the exact requested commit')
            result['tree'] = subprocess.check_output(
                ['git', 'rev-parse', result['head'] + '^{tree}'], cwd=ROOT, text=True,
            ).strip()
            if admission and (result['head'], result['tree']) != (
                    admission.SOURCE_COMMIT, admission.SOURCE_TREE):
                raise ValueError(f'{MODE} requires the reviewed integration commit and tree')
        result['workspace_free_bytes'] = shutil.disk_usage(ROOT).free
        if result['workspace_free_bytes'] < 80*GIB:
            raise ValueError('at least 80 GiB workspace free required')
        cpus = sorted(os.sched_getaffinity(0))[:4]
        os.sched_setaffinity(0, cpus)
        result['cpus'] = cpus
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        charon, aeneas = TOOLS/'charon/bin/charon', TOOLS/'bin/aeneas'
        if sha(charon) != fixed['charon']['binary_sha256'] or sha(aeneas) != fixed['aeneas']['binary_sha256']:
            raise ValueError('fixed Charon or Aeneas binary differs')
        archive = OUT/'source.tar'
        with archive.open('xb') as output:
            subprocess.run(['git','archive',result['head']], cwd=ROOT, stdout=output, check=True)
        with tarfile.open(archive) as tar:
            tar.extractall(source, filter='data')
        if args.source_commit is not None:
            result['source_archive_sha256'] = sha(archive)
            check_source_inputs(source, inputs)
        history = ROOT/'verification/experiments/stage1-protocol/original-source-rows/original-capture'
        if MODE == 'source':
            cfg = json.loads((history/'precheck-config.json').read_text())
            meta = json.loads((history/'name-metadata-plan.json').read_text())
            lib = source/meta['source']
            original = lib.read_bytes()
            assert sha(lib) == meta['original_sha256']
            lib.write_bytes((history/'metadata-view-lib.rs').read_bytes())
            assert sha(lib) == meta['view_sha256']
        else:
            cfg = json.loads((ROOT/'verification/experiments/stage1-protocol/unload-startup-source/capture-profile.json').read_text())['parent']
            cfg.update(namespace='SandproofUnloadFirstObservation', subdir='SandproofUnloadFirstObservation')
            cfg = admission.configuration(cfg) if admission else revision_capture.configuration(cfg, MODE)

        def run(label, argv, seconds):
            started = time.monotonic()
            row = dict(label=label, argv=list(map(str,argv)))
            result['commands'].append(row)
            with (OUT/(label+'.log')).open('xb') as output:
                child = subprocess.Popen(argv, cwd=source, env=env, stdout=output,
                                         stderr=subprocess.STDOUT, start_new_session=True)
                while child.poll() is None:
                    size = sum(p.lstat().st_blocks*512 for p in (OUT/'target').rglob('*') if not p.is_symlink()) if (OUT/'target').exists() else 0
                    result['peak_target_bytes'] = max(size, result.get('peak_target_bytes',0))
                    if time.monotonic()-started > seconds or size > 4*GIB or shutil.disk_usage(ROOT).free < 64*GIB:
                        os.killpg(child.pid,signal.SIGTERM)
                        try:
                            child.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            os.killpg(child.pid,signal.SIGKILL)
                            child.wait(timeout=5)
                        row['bounded_stop']=True
                        break
                    time.sleep(.5)
            row.update(exit_code=child.returncode,seconds=time.monotonic()-started,
                       log_sha256=sha(OUT/(label+'.log')))
            assert child.returncode == 0 and not row.get('bounded_stop'), label

        with LOCK.open('r+') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            selectors = [item for flag, key in [
                ('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                ('--include', 'include'), ('--opaque', 'opaque'),
            ] for value in cfg.get(key, ()) for item in (flag, value)]
            ir=OUT/'actual.llbc'
            run('charon',[charon,'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default',*selectors,
                '--dest-file',ir,'--','--manifest-path','controller-service/Cargo.toml','--lib','--locked','--offline',
                '--target',fixed['target']],240)
            if MODE=='source':
                assert lib.read_bytes()==(history/'metadata-view-lib.rs').read_bytes()
                lib.write_bytes(original)
            captured_ir_sha256 = sha(ir)
            run('aeneas',[aeneas,'-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json',
                '-namespace',cfg['namespace'],'-subdir',cfg['subdir'],'-dest',OUT/'generated',ir],120)
            if sha(ir) != captured_ir_sha256:
                raise ValueError('actual Charon IR changed during generation')
        if args.source_commit is not None:
            if admission:
                result[admission_key] = admission.admit(ir, OUT / 'generated', cfg, fixed['target'])
            else:
                result[admission_key] = revision_capture.admit(ir, OUT / 'generated', cfg, fixed['target'], MODE)
            check_source_inputs(source, inputs)
            if sha(archive) != result['source_archive_sha256']:
                raise ValueError('current source archive changed during capture')
        result.update(complete=True,ir_sha256=sha(ir),generated={str(p.relative_to(OUT/'generated')):sha(p)
            for p in (OUT/'generated').rglob('*') if p.is_file()})
    except BaseException as error:
        result['error']=repr(error)
    finally:
        (OUT/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k not in ('commands','generated')},indent=2))
    return not result['complete']


if __name__=='__main__':
    raise SystemExit(main())
