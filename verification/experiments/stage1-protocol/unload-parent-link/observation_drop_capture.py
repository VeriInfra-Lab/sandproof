"""Capture original live Drop and a rollback-error stale-memo variant."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import observation_sql_save_capture as joint
import source_row_sql_composition as row
from check_observation_drop import check_drop, DROP, MEMO, TRANSACTION
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run

SOURCE, TREE, ARCHIVE, STAGED, WORK = joint.SOURCE, joint.TREE, joint.ARCHIVE, joint.STAGED, joint.WORK
NAMESPACE = 'SandproofObservationDrop'
MEMO_PATH = Path('controller-service/src/controller/transaction/memo.rs')
MEMO_SHA = '48313cde23179019d42279cca51f7c26083ee41b2dbc5c314f3a4fc5296c594d'
MODES = ('core', 'keep-cache')
CFG = {**joint.CFG, 'include': [*joint.CFG['include'], DROP, MEMO,
    *(TRANSACTION + '::' + name for name in (
        'TransactionCells', 'CommittedWorld', 'memo::Cells', '*::set_committed',
        '*::take_candidate', '*::call_once'))]}


def memo_source(data, mode):
    if mode not in MODES:
        raise ValueError('unknown rollback mutation')
    needle = b'        cells.set_committed(None);'
    if data.count(needle) != 1:
        raise ValueError('only the original rollback error invalidation may be removed')
    return data if mode == 'core' else data.replace(needle, b'        let _ = cells;')


def admit(ir, generated, target):
    retained, = CFG['retained_roots']
    translated = checked_translation(ir, CFG, target, CFG['roots'][0], retained)
    relation = check_drop(translated)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    for label in ('drop', 'core', 'callback', 'cell_store'):
        (captured,) = [item for item in mapping if item['def_id'] == relation[label]]
        if captured['is_opaque']:
            raise ValueError('actual rollback implementation disappeared: ' + label)
    return relation


def capture(out, tools, lock, mode):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or min(shutil.disk_usage(path).free for path in (out.parent, row.ROOT)) < 80 * GIB):
        raise ValueError('original start budget is required')
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.umask(0o077)
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    fixed = json.loads((row.ROOT / 'verification/foundations-environment.json').read_bytes())
    for name, binary in (('charon', charon), ('aeneas', aeneas)):
        if row.digest(binary) != fixed[name]['binary_sha256']:
            raise ValueError('fixed tools required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], cwd=row.ROOT, text=True).strip() != TREE:
        raise ValueError('fixed original source tree required')
    out.mkdir()
    source = out / 'source'
    source.mkdir()
    archive = out / 'source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', SOURCE], cwd=row.ROOT, stdout=stream, check=True)
    if row.digest(archive) != ARCHIVE:
        raise ValueError('the original source archive differs')
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    for name, original in joint.slot.finish.prepare.SOURCE_INPUTS.items():
        if row.digest(source / name) != original:
            raise ValueError('the original parent input differs: ' + name)
    codec = row.stage(source, 'core')
    if codec != STAGED:
        raise ValueError('the reviewed SQL staging differs')
    method = source / WORK
    method.write_text(joint.slot.factor(method.read_text(), 'parent'))
    memo = source / MEMO_PATH
    if row.digest(memo) != MEMO_SHA:
        raise ValueError('the adopted original rollback source differs')
    memo.write_bytes(memo_source(memo.read_bytes(), mode))
    selectors = [part for flag, group in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                          ('--include', 'include'), ('--opaque', 'opaque'))
                 for pattern in CFG[group] for part in (flag, pattern)]
    ir, generated = out / 'actual.llbc', out / 'generated'
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
               CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(out / 'target'))
    for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(key, None)
    commands = [
        ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                    *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                    '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
        ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                     '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
    ]
    report = {'mode': mode, 'source': SOURCE, 'memo': row.digest(memo), 'tree': TREE, 'archive': row.digest(archive),
              'codec_sha256': codec, 'work_sha256': row.digest(method), 'options': CFG, 'driver': row.digest(Path(__file__)), 'commands': [], 'complete': False}
    try:
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, command, limit in commands:
                item = run(name, command, limit, source, out, env)
                report['commands'].append(item)
                if item['exit_code'] or item['bounded_stop']:
                    raise ValueError(name + ' did not complete')
                if name == 'charon':
                    report['ir_sha256'] = row.digest(ir)
        report['generated_sha256'] = {str(p.relative_to(generated)): row.digest(p)
                                      for p in generated.rglob('*') if p.is_file()}
        report['admission'] = admit(ir, generated, fixed['target'])
        report['complete'] = True
    finally:
        (out / 'result.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    cli.add_argument('--mode', required=True, choices=MODES)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.mode)
