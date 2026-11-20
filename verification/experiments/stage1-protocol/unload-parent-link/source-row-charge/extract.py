"""Capture both actual source-row charge functions and the missing-key mutation."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'source-metadata-json'))
from capture import run
import source_metadata_capture as metadata
from prepare_transaction_capture import checked_translation
from revision_capture import declaration

CONFIG = Path('controller-service/src/capture/source.rs')
NAMESPACE = 'SandproofSourceRowCharge'
ROOT = metadata.sql.CODEC_ROOT + '::charged_bytes'
BASE = 'sandproof_controller_service::controller::lifecycle_inbox::codec::charged_bytes'
BASE_FILE = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
OPTIONS = dict(roots=[ROOT], retained_roots=[], include=[
    ROOT, BASE, metadata.sql.CODEC_ROOT + '::source_key_is_null', 'sandproof_controller_service::controller::capacity::CapacityConfig',
    'sandproof_controller_service::JournalError', 'rusqlite::row::RowIndex',
    'rusqlite::types::value_ref::ValueRef', 'rusqlite::types::from_sql::FromSql',
], opaque=['sandproof_controller_service', 'sandbox_controller_core', 'sandbox_protocol', 'rusqlite'])
VARIANTS = ('original', 'omit-future-key')


def omit_future_key(text):
    anchor = '            .checked_add(32)'
    if text.count(anchor) != 1:
        raise ValueError('the sole original future-key addition is required')
    return text.replace(anchor, anchor.replace('(32)', '(0)'), 1)


def isolate_key_match(text):
    expression = 'matches!(row.get_ref(4)?, rusqlite::types::ValueRef::Null)'
    if text.count(expression) != 1:
        raise ValueError('the sole original fifth-column Null match is required')
    helper = """
fn source_key_is_null(value: rusqlite::types::ValueRef<'_>) -> bool {
    matches!(value, rusqlite::types::ValueRef::Null)
}
"""
    return text.replace(expression, 'source_key_is_null(row.get_ref(4)?)', 1) + helper


def admit(ir, generated, target):
    translated = checked_translation(ir, OPTIONS, target, ROOT, None)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = {}
    for name, filename in ((ROOT, str(metadata.sql.CODEC)), (BASE, BASE_FILE),
                           (metadata.sql.CODEC_ROOT + '::source_key_is_null', str(metadata.sql.CODEC))):
        actual = declaration(translated['fun_decls'], name)
        entries = [row for row in mapping if row['def_id'] == actual['def_id']]
        if (len(entries) != 1 or entries[0]['is_opaque'] or 'Structured' not in actual['body']
                or entries[0]['source']['file'] != filename):
            raise ValueError('both original charge functions must be transparent: ' + name)
        result[name] = actual['def_id']
    return result


def capture(output, tools, lock, variant):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('the existing formal role is required')
    os.umask(0o077)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    fixed = json.loads((metadata.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    for name, binary in (('charon', charon), ('aeneas', aeneas)):
        if metadata.sql.digest(binary) != fixed[name]['binary_sha256']:
            raise ValueError('the original fixed tool is required: ' + name)
    output.mkdir(mode=0o700)
    result = dict(source=metadata.SOURCE, tree=metadata.TREE, variant=variant, options=OPTIONS,
                  driver=metadata.sql.digest(Path(__file__)), commands=[], complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', metadata.SOURCE], cwd=metadata.sql.ROOT, stdout=stream, check=True)
        if metadata.sql.digest(archive) != metadata.ARCHIVE:
            raise ValueError('the exact parent production archive is required')
        source = output / 'source'
        source.mkdir(mode=0o700)
        codec = isolate_key_match(metadata.reconstruct_codec(source, archive, 'original').decode())
        if variant == 'omit-future-key':
            codec = omit_future_key(codec)
        (source / metadata.sql.CODEC).write_text(codec)
        result['staged'] = {str(path): metadata.sql.digest(source / path)
                            for path in (metadata.sql.CODEC, Path(BASE_FILE))}
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                   CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        selectors = [part for flag, group in (('--start-from', 'roots'), ('--include', 'include'),
                    ('--opaque', 'opaque')) for pattern in OPTIONS[group] for part in (flag, pattern)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings',
                        '--sysroot', 'default', *selectors, '--dest-file', ir, '--',
                        '--manifest-path', 'controller-service/Cargo.toml', '--lib',
                        '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                        '-split-files', '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE,
                        '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, argv, seconds in commands:
                status = run(name, argv, seconds, source, output, env)
                result['commands'].append(status)
                if status['exit_code'] or status['bounded_stop']:
                    raise ValueError('the complete fixed ' + name + ' output is required')
        result['ir'] = metadata.sql.digest(ir)
        result['generated'] = {str(p.relative_to(generated)): metadata.sql.digest(p)
                               for p in sorted(generated.rglob('*')) if p.is_file()}
        result['admission'] = admit(ir, generated, fixed['target'])
        result['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(variant, 'complete original base and source-row charges')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', required=True, choices=VARIANTS)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
