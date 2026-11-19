"""Capture the original save and its unchanged Vec row loop in the SQL view."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import source_row_sql_composition as row
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run
from source_save_order import check_adapter, EFFECTS, SAVE_USING
from revision_capture import declaration

SOURCE = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
TREE = '7ca91afd9cd79601d562f3c636066d192fc25fc5'
ROOT = row.CODEC_ROOT + '::save'
NAMESPACE = 'SandproofSourceSaveRows'


ARCHIVE = '7283e7d393201205aa5ed2dc6bbe8887d900f1263dd88fa08eec6bbf5833a6e8'
STAGED = '7809df9a648357a43d525605dc0d2ba36cf29a0b6aed7c956e9d92d0a93c41c5'
WRITE_ROWS = row.CODEC_ROOT + '::write_candidate_records'
CFG = dict(roots=[ROOT], retained_roots=row.CONFIG['retained_roots'], include=row.CONFIG['include'],
           opaque=[*row.CONFIG['opaque'], *(row.CODEC_ROOT + '::' + name for name in (
               'load', 'reserved_records', 'encode_candidate_records', 'retains_saved_records'))])


def admit(ir, generated, target):
    retained, = CFG['retained_roots']
    translated = checked_translation(ir, CFG, target, ROOT, retained)
    functions = translated['fun_decls']
    save, save_using, write_rows, write_row = (
        declaration(functions, name) for name in (ROOT, SAVE_USING, WRITE_ROWS, row.PARENT))
    trait = declaration(translated['trait_decls'], EFFECTS)['def_id']
    adapters = check_adapter(translated, trait, save_using['def_id'], save['body']['Structured']['body'])
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    for item, count in ((save, 1), (save_using, 1), (write_row, 1), (write_rows, 3)):
        retained = [entry for entry in mapping if entry['def_id'] == item['def_id']]
        if (len(retained) != count or item['item_meta']['opacity'] != 'Transparent'
                or any(entry['is_opaque'] or entry['source']['file'] != str(row.CODEC) for entry in retained)):
            raise ValueError('transparent save, owned row loop, and single-row writer required')
    return dict(save=save['def_id'], generic=save_using['def_id'], rows=write_rows['def_id'],
                writer=write_row['def_id'], adapters=adapters)


def capture(out, tools, lock):
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
    codec = row.stage(source, 'core')
    if codec != STAGED:
        raise ValueError('the reviewed SQL staging differs')

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
    report = {'source': SOURCE, 'tree': TREE, 'archive': row.digest(archive),
              'codec_sha256': codec, 'options': CFG, 'driver': row.digest(Path(__file__)), 'commands': [], 'complete': False}
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
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
