"""Capture one real source writer with the exact SELECT and UPDATE candidates."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

from frontier_capture import indexed
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run
from revision_capture import declaration
from source_row_sql_select import CONFIG as SELECT_CONFIG, PROBE as SELECT_PROBE, stage as stage_select
from source_row_sql_update import CONFIG as UPDATE_CONFIG
from source_row_write import ARCHIVE, CODEC, CODEC_ROOT, CORE, PARENT, ROOT, SOURCE, TREE, digest, parent_operands
from stage_row_sql_update import HERE as UPDATE, STATEMENT


MODES = ('core', 'greatest-vacancy')
NAMESPACE = 'SandproofSourceRowSqlSelect'
SHA = 'sandproof_controller_service::controller::lifecycle_inbox::sha256::digest'
CONFIG = dict(roots=[PARENT], retained_roots=[SELECT_PROBE],
    include=list(dict.fromkeys([*SELECT_CONFIG['include'], *UPDATE_CONFIG['include']])),
    opaque=[*(item for item in SELECT_CONFIG['opaque'] if not item.endswith('::ValueRef')), SHA])


def stage(source, mode):
    if mode not in MODES:
        raise ValueError('unknown composed source row writer')
    stage_select(source, mode)
    path = source / CODEC
    text = path.read_text()
    start = text.index('    fn update(\n        connection: &Connection,')
    stop = text.index('\n    fn capacity()', start)
    first = text[start:stop]
    four = (UPDATE / 'adapter.rs.in').read_text()
    if (first.count(STATEMENT) != 1 or four.count(STATEMENT) != 1
            or first.count('params![bytes, sha.as_slice(), key.as_slice(), id]') != 1):
        raise ValueError('the original update and checked four-parameter adapter are required')
    original_sha = '    fn digest(bytes: &[u8]) -> [u8; 32] {\n        Sha256::digest(bytes).into()\n    }'
    shared_sha = original_sha.replace('Sha256::digest(bytes).into()', 'super::super::sha256::digest(bytes)')
    if text.count(original_sha) != 1:
        raise ValueError('expected a sole direct original SHA implementation')
    leaf = path.parents[1] / 'sha256.rs'
    if leaf.read_text().count('Sha256::digest(bytes).into()') != 1:
        raise ValueError('expected the existing adopted SHA-256 primitive')
    path.write_text((text[:start] + four + text[stop:]).replace(original_sha, shared_sha, 1))
    return digest(path)


def admit(ir, generated, target):
    translated = checked_translation(ir, CONFIG, target, PARENT, SELECT_PROBE)
    functions = translated['fun_decls']
    parent, core = (declaration(functions, name) for name in (PARENT, CORE))
    trait = declaration(translated['trait_decls'], CODEC_ROOT + '::SourceRowEffects')['def_id']
    adapters = parent_operands(translated, parent, core['def_id'], trait)
    all_names = {item['def_id']: item for item in json.loads((generated / 'translation.json').read_bytes())['functions']}
    business = [parent, core, *(indexed(functions, identifier) for identifier in adapters.values())]
    for function in business:
        view = all_names[function['def_id']]
        if function['item_meta']['opacity'] != 'Transparent' or view['is_opaque'] or view['source']['file'] != str(CODEC):
            raise ValueError('a real selected source row writer/adaptor did not stay transparent')
    sha = declaration(functions, SHA)
    if all_names[sha['def_id']]['is_opaque'] is not True:
        raise ValueError('the existing A9 provider boundary must retain its exact signature and origin')
    return dict(parent=parent['def_id'], core=core['def_id'], adapters=adapters, sha=sha['def_id'])


def capture(mode, output, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or shutil.disk_usage(output.parent).free < 80 * GIB or shutil.disk_usage(ROOT).free < 64 * GIB):
        raise ValueError('formal role and original capture budget required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(digest(binary) != fixed[name]['binary_sha256'] for name, binary in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('fixed capture tools required')
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
            raise ValueError('fixed source archive differs')
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        report['codec_sha256'] = stage(source, mode)
        ir, generated = output / 'actual.llbc', output / 'generated'
        selectors = [part for flag, group in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                              ('--include', 'include'), ('--opaque', 'opaque'))
                     for pattern in CONFIG[group] for part in (flag, pattern)]
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        *selectors, '--dest-file', ir, '--', '--manifest-path',
                        'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for label, argv, seconds in commands:
                item = run(label, argv, seconds, source, output, env)
                report['commands'].append(item)
                if item['exit_code'] != 0 or item['bounded_stop']:
                    raise ValueError(label + ' did not complete')
                if label == 'charon':
                    report['ir_sha256'] = digest(ir)
        if digest(ir) != report['ir_sha256']:
            raise ValueError('the actual combined IR changed after capture')
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
