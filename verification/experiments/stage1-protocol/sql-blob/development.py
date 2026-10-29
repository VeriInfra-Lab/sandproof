"""Check the full-byte equations of original rusqlite slice conversions; no SQL seal."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

import preflight as capture

ROOT, HERE, sha = capture.ROOT, capture.HERE, capture.sha
GEN = Path('verification/aeneas/stage1-protocol/sql-blob/SandproofSqlBlob')
PROOF = Path('SandboxModel/Lifecycle/Protocol/SqlBlobValues.lean')
checker = capture.module('blob_original_checker', ROOT/HERE.parent/'formal.py')
resolver = capture.module('blob_original_closure', ROOT/'tools/proof_batch.py')


def run(output):
    output.mkdir(parents=True, mode=0o700); started = time.monotonic()
    rec = dict(kind='original-sql-blob-values-development', complete=False, commands=[],
        formal_acceptance=False, ning_acceptance=False, shared_acceptance=False, full_v3=False,
        sql_parameter_wiring_proved=False, sql_execution_proved=False, digest_correctness_proved=False,
        current_carrier_use_adopted=False, new_external_function_assumptions=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')

    def command(project, folder, label, argv):
        log = folder/(label+'.log'); begin = time.monotonic()
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec, indent=2)+'\n')
        if result.returncode: raise ValueError('failed '+str(log))

    try:
        captured = capture.run(output/'capture', 'slice')
        rec['capture'] = captured
        if not captured['translation_complete']: raise ValueError('original capture failed')
        project = output/'capture/source'; expected = dict(captured['inputs'])
        generated = output/'capture/generated'
        inv = json.loads((ROOT/HERE/'inventory.json').read_text())
        functions = json.loads((generated/'translation.json').read_text())['functions']
        if sorted((f['lean_name'], f['is_opaque'], f['lean_file']) for f in functions) != [tuple(f) for f in inv['generated_functions']]:
            raise ValueError('generated function closure differs')
        if {str(p.relative_to(generated)):sha(p) for p in generated.rglob('*.lean')} != inv['generated_files']:
            raise ValueError('fresh raw generated type/function files differ')
        (project/GEN).mkdir(parents=True)
        for name in ('Types.lean', 'Funs.lean'):
            shutil.copy2(generated/'SandproofSqlBlob'/name, project/GEN/name)
            expected[str(GEN/name)] = sha(project/GEN/name)
        shutil.copy2(ROOT/HERE/'TypesExternal.lean.in', project/GEN/'TypesExternal.lean')
        expected[str(GEN/'TypesExternal.lean')] = sha(project/GEN/'TypesExternal.lean')
        shutil.copy2(ROOT/HERE/'SqlBlobValues.lean.in', project/PROOF)
        expected[str(PROOF)] = sha(project/PROOF)
        lake = project/'lakefile.toml'
        lake.write_text(lake.read_text()+'\n[[lean_lib]]\nname = "SandproofSqlBlob"\nsrcDir = "verification/aeneas/stage1-protocol/sql-blob"\n')
        expected['lakefile.toml'] = sha(lake)
        files = sorted(resolver.closure(project, [str(PROOF)], active_view='lakefile.toml'))
        audits = [a for n in files for a in re.findall(r'^#audit_controller_proof (\S+)$', (project/n).read_text(), re.M)]
        if files != inv['checks']['files'] or audits != inv['checks']['audits']:
            raise ValueError('actual file or audit closure differs')
        if len(set(audits)) != len(audits): raise ValueError('duplicate audit')
        rec.update(files={n:sha(project/n) for n in files}, audits=audits,
            inputs=captured['inputs'], input_modes=captured['input_modes'], source_commit=captured['source_commit'])
        command(project, output, 'build', ['lake', 'build', 'SandboxModel.Lifecycle.Protocol.SqlBlobValues'])
        physical = output/'actual-files'; physical.mkdir()
        rec['actual'] = checker.actual_files(project, files, physical, env, command)
        if rec['actual']['files'] != rec['files'] or rec['actual']['audits'] != audits:
            raise ValueError('strict checker closure differs')
        if any(sha(project/n) != h for n,h in expected.items()) or capture.base.snapshot(ROOT) != captured['inputs']:
            raise ValueError('source changed during checks')
        if any(('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') != m for n,m in captured['input_modes'].items()):
            raise ValueError('original executable modes changed')
        if any(sha(Path(p)) != row['sha256'] for row in captured['provider_sources'].values() for p in row['paths']):
            raise ValueError('fixed providers changed during checks')
        rec['derived_private_inputs'] = {n:h for n,h in expected.items() if captured['inputs'].get(n) != h}
        rec['complete'] = True
    except BaseException as error:
        rec.update(error=repr(error), traceback=traceback.format_exc())
    rec['seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')
    return rec


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True, help='existing shared proof-heavy lock')
    args = parser.parse_args(); os.umask(0o077)
    with args.lock.open('r+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX); result = run(args.output.absolute())
    print(json.dumps({k:result.get(k) for k in ('complete', 'seconds', 'error')}))
    raise SystemExit(not result['complete'])
