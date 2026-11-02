"""Capture changed adopted Rust before updating retained proof fingerprints.

This entry grants no formal/native acceptance. It preserves full fresh IR,
source copies, generated files and failures for review by the batch integrator.
"""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-save-digest')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


support = module('adopted_save_recapture_support', ROOT/HERE/'support.py')
blob = support.blob
parent = support.parent
sha = support.sha


def run(output, views):
    output.mkdir(mode=0o700, parents=True)
    rec = dict(complete=False, capture_only=True, formal_acceptance=False,
        native_checks_run=False, shared_acceptance=False, commands=[], views={})
    began = time.monotonic()
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true')

    def save():
        (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')

    def command(project, folder, label, argv):
        log = folder/(label+'.log')
        start = time.monotonic()
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(label=label, argv=argv, cwd=str(project),
            exit_code=done.returncode, seconds=time.monotonic()-start,
            log=str(log), log_sha256=sha(log)))
        save()
        if done.returncode:
            raise ValueError('fixed tool failure, never a property rejection: '+label)

    try:
        if not 1 <= len(os.sched_getaffinity(0)) <= 4:
            raise ValueError('requires the batch scheduler assigned four-CPU lane')
        inputs = support.base.snapshot(ROOT)
        rec.update(inputs=inputs, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        captures = module('adopted_save_captures', ROOT/HERE/'capture.py')
        digest_capture = module('adopted_load_digest_captures', ROOT/parent.HERE/'capture.py')
        blob_capture = module('adopted_read_captures', ROOT/blob.HERE/'capture.py')
        for view in views:
            folder = output/view
            project = folder/'source'
            project.mkdir(mode=0o700, parents=True)
            for name, digest in inputs.items():
                target = project/name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT/name, target)
                if sha(target) != digest:
                    raise ValueError('source changed while copying: '+name)
            (project/'.tools').symlink_to(support.runtime.tool_root(ROOT), target_is_directory=True)
            (project/'.lake').mkdir()
            (project/'.lake/packages').symlink_to(support.runtime.package_root(ROOT), target_is_directory=True)
            (project/'artifacts/extract-target').mkdir(parents=True)
            rec['phase'] = view
            save()
            changed = {}
            try:
                if view == 'leaf':
                    row = digest_capture.run(project, folder, view, parent, command, 'leaf')
                elif view.startswith('digest-'):
                    variant = view.removeprefix('digest-')
                    derivation, changed = parent.preparation.view(project, variant)
                    row = digest_capture.run(project, folder, view, parent, command)
                    row['preparation'] = derivation
                elif view.startswith('save-') and view != 'save-update':
                    variant = view.removeprefix('save-')
                    derivation, changed = support.preparation.view(project, variant)
                    row = captures.run(project, folder, view, support, command)
                    row['preparation'] = derivation
                elif view.startswith('blob-'):
                    variant = view.removeprefix('blob-')
                    derivation, changed = blob.preparation.read_view(project, variant)
                    row = blob_capture.run(project, folder, view, blob, command, 'standard')
                    row['preparation'] = derivation
                elif view == 'save-update':
                    row = update_probe(project, folder, command, blob_capture, changed)
                else:
                    raise ValueError('unregistered narrow capture view: '+view)
                rec['views'][view] = dict(complete=True, capture=row)
            except Exception as error:
                rec['views'][view] = dict(complete=False, error=repr(error),
                    traceback=traceback.format_exc())
            expected = {**inputs, **changed}
            rec['views'][view]['private_inputs_unchanged'] = all(
                not (project/n).is_symlink() and sha(project/n) == h for n,h in expected.items())
            if not rec['views'][view]['private_inputs_unchanged']:
                rec['views'][view].update(complete=False, source_guard_error='derived source changed')
            save()
        rec['source_unchanged'] = support.base.snapshot(ROOT) == inputs
        rec['complete'] = rec['source_unchanged'] and all(v['complete'] for v in rec['views'].values())
    except Exception as error:
        rec.update(error=repr(error), traceback=traceback.format_exc())
    rec['seconds'] = time.monotonic()-began
    save()
    return rec


def update_probe(project, folder, command, blob_capture, changed):
    relative = Path('controller-service/src/controller/lifecycle_inbox/codec.rs')
    path = project/relative
    marker = 'fn proof_stage1_save_blob_update('
    if marker in path.read_text():
        raise ValueError('actual update root already exists')
    path.write_text(path.read_text()+'''
#[expect(dead_code, reason = "selected by fixed Charon")]
fn proof_stage1_save_blob_update(
    receiver: &mut StandardInboxSave<'_>,
    statement: &str,
    bytes: &[u8],
    digest: &[u8],
) -> Result<usize, JournalError> {
    InboxSaveEffects::update(receiver, statement, bytes, digest)
}
''')
    changed[str(relative)] = sha(path)
    fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
    for name, suffix in [('charon','charon/bin/charon'),('aeneas','bin/aeneas')]:
        if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
            raise ValueError('fixed proof binary differs: '+name)
    roots = ['crate::controller::lifecycle_inbox::codec::proof_stage1_save_blob_update']
    include = ['sandproof_controller_service', 'sandbox_controller_core', 'sandproof_protocol',
        'rusqlite::types::to_sql::ToSqlOutput', 'rusqlite::types::value_ref::ValueRef',
        'rusqlite::types::value::Value', 'rusqlite::types::to_sql::*::to_sql',
        'rusqlite::types::value_ref::*::from', 'rusqlite::params::*::__bind_in',
        'rusqlite::types::to_sql::*::from', 'rusqlite::types::value::*::from']
    opaque = ['rusqlite::error::Error']
    virtual = support.runtime.VIRTUAL
    ir = project/'artifacts/save-update.llbc'
    args = [str(virtual/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
        '--error-on-warnings', '--sysroot', 'default']
    for flag, values in [('--include',include),('--opaque',opaque),('--start-from',roots)]:
        for value in values:
            args += [flag,value]
    args += ['--dest-file',str(virtual/'artifacts'/ir.name),'--','--manifest-path',
        'controller-service/Cargo.toml','--lib','--locked','--offline','--target',fixed['target']]
    command(project,folder,'save-update-charon',blob_capture.wrapper(project,blob)+args)
    if ir.is_symlink() or not ir.is_file() or ir.stat().st_size > 128*2**20:
        raise ValueError('bounded original IR refused')
    raw = json.loads(ir.read_text())
    opts = raw['translated']['options']
    if raw['has_errors'] or opts['start_from'] != roots or opts['include'] != include or opts['opaque'] != opaque:
        raise ValueError('incomplete or changed update capture')
    if any(opts[n] for n in ('exclude','skip_borrowck','no_typecheck','monomorphize','remove_unused_clauses')):
        raise ValueError('weakened compiler settings')
    if {r['key'] for r in raw['translated']['target_information']} != {fixed['target']}:
        raise ValueError('wrong compiler target')
    generated = folder/'generated-update'
    generated.mkdir()
    command(project,folder,'save-update-aeneas',[str(ROOT/'.tools/aeneas/bin/aeneas'),
        '-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json',
        '-namespace','SandproofInboxSaveBlobRust','-subdir','SandproofInboxSaveBlob',
        '-dest',str(generated),str(ir)])
    return dict(capture_only=True, formal_acceptance=False, ir=str(ir), ir_sha256=sha(ir),
        ir_value_sha256=blob.ir_value_digest(raw),generated_directory=str(generated),
        generated={str(p.relative_to(generated)):sha(p) for p in generated.rglob('*') if p.is_file()},
        translation=json.loads((generated/'translation.json').read_text()))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--views', nargs='+', default=['leaf','digest-actual','save-actual','blob-actual','save-update'])
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.views)
    print(json.dumps({k:result.get(k) for k in ('complete','capture_only','seconds','error')}))
    raise SystemExit(not result['complete'])
