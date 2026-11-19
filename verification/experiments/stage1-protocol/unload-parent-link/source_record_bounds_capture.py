"""Capture the original source record capacity guard and its query."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import source_row_sql_composition as sql
from source_sql_cursor_capture import SCAN, staged_codec
from prepare_transaction_capture import checked_translation, method
from probe_prepare_iterators import GIB, run
from revision_capture import declaration
from source_save_row_list_capture import ARCHIVE, SOURCE, STAGED, TREE


NAMESPACE = 'SandproofSourceRecordBounds'
LOAD = sql.CODEC_ROOT + '::load_records'
RESERVED = sql.CODEC_ROOT + '::reserved_records'
PER_SESSION = sql.CODEC_ROOT + '::records_per_session'
CONVERT = sql.CODEC_ROOT + '::sql'
SOURCE_CONFIG = 'sandproof_controller_service::capture::source'
POLICY = 'sandproof_controller_service::controller::checkpoint::config'
RETIREMENT = 'sandproof_controller_service::capture::retirement'
AGGREGATE_SQL = ('SELECT count(*),COALESCE(min(id),0),COALESCE(max(id),0),COALESCE(min(reserve_bytes),0),'
                 'COALESCE(max(reserve_bytes),0),COALESCE(max(length(state_json)),0) '
                 'FROM controller_source_work_records')
VARIANTS = ('original', 'deployment-cap-for-length')
CFG = dict(roots=[LOAD], retained_roots=[], include=[
    LOAD, RESERVED, PER_SESSION, CONVERT, SOURCE_CONFIG + '::SourceObservationConfig',
    SOURCE_CONFIG + '::*::required_bytes', sql.CODEC_ROOT + '::Configuration',
    sql.CODEC_ROOT + '::VALUE_LIMIT',
    *(RETIREMENT + '::MATERIAL_' + name + '_MAX_BYTES'
      for name in ('RETIREMENT_PLAN', 'RETIREMENT_RECORD', 'RELEASE_RECORD')),
    'sandproof_controller_service::controller::capacity::CapacityConfig',
    'sandproof_controller_service::controller::checkpoint::config::MaintenanceConfig',
    'sandproof_controller_service::JournalError',
    'rusqlite::params::Params', 'rusqlite::types::from_sql::FromSql',
    'rusqlite::row::RowIndex', 'core::ops::function::FnOnce', 'rusqlite::types::value_ref::ValueRef',
], opaque=['sandproof_controller_service', 'sandbox_controller_core', 'sandbox_protocol',
           'serde_json', 'rusqlite', SCAN, POLICY])


def substitute_length_limit(text):
    original = '        || u64::try_from(bounds.5).map_err(|_| JournalError::CorruptStore)? > reserve\n'
    if text.count(original) != 1:
        raise ValueError('the sole original payload length comparison must be present')
    return text.replace(original, original.replace('> reserve', '> cap.max_sql_value_bytes'), 1)


def reconstruct_codec(source, archive, variant):
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    if sql.stage(source, 'core') != STAGED:
        raise ValueError('the original reviewed save projection differs')
    text = staged_codec((source / sql.CODEC).read_text(), 'original')
    if text.count(AGGREGATE_SQL) != 1:
        raise ValueError('the original six-column bounds query is required')
    return (substitute_length_limit(text) if variant == 'deployment-cap-for-length' else text).encode()


def admit(ir, generated, target):
    translated = checked_translation(ir, CFG, target, LOAD, None)
    functions = translated['fun_decls']
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = {}
    for name, source, opaque in (
        (LOAD, str(sql.CODEC), False),
        (RESERVED, str(sql.CODEC), False),
        (PER_SESSION, str(sql.CODEC), False),
        (CONVERT, str(sql.CODEC), False),
        (SCAN, str(sql.CODEC), True),
        (POLICY, 'controller-service/src/controller/checkpoint.rs', True),
        ('required_bytes', 'controller-service/src/capture/source.rs', False),
    ):
        actual = (method(functions, SOURCE_CONFIG, name) if name == 'required_bytes'
                  else declaration(functions, name))
        bodies = [item for item in mapping if item['def_id'] == actual['def_id']]
        if (len(bodies) != 1 or bodies[0]['is_opaque'] is not opaque
                or bodies[0]['source']['file'] != source):
            raise ValueError('the original capacity helper or boundary changed: ' + name)
        result[name] = actual['def_id']
    return result


def capture(output, tools, lock, variant):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or min(shutil.disk_usage(path).free for path in (output.parent, sql.ROOT)) < 80 * GIB):
        raise ValueError('the original formal role and capture budget are required')
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.umask(0o077)
    fixed = json.loads((sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(sql.digest(binary) != fixed[name]['binary_sha256']
           for name, binary in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('the original tool binaries are required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], cwd=sql.ROOT, text=True).strip() != TREE:
        raise ValueError('the fixed production tree is required')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(source=SOURCE, tree=TREE, variant=variant, options=CFG, driver=sql.digest(Path(__file__)),
                  commands=[], complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=sql.ROOT, stdout=stream, check=True)
        if sql.digest(archive) != ARCHIVE:
            raise ValueError('the fixed production archive is required')
        codec = source / sql.CODEC
        codec.write_bytes(reconstruct_codec(source, archive, variant))
        report['codec'] = sql.digest(codec)

        selectors = [item for flag, group in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                               ('--include', 'include'),
                                               ('--opaque', 'opaque'))
                     for pattern in CFG[group] for item in (flag, pattern)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                   CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        commands = (
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                        '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        )
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, argv, timeout in commands:
                verdict = run(name, argv, timeout, source, output, env)
                report['commands'].append(verdict)
                if verdict['exit_code'] or verdict['bounded_stop']:
                    raise ValueError(f'the fixed {name} did not finish')
        report['ir'] = sql.digest(ir)
        report['generated'] = {str(path.relative_to(generated)): sql.digest(path)
                               for path in sorted(generated.rglob('*')) if path.is_file()}
        report['admission'] = admit(ir, generated, fixed['target'])
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(report['admission'])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    cli.add_argument('--variant', required=True, choices=VARIANTS)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
