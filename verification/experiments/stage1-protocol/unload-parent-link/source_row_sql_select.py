"""Capture the two actual source SELECT adapters and a greatest-vacancy control."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

from frontier_capture import call_kind, indexed
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run
from revision_capture import declaration, ir_values
from source_row_write import ARCHIVE, CODEC, CODEC_ROOT, ROOT, SOURCE, TREE, digest, stage as stage_row


HERE = Path(__file__).resolve().parent / 'row-sql-select'
MODES = ('original', 'core', 'greatest-vacancy')
NAMESPACE = 'SandproofSourceRowSqlSelect'
PROBE_MODULE = CODEC_ROOT + '::proof_source_slot_sql'
PROBE = PROBE_MODULE + '::*'
EXISTING = 'SELECT id FROM controller_source_work_records WHERE work_key=?1'
VACANT = 'SELECT min(id) FROM controller_source_work_records WHERE work_key IS NULL'
CONFIG = dict(roots=[PROBE], retained_roots=[], include=[
    'sandproof_controller_service', 'sandbox_controller_core', 'sandproof_protocol',
    'rusqlite::params::*::__bind_in',
], opaque=['rusqlite::error::Error', 'rusqlite::types::from_sql::FromSqlError',
           'rusqlite::types::value_ref::ValueRef'])


def stage(source, mode):
    if mode not in MODES:
        raise ValueError('unknown source SELECT view')
    stage_row(source, 'core')
    path = source / CODEC
    text = path.read_text()
    if text.count(EXISTING) != 1 or text.count(VACANT) != 1:
        raise ValueError('actual existing and vacant SELECT literals must each occur once')
    if mode != 'original':
        zero_args = f'"{VACANT}",\n            [],'
        if text.count(zero_args) != 1:
            raise ValueError('expected the original zero-argument vacancy query')
        text = text.replace(zero_args, zero_args.replace('[],', '(),'), 1)
    if mode == 'greatest-vacancy':
        text = text.replace(VACANT, VACANT.replace('min(id)', 'max(id)'), 1)
    path.write_text(text + (HERE / 'probes.rs.in').read_text())
    return digest(path)


def admit(ir, generated, target):
    translated = checked_translation(ir, CONFIG, target, PROBE, None)
    functions = translated['fun_decls']
    names = {row['def_id']: row for row in json.loads((generated / 'translation.json').read_bytes())['functions']}
    result = {}
    for name in ('existing', 'vacant'):
        probe = declaration(functions, PROBE_MODULE + '::' + name)
        forwarded, = (row['call'] for row in ir_values(probe['body']['Structured']['body'], 'Call'))
        adapter = indexed(functions, call_kind(forwarded)['Fun'])
        called = [indexed(functions, kind['Fun']) for row in ir_values(adapter['body']['Structured']['body'], 'Call')
                  if 'Fun' in (kind := call_kind(row['call']))]
        query, = (decl for decl in called
                  if names.get(decl['def_id'], {}).get('rust_name') == 'rusqlite::{rusqlite::Connection}::query_row')
        if any(names[item['def_id']]['source']['file'] != str(CODEC) or names[item['def_id']]['is_opaque']
               for item in (probe, adapter)):
            raise ValueError('the two original source SELECT bodies must remain transparent')
        if names[adapter['def_id']]['rust_name'].rsplit('::', 1)[-1] != name:
            raise ValueError('the original corresponding trait method must be selected')
        result[name] = dict(probe=probe['def_id'], adapter=adapter['def_id'], original_query=query['def_id'])
    return result


def capture(mode, output, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * GIB or shutil.disk_usage(ROOT).free < 64 * GIB):
        raise ValueError('formal role and actual capture filesystem budget required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(digest(path) != fixed[name]['binary_sha256'] for name, path in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('fixed proof binaries required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], text=True, cwd=ROOT).strip() != TREE:
        raise ValueError('fixed integration tree required')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source_commit=SOURCE, source_tree=TREE, complete=False, commands=[],
                  public_acceptance=False, driver_sha256=digest(Path(__file__)), cpus=cpus)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=ROOT, stdout=stream, check=True)
        if digest(archive) != ARCHIVE:
            raise ValueError('fixed source SELECT input archive differs')
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        report['codec_sha256'] = stage(source, mode)
        ir, generated = output / 'actual.llbc', output / 'generated'
        paths = [part for flag, names in (('--include', CONFIG['include']), ('--opaque', CONFIG['opaque']))
                 for name in names for part in (flag, name)]
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        '--start-from', PROBE, *paths, '--dest-file', ir, '--', '--manifest-path',
                        'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for label, argv, seconds in commands:
                result = run(label, argv, seconds, source, output, env)
                report['commands'].append(result)
                if result['exit_code'] != 0 or result['bounded_stop']:
                    raise ValueError(label + ' did not complete')
                if label == 'charon':
                    report['ir_sha256'] = digest(ir)
        if digest(ir) != report['ir_sha256']:
            raise ValueError('captured source SELECT IR changed')
        report['admission'] = admit(ir, generated, fixed['target'])
        report['generated_sha256'] = {str(file.relative_to(generated)): digest(file)
                                      for file in sorted(generated.rglob('*')) if file.is_file()}
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(mode, report['admission'])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=MODES, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
