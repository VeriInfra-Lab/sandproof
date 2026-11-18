"""Capture the isolated source-row save phases and their actual call order."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

from finish_capture import ARCHIVE, SOURCE, TREE
import prepare_transaction_capture as prepare
from probe_prepare_iterators import GIB, ROOT, digest, run
from revision_capture import declaration, ir_values, regular_calls


HERE = Path(__file__).resolve().parent / 'save-codec'
CODEC = Path('controller-service/src/controller/lifecycle_inbox/source_work/codec.rs')
CODEC_ROOT = prepare.WORK + '::codec'
SAVE = CODEC_ROOT + '::save'
NAMESPACE = 'SandproofSourceCodecSave'
DIGESTS = {
    'original': '87561cb050f40186d20be73b7eeaf91a93be2248d017316fcb886864a062a453',
    'factored': '747cf75d0764d2458e946ccda9d5c724e88b3220e0e7f12b33ceed4edef59405',
    'reordered': '4b2a90247df44b42c8091214f4dd325087d5cc85d30c43aa4f4810d47f422177',
}
PHASES = ('load', 'reserved_records', 'encode_candidate_records', 'retains_saved_records',
          'write_candidate_records')
MODES = ('factored', 'reordered')


def stage(source, mode):
    path = source / CODEC
    if digest(path) != DIGESTS['original'] or mode not in DIGESTS:
        raise ValueError('expected the exact current source codec and a named view')
    patches = {'original': (), 'factored': ('factor',), 'reordered': ('factor', 'reorder')}
    for patch in patches[mode]:
        subprocess.run(['git', 'apply', '--unidiff-zero', str(HERE / (patch + '.patch'))],
                       cwd=source, check=True)
    if digest(path) != DIGESTS[mode]:
        raise ValueError('actual staged source codec differs')
    return DIGESTS[mode]


def configuration():
    return dict(roots=[SAVE], retained_roots=[], include=[SAVE, CODEC_ROOT + '::Sources',
                prepare.CRATE + '::JournalError'], opaque=[prepare.CRATE, 'sandbox_controller_core',
                'sandproof_protocol', 'rusqlite', 'serde_json',
                *(CODEC_ROOT + '::' + name for name in PHASES)])


def check_retention_branch(body, translated, guard, writer, mode):
    guard_call, = (row['call'] for row in ir_values(body, 'Call')
                   if row['call']['func'].get('Regular', {}).get('kind', {}).get('Fun') == guard)
    switches = list(ir_values(body, 'Switch'))
    gate, = (row for row in switches
             if row['data']['scrutinee'] == {'Value': {'Move': guard_call['dest']}})
    values = {selector['Value'][0]: selector['Value'][1]
              for switch in switches for selector, _ in switch['data']['branches']
              if 'Value' in selector}
    (selector, rejected), = gate['data']['branches']
    test = values[selector['Deduplicated']] if 'Deduplicated' in selector else selector['Value'][1]
    passed = gate['data']['fallback']
    if (test[0] != {'Bool': False} or {passed, rejected} != {0, 1}
            or len(gate['branches']) != 2):
        raise ValueError('expected the actual successful and failed old-work check')
    if (list(regular_calls(gate['branches'][rejected]))
            or list(regular_calls(gate['branches'][passed])) != ([] if mode == 'reordered' else [writer])):
        raise ValueError('only the passing retention branch may call the writer in the correct view')

    types = translated['type_decls']
    error = declaration(types, prepare.CRATE + '::JournalError')
    result = declaration(types, 'core::result::Result')
    conflict, = (row['id'] for row in error['kind']['Enum'] if row['name'] == 'Conflict')
    err, = (row['id'] for row in result['kind']['Enum'] if row['name'] == 'Err')
    ((place, error_value), (return_place, result_value)) = ir_values(gate['branches'][rejected], 'Assign')
    error_adt, error_args = error_value['Aggregate']
    result_adt, result_args = result_value['Aggregate']
    if (error_adt['Adt'][0]['id'] != error['def_id'] or error_adt['Adt'][1] != conflict
            or error_args or result_adt['Adt'][0]['id'] != result['def_id'] or result_adt['Adt'][1] != err
            or return_place['kind'] != {'Local': 0} or result_args != [{'Move': place}]):
        raise ValueError('the retention rejection must return the original Conflict')


def check_connection_args(body, ids):
    for phase in ('load', 'reserved_records', 'write_candidate_records'):
        call, = (row['call'] for row in ir_values(body, 'Call')
                 if row['call']['func'].get('Regular', {}).get('kind', {}).get('Fun') == ids[phase])
        argument = call['args'][0]['Move']
        borrowed, = (right['Ref'] for left, right in ir_values(body, 'Assign') if left == argument)
        original, projection = borrowed['place']['kind']['Projection']
        if (borrowed['kind'], original['kind'], projection) != ('Shared', {'Local': 1}, 'Deref'):
            raise ValueError('save must pass the original connection to ' + phase)


def admit(ir, generated, cfg, target, mode):
    translated = prepare.checked_translation(ir, cfg, target, SAVE, None)
    functions = translated['fun_decls']
    save = declaration(functions, SAVE)
    ids = {name: declaration(functions, CODEC_ROOT + '::' + name)['def_id'] for name in PHASES}
    order = (*PHASES[:-2], PHASES[-1], PHASES[-2]) if mode == 'reordered' else PHASES
    body = save['body']['Structured']['body']
    found = [item for item in regular_calls(body) if item in ids.values()]
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    selected, = (row for row in mapping if row['def_id'] == save['def_id'])
    if (save['item_meta']['opacity'] != 'Transparent' or found != [ids[name] for name in order]
            or selected['is_opaque'] is not False or selected['source']['file'] != str(CODEC)):
        raise ValueError('generated save body or validation and update order differs')
    check_retention_branch(body, translated, ids['retains_saved_records'], ids['write_candidate_records'], mode)
    check_connection_args(body, ids)
    return dict(save=save['def_id'], phases=ids, order=order, lean=selected['lean_name'],
                original_connection=True, write_inside_passing_retention_branch=mode == 'factored')


def capture(mode, output, tools, lock):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal' or shutil.disk_usage(ROOT).free < 80 * GIB:
        raise ValueError('proof role and at least 80 GiB of workspace space required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    for label, path in (('charon', charon), ('aeneas', aeneas)):
        if digest(path) != fixed[label]['binary_sha256']:
            raise ValueError('fixed extraction tool differs: ' + label)
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], text=True, cwd=ROOT).strip() != TREE:
        raise ValueError('original integration tree differs')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source_commit=SOURCE, source_tree=TREE, complete=False, commands=[],
                  driver_sha256=digest(Path(__file__)), cpus=cpus, public_acceptance=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=ROOT, stdout=stream, check=True)
        if digest(archive) != ARCHIVE:
            raise ValueError('original integration archive differs')
        with tarfile.open(archive) as contents:
            contents.extractall(source, filter='data')
        report['codec_sha256'] = stage(source, mode)
        cfg = configuration()
        selectors = [item for flag, key in [('--start-from', 'roots'), ('--include', 'include'),
                     ('--opaque', 'opaque')] for value in cfg[key] for item in (flag, value)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            commands = [
                ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                            *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                            '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
                ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                            '-split-files', '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE,
                            '-dest', generated, ir], 120),
            ]
            for label, argv, seconds in commands:
                receipt = run(label, argv, seconds, source, output, env)
                report['commands'].append(receipt)
                if receipt['exit_code'] != 0 or receipt['bounded_stop']:
                    raise ValueError(label + ' did not complete')
                if label == 'charon':
                    report['ir_sha256'] = digest(ir)
        if digest(ir) != report['ir_sha256']:
            raise ValueError('original source save IR changed')
        report['admission'] = admit(ir, generated, cfg, fixed['target'], mode)
        report['generated_sha256'] = {str(path.relative_to(generated)): digest(path)
                                      for path in sorted(generated.rglob('*')) if path.is_file()}
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(mode, report['codec_sha256'], report['admission'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=MODES, required=True)
    for name in ('output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
