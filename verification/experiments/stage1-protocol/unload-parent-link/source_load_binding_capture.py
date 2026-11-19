"""Capture the real SQLite source row consumer and original full work-key guard."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import source_row_sql_composition as sql
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run
from revision_capture import declaration
from source_save_row_list_capture import ARCHIVE, SOURCE, STAGED, TREE


NAMESPACE = 'SandproofSourceLoadBinding'
HERE = Path(__file__).with_name('source-load-binding')
READER = sql.CODEC_ROOT + '::read_loaded_rows'
DECODE = sql.CODEC_ROOT + '::decode_loaded_record'
WORK = 'sandproof_controller_service::controller::lifecycle_inbox::source_work'
CFG = dict(roots=[READER], retained_roots=[], include=[
    READER, DECODE, WORK + '::SourceWork', WORK + '::SourceWorkRecord',
    'sandproof_controller_service::capture::source::SourceObservationRequirement',
    'sandproof_controller_service::JournalError', 'sandbox_protocol::lifecycle::digest::ContentDigest',
    'sandbox_protocol::lifecycle::digest::*::as_bytes',
], opaque=['sandproof_controller_service', 'sandbox_controller_core', 'sandbox_protocol',
           'serde_json::*::to_vec', 'rusqlite::error::Error'])


SELECT = 'SELECT state_json,sha256,work_key FROM controller_source_work_records ORDER BY id'


def staged_codec(text):
    front = text.index('    let mut records = Vec::new();', text.index('fn load_records('))
    raw_start = text.index('statement.query_map([], |row| {', front)
    raw_end = text.index('})? {', raw_start) + len('})?')
    stop = text.index('\n}\n', raw_end)
    loader = (HERE / 'loader.rs.in').read_text()
    original = text[raw_end:stop]
    key_guard = original[original.index('            (Some(record), Some(key))'):
                         original.index('            {\n', original.index('(Some(record), Some(key))'))]
    if (text[:front].count(SELECT) != 1 or original.count('records.push(record);') != 1
            or original.count('let (bytes, sha, key) = row?;') != 1
            or original.count('let record: Option<SourceWorkRecord> = checked(&bytes, &sha)?;') != 1
            or original.count('(None, None) => (),') != 1
            or original.count('_ => return Err(JournalError::CorruptStore),') != 1
            or ''.join(key_guard.split()).replace('serde_json::to_vec', 'encode_load_anchor')
                not in ''.join(loader.split())):
        raise ValueError('the original cursor, row consumption and full key guard must be present')
    return text[:front] + '    let rows = ' + text[raw_start:raw_end] + ';\n    read_loaded_rows(rows)' + text[stop:] + '\n' + loader


def admit(ir, generated, target):
    translated = checked_translation(ir, CFG, target, READER, None)
    functions = translated['fun_decls']
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = {}
    for name, expected in ((READER, 3), (DECODE, 1)):
        actual = declaration(functions, name)
        bodies = [item for item in mapping if item['def_id'] == actual['def_id']]
        if (len(bodies) != expected or 'Structured' not in actual['body']
                or any(item['is_opaque'] or item['source']['file'] != str(sql.CODEC) for item in bodies)):
            raise ValueError('the original row operation must be transparent: ' + name)
        result[name] = actual['def_id']
    for name in ('checked_source_payload', 'encode_load_anchor'):
        actual = declaration(functions, sql.CODEC_ROOT + '::' + name)
        result[name] = actual['def_id']
    return result


def capture(output, tools, lock):
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
    report = dict(source=SOURCE, tree=TREE, options=CFG, driver=sql.digest(Path(__file__)),
                  commands=[], complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=sql.ROOT, stdout=stream, check=True)
        if sql.digest(archive) != ARCHIVE:
            raise ValueError('the fixed production archive is required')
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        if sql.stage(source, 'core') != STAGED:
            raise ValueError('the original reviewed save projection differs')
        codec = source / sql.CODEC
        codec.write_text(staged_codec(codec.read_text()))
        report['codec'] = sql.digest(codec)

        selectors = [item for flag, group in (('--start-from', 'roots'), ('--include', 'include'),
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
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
