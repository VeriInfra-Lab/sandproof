"""Capture the observation slot search, decision, and their original parent."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

from frontier_capture import call_kind, indexed
import observation_finish_capture as finish
from prepare_transaction_capture import WORK, checked_translation
from probe_prepare_iterators import GIB, ROOT, digest, run
from revision_capture import declaration, ir_values, regular_calls


HERE = Path(__file__).resolve().parent / 'observation-slot'
NAMESPACE = 'SandproofObservationSlot'
FINDER = WORK + '::observation_slot_using'
DECISION = WORK + '::observation_slot_decision'
TRAIT = WORK + '::ObservationWork'
DECISION_TYPE = WORK + '::ObservationSlotDecision'
MODES = ('core', 'binding-only', 'parent')
ORIGINAL_FIND = '''        let record = sources
            .records
            .iter_mut()
            .find(|r| r.work == *work)
            .ok_or(JournalError::Conflict)?;
'''
ORIGINAL_GUARD = '''        if record
            .observation
            .as_ref()
            .is_some_and(|old| old != &observation)
        {
            return Err(JournalError::Conflict);
        }
        if record.observation.is_some() {
            return Ok(record.clone());
        }
'''
SELECT = '''        let index = observation_slot_using::<_, _, JournalObservationWork>(&sources.records, work)
            .ok_or(JournalError::Conflict)?;
        let record = &mut sources.records[index];
'''
DECIDE = '''        match observation_slot_decision(&record.observation, &observation) {
            ObservationSlotDecision::Conflict => return Err(JournalError::Conflict),
            ObservationSlotDecision::Replay => return Ok(record.clone()),
            ObservationSlotDecision::Fresh => (),
        }
'''


def factor(text, mode):
    if mode not in MODES:
        raise ValueError('unknown observation slot mode')
    text = finish.factor(text)
    finder = (HERE / 'find.rs.in').read_text() + '\n\n'
    if mode == 'binding-only':
        call = 'Check::same_work(&records[index], work)'
        if finder.count(call) != 1:
            raise ValueError('one actual complete comparison expected')
        finder = finder.replace(call, call.replace('same_work', 'same_binding'))
    opening = '    pub(crate) fn ' + finish.METHOD
    prefix, parent = text.split(opening)
    marker = 'impl ControllerJournal {'
    if any(parent.count(part) != 1 for part in (ORIGINAL_FIND, ORIGINAL_GUARD)) or prefix.count(marker) != 1:
        raise ValueError('the exact original slot lookup and duplicate guard are required')
    return (prefix.replace(marker, finder + marker) + opening
            + parent.replace(ORIGINAL_FIND, SELECT).replace(ORIGINAL_GUARD, DECIDE))


def configuration(mode):
    transparent = [FINDER, DECISION, TRAIT, DECISION_TYPE]
    if mode == 'parent':
        cfg = finish.configuration(mode)
        cfg['include'].extend([*transparent, WORK + '::*::same_work'])
    else:
        cfg = dict(roots=[FINDER, DECISION], retained_roots=[], include=transparent,
                   opaque=finish.configuration('tail')['opaque'])
    return dict(cfg, namespace=NAMESPACE, subdir=NAMESPACE)


def check_parent(translated):
    fns = translated['fun_decls']
    parent = finish.prepare.method(fns, WORK, finish.METHOD)
    search = declaration(fns, FINDER)
    guard = declaration(fns, DECISION)
    tail = declaration(fns, finish.OBSERVE)['def_id']
    body = parent['body']['Structured']['body']
    selected = [identifier for identifier in regular_calls(body) if identifier in (
        search['def_id'], guard['def_id'], tail)]
    if selected != [search['def_id'], guard['def_id'], tail]:
        raise ValueError('the actual parent must use one scan, guard and finish')
    call, = [row['call'] for row in ir_values(body, 'Call')
             if call_kind(row['call']).get('Fun') == search['def_id']]
    reference, = call['func']['Regular']['generics']['trait_refs']
    instance = indexed(translated['trait_impls'], reference['Value'][1]['kind']['TraitImpl']['id'])
    method, = [row for row in instance['methods'] if row['kind']['TraitMethod'][1] == 0]
    actual = indexed(fns, method['skip_binder']['id'])
    member = actual['item_meta']['name'][-1]['Ident'][0]
    if member != 'same_work' or actual['item_meta']['opacity'] != 'Transparent':
        raise ValueError('the parent must use the actual complete-comparison dictionary')
    return dict(parent=parent['def_id'], lookup=search['def_id'], guard=guard['def_id'], tail=tail,
                comparison=actual['def_id'])


def admit(ir, generated, target, mode):
    cfg = configuration(mode)
    if mode == 'parent':
        translated = checked_translation(ir, cfg, target, finish.PARENT, finish.RETAIN)
        return check_parent(translated)
    # The prior capture verifier is for one root. Check both retained transparent roots here.
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    if (raw.get('has_errors') is not False or options['start_from'] != cfg['roots']
            or options['start_from_if_exists'] != [] or options['include'] != cfg['include']
            or options['exclude'] != [] or options['opaque'] != cfg['opaque']
            or any(options[key] is not False for key in (
                'skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or {row['key'] for row in translated['target_information']} != {target}):
        raise ValueError('actual isolated lookup capture options differ')
    expected = {name: declaration(translated['fun_decls'], name)['def_id'] for name in (FINDER, DECISION)}
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    retained = {}
    for name, identifier in expected.items():
        rows = [row for row in mapping if row['def_id'] == identifier]
        if (len(rows) != (3 if name == FINDER else 1)
                or any(row['source']['file'] != finish.prepare.SOURCE or row['is_opaque'] is not False for row in rows)):
            raise ValueError('all actual slot functions must be translated transparently')
        retained[name] = sorted(row['lean_name'] for row in rows)
    return retained


def capture(mode, output, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or min(shutil.disk_usage(ROOT).free, shutil.disk_usage(output.parent).free) < 80 * GIB):
        raise ValueError('formal role and the original capture budget are required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(digest(binary) != fixed[name]['binary_sha256'] for name, binary in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('the same fixed extractors are required')
    if subprocess.check_output(['git', 'rev-parse', finish.SOURCE + '^{tree}'], cwd=ROOT, text=True).strip() != finish.TREE:
        raise ValueError('the exact integration tree is required')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source_commit=finish.SOURCE, source_tree=finish.TREE, commands=[], complete=False,
                  driver_sha256=digest(Path(__file__)), candidate_sha256=digest(HERE / 'find.rs.in'), cpus=cpus)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as handle:
            subprocess.run(['git', 'archive', finish.SOURCE], cwd=ROOT, stdout=handle, check=True)
        if digest(archive) != finish.ARCHIVE:
            raise ValueError('the fixed source archive differs')
        with tarfile.open(archive) as contents:
            contents.extractall(source, filter='data')
        for name, expected in finish.prepare.SOURCE_INPUTS.items():
            if digest(source / name) != expected:
                raise ValueError('the original source input differs: ' + name)
        path = source / finish.prepare.SOURCE
        path.write_text(factor(path.read_text(), mode))
        report['candidate_source_sha256'] = digest(path)
        cfg = configuration(mode)
        selectors = [arg for flag, key in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                          ('--include', 'include'), ('--opaque', 'opaque'))
                     for value in cfg[key] for arg in (flag, value)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                        '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for label, command, limit in commands:
                result = run(label, command, limit, source, output, env)
                report['commands'].append(result)
                if result['exit_code'] != 0 or result['bounded_stop']:
                    raise ValueError(label + ' did not complete')
                if label == 'charon':
                    report['ir_sha256'] = digest(ir)
        if digest(ir) != report['ir_sha256']:
            raise ValueError('the same captured slot IR is required')
        report['admission'] = admit(ir, generated, fixed['target'], mode)
        report['generated_sha256'] = {str(file.relative_to(generated)): digest(file)
                                      for file in sorted(generated.rglob('*')) if file.is_file()}
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(mode, report['admission'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', required=True, choices=MODES)
    for name in ('output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
