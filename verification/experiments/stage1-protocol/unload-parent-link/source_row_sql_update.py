"""Capture the actual fixed four-parameter source UPDATE and a swapped-binding variant."""
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
from source_row_write import ARCHIVE, CODEC, CODEC_ROOT, ROOT, SOURCE, TREE, digest
from stage_row_sql_update import HERE, MODES, stage


NAMESPACE = 'SandproofSourceRowSqlUpdate'
PROBE = CODEC_ROOT + '::proof_source_row_sql_update'
CONFIG = dict(roots=[PROBE], retained_roots=[], include=[
    'sandproof_controller_service', 'sandbox_controller_core', 'sandproof_protocol',
    'rusqlite::types::to_sql::ToSqlOutput', 'rusqlite::types::value_ref::ValueRef',
    'rusqlite::types::value::Value', 'rusqlite::types::to_sql::*::to_sql',
    'rusqlite::types::value_ref::*::from', 'rusqlite::params::*::__bind_in',
    'rusqlite::types::to_sql::*::from', 'rusqlite::types::value::*::from',
], opaque=['rusqlite::error::Error'])


def admit(ir, generated, target):
    translated = checked_translation(ir, CONFIG, target, PROBE, None)
    functions = translated['fun_decls']
    probe = declaration(functions, PROBE)
    forwarded, = (entry['call'] for entry in ir_values(probe['body']['Structured']['body'], 'Call'))
    body = indexed(functions, call_kind(forwarded)['Fun'])
    execute = [indexed(functions, kind['Fun']) for entry in ir_values(body['body']['Structured']['body'], 'Call')
               if 'Fun' in (kind := call_kind(entry['call']))]
    selected, = (decl for decl in execute if decl['item_meta']['name'][-1] == {'Ident': ['execute', 0]})
    names = json.loads((generated / 'translation.json').read_bytes())['functions']
    for actual in (probe, body):
        row, = (item for item in names if item['def_id'] == actual['def_id'])
        if row['source']['file'] != str(CODEC) or row['is_opaque'] is not False:
            raise ValueError('the original actual SQLite delegating bodies must remain transparent')
    return dict(probe=probe['def_id'], adapter=body['def_id'], original_execute=selected['def_id'])


def capture(mode, output, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * GIB or shutil.disk_usage(ROOT).free < 64 * GIB):
        raise ValueError('proof role and actual capture filesystem budget required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(digest(path) != fixed[name]['binary_sha256'] for name, path in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('fixed tools required')
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
            raise ValueError('fixed integration archive required')
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        report['codec_sha256'] = stage(source, mode)
        ir, generated = output / 'actual.llbc', output / 'generated'
        paths = [entry for flag, names in (('--include', CONFIG['include']), ('--opaque', CONFIG['opaque']))
                 for name in names for entry in (flag, name)]
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
            raise ValueError('original extracted IR changed')
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
