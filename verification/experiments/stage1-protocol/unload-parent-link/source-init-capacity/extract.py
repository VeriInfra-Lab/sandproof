"""Capture the actual source initializer, row budget and original capacity guards."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from prepare_transaction_capture import method
from source_save_row_list_capture import ARCHIVE, SOURCE, TREE
import source_row_sql_composition as sql
from probe_prepare_iterators import GIB, run
from revision_capture import declaration

CODEC = Path('controller-service/src/controller/lifecycle_inbox/source_work/codec.rs')
LIB = Path('controller-service/src/lib.rs')
WORK = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::codec'
POLICY = 'sandproof_controller_service::controller::checkpoint::config'
SOURCE_CONFIG = 'sandproof_controller_service::capture::source'
ROOT = WORK + '::initialize'
ENTRY = WORK.rsplit('::', 1)[0] + '::initialize'
BUDGET = WORK + '::initial_source_bytes'
GUARD = WORK + '::ensure_source_capacity'
INSERT = WORK + '::insert_initial_source_row'
ROWS = WORK + '::write_initial_source_rows'
NEXT = WORK + '::next_initial_source_id'
ENCODE = WORK + '::encode_initial_configuration'
DIGEST = WORK + '::initial_source_digest'
NAMESPACE = 'SandproofSourceInitCapacity'
VARIANTS = ('original', 'omit-row-overhead')
EXPRESSION = 'METADATA_BYTES + 48 + rows * (reserve + 80)'
GUARDS = """    if reserve > cap.max_sql_value_bytes
        || reserve > VALUE_LIMIT
        || METADATA_BYTES > cap.max_sql_value_bytes
        || rows + 1 > cap.max_sql_rows
        || source_bytes > cap.max_sql_bytes
    {
        return Err(JournalError::MaintenanceCapacity);
    }
"""
OPTIONS = dict(roots=[ENTRY, ROOT], retained_roots=[BUDGET, GUARD], include=[
    ENTRY, ROOT, BUDGET, GUARD, INSERT, ROWS, WORK + '::reserved_records', WORK + '::records_per_session', WORK + '::sql',
    WORK + '::Configuration', WORK + '::METADATA_BYTES', WORK + '::VALUE_LIMIT', WORK + '::TABLE', WORK + '::RECORDS_TABLE',
    SOURCE_CONFIG + '::SourceObservationConfig', SOURCE_CONFIG + '::*::required_bytes', SOURCE_CONFIG + '::*::valid',
    *('sandproof_controller_service::capture::retirement::MATERIAL_' + n + '_MAX_BYTES'
      for n in ('RETIREMENT_PLAN', 'RETIREMENT_RECORD', 'RELEASE_RECORD')),
    'sandproof_controller_service::controller::lifecycle_inbox::model::LifecycleInboxConfig',
    'sandproof_protocol::internal::NodeIdentity', 'sandproof_protocol::internal::PositiveCounter',
    'sandproof_protocol::NodeId', 'sandproof_protocol::DeploymentId',
    'sandproof_protocol::lifecycle::authority::CenterIdentity', 'sandproof_protocol::lifecycle::authority::CenterId',
    'sandproof_protocol::lifecycle::digest::ContentDigest', 'sandproof_controller_service::capture::HostEpochRegistration',
    'sandproof_protocol::internal::*::clone',
    POLICY, POLICY.rsplit('::', 1)[0] + '::config_using',
    *(POLICY + '::' + n for n in ('MaintenanceConfig', 'ReadConfigEffects', 'StandardConfigRead', 'read', 'read_using')),
    *(POLICY + '::*::' + n for n in ('validate', 'query_i64', 'query_bytes', 'decode_maintenance')),
    *('sandproof_controller_service::controller::capacity::' + n for n in (
        'CapacityConfig', 'config', 'config_using', 'invalid_capacity_values', 'add', 'mul',
        'SCALAR_BYTES', 'OPTIONS_BYTES', '*::validate')),
    'sandproof_controller_service::JournalError', 'sandproof_controller_service::*::from',
    'sandproof_controller_service::host_contract::*::MAX_ENCODED_BYTES',
    'sandproof_controller_service::journal::MAINTAINED_FORMAT_VERSION',
    'core::ops::range::_::contains', 'core::ops::range::_::start_bound',
    'core::ops::range::_::end_bound', 'core::ops::range::RangeInclusive',
    'core::option::*::and_then', 'rusqlite::row::RowIndex', 'rusqlite::types::from_sql::FromSql',
    'rusqlite::params::Params', 'core::ops::function::FnOnce',
], opaque=['sandproof_controller_service', 'sandbox_controller_core', 'sandproof_protocol', 'serde_json',
           'rusqlite', POLICY + '::_', NEXT, ENCODE, DIGEST])


def mutate(text, variant):
    anchor = '    let source_bytes = ' + EXPRESSION + ';\n'
    if text.count(anchor) != 1:
        raise ValueError('one complete original source budget expression is required')
    if variant == 'omit-row-overhead':
        return text.replace(anchor, anchor.replace('reserve + 80', 'reserve + 0'), 1)
    if variant != 'original':
        raise ValueError('unknown variant')
    return text


def reconstruct(archive, output, variant):
    with tarfile.open(archive) as tree:
        tree.extractall(output, filter='data')
    path = output / CODEC
    original = mutate(path.read_text(), variant)
    expr = EXPRESSION if variant == 'original' else EXPRESSION.replace('reserve + 80', 'reserve + 0')
    anchor = '    let source_bytes = ' + expr + ';\n' + GUARDS
    if original.count(anchor) != 1:
        raise ValueError('the contiguous original budget and five capacity checks are required')
    helpers = ('fn initial_source_bytes(rows: u64, reserve: u64) -> u64 {\n    ' + expr + '\n}\n\n'
        'fn ensure_source_capacity(rows: u64, reserve: u64, cap: &crate::CapacityConfig) -> Result<(), JournalError> {\n'
        '    let source_bytes = initial_source_bytes(rows, reserve);\n' + GUARDS + '    Ok(())\n}\n\n')
    entry = 'pub(super) fn initialize(\n'
    if original.count(entry) != 1:
        raise ValueError('one original initializer is required')
    staged = original.replace(anchor, '    ensure_source_capacity(rows, reserve, &cap)?;\n', 1)
    staged = staged.replace(entry, helpers + entry, 1)
    inner = """        connection.execute(
            "INSERT INTO controller_source_work_records VALUES(?1,?2,?3,?4,NULL)",
            params![sql(id)?, empty.as_slice(), sha.as_slice(), sql(reserve)?],
        )?;
"""
    loop = '    for id in 1..=rows {\n' + inner + '    }\n    Ok(())\n'
    if staged.count(loop) != 1:
        raise ValueError('the sole original row loop is required')
    insert = ('fn insert_initial_source_row(connection: &Connection, id: u64, empty: &[u8], sha: &[u8], reserve: u64) -> Result<(), JournalError> {\n'
              + inner.replace('empty.as_slice(), sha.as_slice()', 'empty, sha').replace('        ', '    ', 1)
              + '    Ok(())\n}\n\n')
    normalized = """    write_initial_source_rows(connection, rows, reserve, empty.as_slice(), sha.as_slice())
"""
    row_loop = """fn next_initial_source_id(ids: &mut std::ops::RangeInclusive<u64>) -> Option<u64> {
    ids.next()
}

fn write_initial_source_rows(connection: &Connection, rows: u64, reserve: u64, empty: &[u8], sha: &[u8]) -> Result<(), JournalError> {
    let mut ids = 1..=rows;
    loop {
        match next_initial_source_id(&mut ids) {
            None => break Ok(()),
            Some(id) => match insert_initial_source_row(connection, id, empty, sha, reserve) {
                Ok(()) => (),
                Err(error) => break Err(error),
            },
        }
    }
}

"""
    staged = staged.replace(loop, normalized, 1).replace(entry, insert + row_loop + entry, 1)
    encode = '    let bytes = serde_json::to_vec(&value)?;\n'
    if staged.count(encode) != 1:
        raise ValueError('the sole original configuration encoder is required')
    encode_helper = ('fn encode_initial_configuration(value: &Configuration) -> Result<Vec<u8>, serde_json::Error> {\n'
                     '    serde_json::to_vec(value)\n}\n\n')
    staged = staged.replace(encode, '    let bytes = encode_initial_configuration(&value)?;\n', 1)
    staged = staged.replace(entry, encode_helper + entry, 1)
    metadata_params = """params![
            bytes,
            Sha256::digest(&bytes).as_slice(),
            sql(METADATA_BYTES)?
        ]"""
    if staged.count(metadata_params) != 1:
        raise ValueError('the original metadata INSERT arguments are required')
    staged = staged.replace(metadata_params,
        '(bytes.as_slice(), Sha256::digest(&bytes).as_slice(), sql(METADATA_BYTES)?)', 1)
    row_params = 'params![sql(id)?, empty, sha, sql(reserve)?]'
    if staged.count(row_params) != 1:
        raise ValueError('the original four row INSERT arguments are required')
    staged = staged.replace(row_params, '(sql(id)?, empty, sha, sql(reserve)?)', 1)
    start = staged.index(entry)
    end = staged.index('\npub(in crate::controller) fn validate(', start)
    initializer = staged[start:end]
    for original_digest in ('Sha256::digest(&bytes)', 'Sha256::digest(empty)'):
        if initializer.count(original_digest) != 1:
            raise ValueError('one original initialization digest call is required')
        initializer = initializer.replace(original_digest, original_digest.replace('Sha256::digest', 'initial_source_digest'), 1)
    staged = staged[:start] + initializer + staged[end:]
    staged = staged.replace(entry,
        'fn initial_source_digest(bytes: &[u8]) -> [u8; 32] {\n'
        '    Sha256::digest(bytes).into()\n}\n\n' + entry, 1)
    path.write_text(staged)
    path = output / LIB
    anchor = 'impl From<serde_json::Error> for JournalError {\n    fn from(error: serde_json::Error) -> Self {'
    text = path.read_text()
    if text.count(anchor) != 1:
        raise ValueError('the original JSON error conversion must be unique')
    text = text.replace(anchor, '#[charon::rename("JournalErrorFromJson")]\nimpl From<serde_json::Error> for JournalError {\n'
        '    #[charon::rename("from_json")]\n    fn from(error: serde_json::Error) -> Self {', 1)
    path.write_text('#![feature(register_tool)]\n#![register_tool(charon)]\n' + text)


def admit(ir, generated, target):
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    if (raw.get('has_errors') is not False
            or any(options[n] is not False for n in ('skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or options['exclude'] != []
            or any(options[k] != OPTIONS[v] for k,v in (('start_from','roots'),('start_from_if_exists','retained_roots'),
                ('include','include'),('opaque','opaque')))
            or {r['key'] for r in translated['target_information']} != {target}):
        raise ValueError('the fixed real initializer selection is required')
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    functions = translated['fun_decls']
    selected = {}
    for name in (ENTRY, ROOT, BUDGET, GUARD, INSERT, ROWS):
        actual = declaration(functions, name)
        rows = [r for r in mapping if r['def_id'] == actual['def_id']]
        expected_file = CODEC if name != ENTRY else CODEC.parent.parent / 'source_work.rs'
        if not rows or any(r['is_opaque'] or r['source']['file'] != str(expected_file) for r in rows) or 'Structured' not in actual['body']:
            raise ValueError('the complete actual initializer and budget helpers must be transparent: ' + name)
        selected[name] = actual['def_id']
    for name in (NEXT, ENCODE, DIGEST):
        actual = declaration(functions, name)
        rows = [r for r in mapping if r['def_id'] == actual['def_id']]
        if len(rows) != 1 or rows[0]['is_opaque'] is not True or rows[0]['source']['file'] != str(CODEC):
            raise ValueError('the exact thin library call must be an explicit port: ' + name)
    return dict(functions=selected, mapped_functions=len(mapping))


def capture(output, tools, lock, variant):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * GIB):
        raise ValueError('the formal role and original capture budget are required')
    os.umask(0o077)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    fixed = json.loads((sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(sql.digest(binary) != fixed[name]['binary_sha256'] for name, binary in (
            ('charon', charon), ('aeneas', aeneas))):
        raise ValueError('the original fixed tools are required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], cwd=sql.ROOT, text=True).strip() != TREE:
        raise ValueError('the fixed production tree is required')
    output.mkdir(mode=0o700)
    report = dict(source=SOURCE, tree=TREE, variant=variant, options=OPTIONS,
                  driver=sql.digest(Path(__file__)), commands=[], complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=sql.ROOT, stdout=stream, check=True)
        if sql.digest(archive) != ARCHIVE:
            raise ValueError('the exact original production archive is required')
        source = output / 'source'
        source.mkdir(mode=0o700)
        reconstruct(archive, source, variant)
        report['staged'] = {str(path): sql.digest(source / path) for path in (CODEC, LIB)}
        selectors = [part for flag, group in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'), ('--include', 'include'),
                    ('--opaque', 'opaque')) for pattern in OPTIONS[group] for part in (flag, pattern)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                   CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                        '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE,
                        '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, argv, seconds in commands:
                reply = run(name, argv, seconds, source, output, env)
                report['commands'].append(reply)
                if reply['exit_code'] or reply['bounded_stop']:
                    raise ValueError('the complete fixed ' + name + ' result is required')
        report['ir'] = sql.digest(ir)
        report['generated'] = {str(p.relative_to(generated)): sql.digest(p)
                               for p in sorted(generated.rglob('*')) if p.is_file()}
        report['admission'] = admit(ir, generated, fixed['target'])
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(variant, 'complete actual source initialization capacity')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
