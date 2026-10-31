"""Re-extract original calls and inspect the concrete protocol Serialize graph.

This entry produces source-binding evidence for A6/B4 review. It does not turn
the fixed library contract into a Lean theorem or issue a shared native seal.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

from check import check
import library

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/serde-binding')
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('serde_binding_base', ROOT / HERE.parent / 'schema-leaves/formal.py')
bytes_module = module('serde_binding_bytes', ROOT / HERE.parent / 'envelope-bytes/formal.py')
writer = module('serde_binding_writer', ROOT / HERE.parent / 'envelope-writer/formal.py')
canonical = module('serde_binding_canonical', ROOT / 'tools/check_foundations_extraction.py').llbc_digest


def run(output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='protocol4-serde-source-binding', complete=False,
                  a6_instance_adopted=False, library_refinement_proved=False,
                  ning_acceptance=False, shared_acceptance=False, full_v3=False, commands=[])
    try:
        byte_inventory = bytes_module.inventory(ROOT)
        writer_inventory = writer.inventory(ROOT)
        schema = bytes_module.schema.inventory(ROOT)
        candidate = json.loads((ROOT / HERE / 'candidate.json').read_text())
        record['library_inputs'] = library.check(candidate)
        inputs = base.snapshot(ROOT)
        record.update(inputs=inputs, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, wanted in inputs.items():
            target = project / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
            if base.sha(target) != wanted:
                raise ValueError('source changed while copying: ' + name)
        runtime = module('protocol_stable_tools', ROOT / HERE.parent / 'formal_runtime.py')
        (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project / 'artifacts/extract-target').mkdir(parents=True)
        fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
        for name, path in [('charon', 'aeneas/charon/bin/charon'), ('aeneas', 'aeneas/bin/aeneas')]:
            if base.sha(ROOT / '.tools' / path) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool changed: ' + name)
        record['tools'] = {name: fixed[name] for name in ('charon', 'aeneas')}
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
            *runtime.tool_bind(ROOT), '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL / 'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        record['roots'] = {}
        for name in ('accepted_encode', 'recorded_encode', 'writer'):
            if name == 'writer':
                command = ['.tools/lean-mcp/bin/python', str(HERE.parent / 'envelope-writer/extract.py'),
                           '--mode', 'calls', '--label', 'writer-original-calls-01']
                raw_name = 'writer-original-calls-01.llbc'
            else:
                raw_name = 'bytes-' + name + '.llbc'
                command = ['.tools/aeneas/charon/bin/charon', 'cargo', '--preset=aeneas',
                    '--error-on-warnings', '--sysroot', 'default', '--include', 'sandproof_protocol',
                    '--start-from', 'crate::' + name, '--dest-file', '../../../../artifacts/' + raw_name,
                    '--', '--manifest-path', str(HERE.parent / 'envelope-bytes/Cargo.toml'),
                    '--lib', '--locked', '--offline']
            log = output / (name + '-charon.log')
            begin = time.monotonic()
            with log.open('xb') as stream:
                result = subprocess.run(wrapper + command, cwd=project,
                    env=dict(os.environ, PYTHONDONTWRITEBYTECODE='1'), stdin=subprocess.DEVNULL,
                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
            record['commands'].append(dict(argv=wrapper + command, cwd=str(project),
                seconds=time.monotonic()-begin, exit_code=result.returncode, log_sha256=base.sha(log)))
            if result.returncode:
                raise ValueError('original extraction failed: ' + str(log))
            rawpath = project / 'artifacts' / raw_name
            raw = json.loads(rawpath.read_text())
            if name == 'writer':
                actual = writer.binding.snapshot(raw, base.sha(project / HERE.parent / 'envelope-writer/roots.rs.in'))
                frozen = writer_inventory['original_calls']
                checker = writer.binding
            else:
                actual = bytes_module.binding.snapshot(raw, name,
                    base.sha(project / HERE.parent / 'envelope-bytes/roots.rs.in'))
                frozen = byte_inventory['byte_roots'][name]
                checker = bytes_module.binding
            if (checker.digest(actual) != frozen['snapshot_sha256'] or
                    canonical(raw) != frozen['llbc_canonical_sha256']):
                raise ValueError('original complete call/type/dictionary/body changed: ' + name)
            evidence = check(raw, schema)
            report_path = output / (name + '-binding.json')
            report_path.write_text(json.dumps(evidence, indent=2)+'\n')
            record['roots'][name] = dict(llbc_sha256=base.sha(rawpath),
                canonical_llbc_sha256=canonical(raw), report_sha256=base.sha(report_path),
                original_bodies=len(evidence['bodies']), source_checks=len(evidence['checked']))
        for name, wanted in inputs.items():
            if base.sha(project / name) != wanted:
                raise ValueError('original source changed during extraction: ' + name)
        if inputs != base.snapshot(ROOT):
            raise ValueError('source inputs changed during admission')
        if record['library_inputs'] != library.check(candidate):
            raise ValueError('library sources/features changed during admission')
        record['complete'] = True
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute())
    print(json.dumps({k: result.get(k) for k in ('kind', 'complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
