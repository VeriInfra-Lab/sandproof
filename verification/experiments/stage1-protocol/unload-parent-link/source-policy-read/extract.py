"""Capture the original complete maintenance-policy read and transparent forwarding roots."""
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

CONFIG = Path('controller-service/src/controller/checkpoint/config.rs')
CHECKPOINT = Path('controller-service/src/controller/checkpoint.rs')
LIB = Path('controller-service/src/lib.rs')
POLICY = 'sandproof_controller_service::controller::checkpoint::config'
ROOT = 'sandproof_controller_service::controller::checkpoint::proof_source_policy_read'
EFFECTS_ROOT = 'sandproof_controller_service::controller::checkpoint::proof_source_policy_effects'
NAMESPACE = 'SandproofSourcePolicyRead'
VARIANTS = ('original', 'skip-maintenance-validation')
FORWARD = b'\n// Transparent extraction roots; no replacement policy or SQL algorithm.\n#[allow(dead_code)]\npub(crate) fn proof_source_policy_read(\n    connection: &Connection,\n) -> Result<Option<MaintenanceConfig>, JournalError> {\n    config(connection)\n}\n#[allow(dead_code)]\npub(crate) fn proof_source_policy_effects<E: ReadConfigEffects>(\n    effects: &mut E,\n) -> Result<Option<MaintenanceConfig>, JournalError> {\n    config_using(effects)\n}\n'
OPTIONS = dict(roots=[ROOT, EFFECTS_ROOT], retained_roots=[POLICY + '::*::validate'], include=[
    ROOT, EFFECTS_ROOT, POLICY, POLICY.rsplit('::', 1)[0] + '::config_using',
    *(POLICY + '::' + n for n in ('MaintenanceConfig', 'ReadConfigEffects', 'StandardConfigRead', 'read', 'read_using')),
    *(POLICY + '::*::' + n for n in ('validate', 'query_i64', 'query_bytes', 'decode_maintenance')),
    *('sandproof_controller_service::controller::capacity::' + n for n in (
        'CapacityConfig', 'invalid_capacity_values', 'add', 'mul', 'SCALAR_BYTES', 'OPTIONS_BYTES', '*::validate')),
    'sandproof_controller_service::JournalError', 'sandproof_controller_service::*::from',
    'sandproof_controller_service::host_contract::*::MAX_ENCODED_BYTES', 'sandproof_controller_service::journal::MAINTAINED_FORMAT_VERSION',
    'core::ops::range::_::contains', 'core::ops::range::_::start_bound',
    'core::ops::range::_::end_bound', 'core::ops::range::RangeInclusive',
    'core::option::*::and_then', 'rusqlite::row::RowIndex', 'rusqlite::types::from_sql::FromSql',
    'rusqlite::params::Params', 'core::ops::function::FnOnce',
], opaque=['sandproof_controller_service', 'sandbox_controller_core', 'sandbox_protocol', 'serde_json', 'rusqlite', POLICY + '::_'])


def skip_validation(text):
    anchor = '    let config: MaintenanceConfig = effects.decode_maintenance(&bytes)?;\n    config.validate()?;\n'
    if text.count(anchor) != 1:
        raise ValueError('the single original decoded-policy validation is required')
    return text.replace(anchor, anchor.replace('    config.validate()?;\n', ''), 1)


def reconstruct(archive, output, variant):
    with tarfile.open(archive) as tree:
        tree.extractall(output, filter='data')
    if variant == 'skip-maintenance-validation':
        path = output / CONFIG
        path.write_text(skip_validation(path.read_text()))
    elif variant != 'original':
        raise ValueError('unknown variant')
    path = output / CHECKPOINT
    path.write_bytes(path.read_bytes() + FORWARD)
    path = output / LIB
    anchor = 'impl From<serde_json::Error> for JournalError {\n    fn from(error: serde_json::Error) -> Self {'
    text = path.read_text()
    if text.count(anchor) != 1:
        raise ValueError('the original JSON error conversion must be unique')
    text = text.replace(anchor, '#[charon::rename(\"JournalErrorFromJson\")]\nimpl From<serde_json::Error> for JournalError {\n    #[charon::rename(\"from_json\")]\n    fn from(error: serde_json::Error) -> Self {', 1)
    path.write_text('#![feature(register_tool)]\n#![register_tool(charon)]\n' + text)


def admit(ir, generated, target):
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    if (raw.get('has_errors') is not False
            or any(options[n] is not False for n in ('skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or options['exclude'] != []
            or any(options[k] != OPTIONS[v] for k, v in (('start_from', 'roots'), ('start_from_if_exists', 'retained_roots'),
                ('include', 'include'), ('opaque', 'opaque')))
            or {r['key'] for r in translated['target_information']} != {target}):
        raise ValueError('the full fixed root/options/target selection is required')
    functions = translated['fun_decls']
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    required = [(ROOT, str(CHECKPOINT)), (EFFECTS_ROOT, str(CHECKPOINT)),
                (POLICY, str(CHECKPOINT)), (POLICY + '::read', str(CONFIG)),
                (POLICY + '::read_using', str(CONFIG))]
    result = {}
    for name, file in required:
        actual = declaration(functions, name)
        rows = [r for r in mapping if r['def_id'] == actual['def_id']]
        if len(rows) != 1 or rows[0]['is_opaque'] or 'Structured' not in actual['body'] or rows[0]['source']['file'] != file:
            raise ValueError('the exact original wrapper or read body must be transparent: ' + name)
        result[name] = actual['def_id']
    actual = method(functions, POLICY, 'validate')
    rows = [r for r in mapping if r['def_id'] == actual['def_id']]
    if (len(rows) != 3 or any(r['is_opaque'] or r['source']['file'] != str(CONFIG) for r in rows)
            or 'Structured' not in actual['body'] or sum('loop' not in r for r in rows) != 1):
        raise ValueError('the complete original maintenance validator and its loop must be transparent')
    result['validate'] = actual['def_id']
    def calls(node):
        if isinstance(node, dict):
            return ([node['Call']['call']] if 'Call' in node else []) + [c for x in node.values() for c in calls(x)]
        if isinstance(node, list):
            return [c for x in node for c in calls(x)]
        return []
    for wrapper, callee in ((ROOT, POLICY), (EFFECTS_ROOT, POLICY.rsplit('::', 1)[0] + '::config_using')):
        selected = calls(declaration(functions, wrapper)['body'])
        if len(selected) != 1 or selected[0]['func']['Regular']['kind']['Fun'] != declaration(functions, callee)['def_id']:
            raise ValueError('each transparent root must forward exactly once to its original method')
    return dict(functions=result, mapped_functions=len(mapping))


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
        report['staged'] = {str(path): sql.digest(source / path) for path in (CONFIG, CHECKPOINT, LIB)}
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
    print(variant, 'complete original maintenance-policy read')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
