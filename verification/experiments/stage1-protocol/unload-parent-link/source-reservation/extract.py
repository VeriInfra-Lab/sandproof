"""Capture the original source reservation arithmetic without source adapters."""
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
from prepare_transaction_capture import checked_translation, method
from source_record_bounds_capture import SOURCE_CONFIG, RETIREMENT
from source_save_row_list_capture import ARCHIVE, SOURCE, TREE
import source_row_sql_composition as sql
from probe_prepare_iterators import GIB, run

CONFIG = Path('controller-service/src/capture/source.rs')
ROOT = SOURCE_CONFIG + '::*::required_bytes'
NAMESPACE = 'SandproofSourceReservation'
VARIANTS = ('original', 'omit-release-budget')
RELEASE = RETIREMENT + '::MATERIAL_RELEASE_RECORD_MAX_BYTES'
OPTIONS = dict(roots=[ROOT], retained_roots=[RELEASE], include=[
    SOURCE_CONFIG + '::SourceObservationConfig', SOURCE_CONFIG + '::*::required_bytes',
    *(RETIREMENT + '::MATERIAL_' + name + '_MAX_BYTES'
      for name in ('RETIREMENT_PLAN', 'RETIREMENT_RECORD', 'RELEASE_RECORD')),
], opaque=['sandproof_controller_service', 'sandbox_controller_core', 'sandbox_protocol'])


def omit_release(text):
    anchor = '+ super::MATERIAL_RELEASE_RECORD_MAX_BYTES)'
    if text.count(anchor) != 1:
        raise ValueError('the sole original release allowance is required')
    return text.replace(anchor, '+ 0)', 1)


def reconstruct(archive, output, variant):
    with tarfile.open(archive) as tree:
        tree.extractall(output, filter='data')
    if variant == 'omit-release-budget':
        path = output / CONFIG
        path.write_text(omit_release(path.read_text()))
    return (output / CONFIG).read_bytes()


def admit(ir, generated, target):
    translated = checked_translation(ir, OPTIONS, target, ROOT, RELEASE)
    actual = method(translated['fun_decls'], SOURCE_CONFIG, 'required_bytes')
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    rows = [r for r in mapping if r['def_id'] == actual['def_id']]
    if (len(rows) != 1 or rows[0]['is_opaque'] or 'Structured' not in actual['body']
            or rows[0]['source']['file'] != str(CONFIG)):
        raise ValueError('the complete original arithmetic body must be transparent')
    if any(r['is_opaque'] for r in mapping):
        raise ValueError('this arithmetic root requires no opaque function')
    return dict(required_bytes=actual['def_id'], mapped_functions=len(mapping))


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
        report['staged'] = {str(CONFIG): sql.digest(source / CONFIG)}
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
    print(variant, 'complete original reservation arithmetic')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
