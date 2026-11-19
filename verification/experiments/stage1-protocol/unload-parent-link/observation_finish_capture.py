"""Capture the original observation caller and its record-returning save tail."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import finish_capture as finish
from frontier_capture import call_kind, indexed
import prepare_transaction_capture as prepare
from probe_prepare_iterators import GIB, ROOT, digest, run
from revision_capture import declaration, ir_values, regular_calls


SOURCE = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
TREE = '7ca91afd9cd79601d562f3c636066d192fc25fc5'
ARCHIVE = '7283e7d393201205aa5ed2dc6bbe8887d900f1263dd88fa08eec6bbf5833a6e8'
METHOD = 'save_source_observation'
OBSERVE = prepare.WORK + '::finish_observation_save'
PARENT = prepare.CONTROLLER + '::ControllerJournal::' + METHOD
RETAIN = prepare.WORK + '::*::' + METHOD
MODES = ('parent', 'tail', 'reordered')
WRAPPER = '''fn finish_observation_save<Rows, Save: SaveSourceInputs<Rows>, C: CrashInjector, Record>(
    tx: super::WorldTransaction<'_>,
    sources: &Rows,
    max_sessions: u16,
    crashes: &mut C,
    record: Record,
) -> Result<Record, JournalError> {
    finish_source_work::<Rows, Save, C>(tx, sources, max_sessions, crashes)?;
    Ok(record)
}

'''
CALL = '''        finish_observation_save::<_, SaveSourceInputsToJournal, _, _>(
            tx, &sources, inbox.config.max_sessions, crashes, record,
        )
'''


def factor(text, *, reordered=False):
    method = '    pub(crate) fn ' + METHOD
    original = finish.SAVE + finish.BEFORE + finish.COMMIT + finish.AFTER + '        Ok(record)\n'
    insertion = 'impl ControllerJournal {'
    head, body = text.split(method)
    if head.count(insertion) != 1 or body.count(original) != 1:
        raise ValueError('the exact original observation save and returned record are required')
    prepared = finish.factor(text, reordered=reordered)
    helper = finish.HEADER + prepared.split(finish.HEADER, 1)[1].split('fn encode_prepared_source_work', 1)[0]
    return head.replace(insertion, helper + WRAPPER + insertion) + method + body.replace(original, CALL)


def configuration(mode):
    cfg = finish.configuration(mode)
    if mode == 'parent':
        cfg['roots'] = [PARENT]
        cfg['retained_roots'] = [RETAIN]
        cfg['include'] = [RETAIN if item == prepare.RETAIN else item for item in cfg['include']]
        cfg['include'].append(prepare.CRATE + '::capture::source::RawSourceObservation')
        cfg['namespace'] = cfg['subdir'] = 'SandproofSourceObservationParent'
    else:
        cfg['roots'] = [OBSERVE]
    cfg['include'].append(OBSERVE)
    return cfg


def local(place):
    return place['kind']['Local']


def origin(place, bindings, anchors):
    """Resolve the root and field path through temporaries, reborrows and derefs."""
    kind = place['kind']
    if 'Projection' in kind:
        base, projection = kind['Projection']
        root, *fields = origin(base, bindings, anchors)
        return (root, *fields) if projection == 'Deref' else (root, *fields, projection['Field'][1])
    root = kind['Local']
    return origin(bindings[root], bindings, anchors) if root in bindings and root not in anchors else (root,)


def arm_condition(switch, arm, values, variants):
    data = switch['data']
    tag, place = next(iter(data['scrutinee'].items()))
    matched = []
    for operand, index in data['branches']:
        if 'Value' in operand:
            identifier, payload = operand['Value']
            values[identifier] = payload[0]
        else:
            payload = [values[operand['Deduplicated']]]
        if arm == index:
            matched.append(payload[0])
    if tag == 'Discriminant':
        literal, = matched
        enum, variant = literal['Discriminant']
        return local(place), variants[enum['id']][variant]
    if not matched:
        if data['fallback'] != arm or len(data['branches']) != 1:
            raise ValueError('the original Boolean guard must have both arms')
        operand = data['branches'][0][0]
        literal = values[operand['Value'][0] if 'Value' in operand else operand['Deduplicated']]
        return local(place['Move']), not literal['Bool']
    literal, = matched
    return local(place['Move']), literal['Bool']


def routes(block, target, variants):
    """Locate selected call sites on ordinary paths, without traversing unwind bodies."""
    values = {}

    def visit(block, steps, guards):
        steps = list(steps)
        for row in block['statements']:
            step = row['kind']
            if not isinstance(step, dict):
                continue
            if 'Call' in step and call_kind(step['Call']['call']).get('Fun') == target:
                yield step['Call']['call'], steps, guards
            if 'Switch' in step:
                switch = step['Switch']
                for arm, child in enumerate(switch['branches']):
                    condition = arm_condition(switch, arm, values, variants)
                    yield from visit(child, steps, (*guards, condition))
            else:
                steps.append(step)

    return list(visit(block, [], ()))


def parent_link(translated):
    fns = translated['fun_decls']
    function = prepare.method(fns, prepare.WORK, METHOD)
    wrapper = declaration(fns, OBSERVE)['def_id']
    save = declaration(fns, prepare.WORK + '::codec::save')['def_id']
    trait = declaration(translated['trait_decls'], prepare.WORK + '::SaveSourceInputs')['def_id']
    begin = prepare.method(fns, prepare.TRANSACTION, 'begin')['def_id']
    world = prepare.method(fns, prepare.TRANSACTION, 'world')['def_id']
    commit = prepare.method(fns, prepare.TRANSACTION, 'commit')['def_id']
    load_inbox = declaration(fns, prepare.INBOX + '::codec::load')['def_id']
    load_sources = declaration(fns, prepare.WORK + '::codec::load')['def_id']
    current = declaration(fns, prepare.WORK + '::snapshot::current')['def_id']
    control = declaration(translated['type_decls'], 'core::ops::control_flow::ControlFlow')
    variants = {control['def_id']: {row['id']: row['name'] for row in control['kind']['Enum']}}
    structured = function['body']['Structured']
    body = structured['body']
    if function['item_meta']['opacity'] != 'Transparent' or any(call in regular_calls(body) for call in (save, commit)):
        raise ValueError('the real observation caller must be transparent and delegate the commit tail')
    (call, steps, guards), = routes(body, wrapper, variants)
    if [value for _, value in guards] != [True, *(['Continue'] * 7), False, 'Continue', False, False]:
        raise ValueError('only the original successful prefix and fresh record may reach the writer')
    names = {row['name']: row['index'] for row in structured['locals']['locals'] if row['name'] is not None}
    tx, sources, inbox, crashes, record = (names[key] for key in ('tx', 'sources', 'inbox', 'crashes', 'record'))
    anchors = {tx, sources, inbox, crashes, record}
    bindings = {}
    calls = {}
    by_destination = {}
    for step in steps:
        if 'Assign' in step:
            place, value = step['Assign']
            if 'Local' not in place['kind']:
                continue
            operand = value['Use'][0] if 'Use' in value else {}
            source = value.get('Ref', {}).get('place') or operand.get('Move') or operand.get('Copy')
            if source is not None:
                bindings[local(place)] = source
        elif 'Call' in step:
            invoked = step['Call']['call']
            identifier = call_kind(invoked).get('Fun')
            if identifier is None:
                continue
            calls.setdefault(identifier, []).append(invoked)
            destination = local(invoked['dest'])
            name = indexed(fns, identifier)['item_meta']['name'][-1].get('Ident', [''])[0]
            by_destination[destination] = (name, invoked)
            if name in ('deref', 'deref_mut'):
                bindings[destination] = invoked['args'][0]['Move']
    arguments = [origin(value['Move'], bindings, anchors) for value in call['args']]
    if arguments != [(tx,), (sources,), (inbox, 0, 3), (crashes,), (record,)] or local(call['dest']) != 0:
        raise ValueError('the same opened owner, rows, inbox capacity, hooks and cloned record must be returned')
    for identifier in (world, load_inbox, load_sources, current):
        invoked, = calls[identifier]
        if origin(invoked['args'][0]['Move'], bindings, anchors) != (tx,):
            raise ValueError('the original transaction must supply the loaders, World and current source')
    began, = calls[begin]
    begin_gate, = [key for key, _ in guards if key in by_destination
                   and by_destination[key][1]['args'] == [{'Move': began['dest']}]]
    if origin(bindings[tx], bindings, anchors - {tx}) != (begin_gate, 0):
        raise ValueError('the finish owner must be the actual successful original Begin result')
    iter_result, = [invoked['dest'] for name, invoked in by_destination.values()
                    if name == 'iter_mut' and origin(invoked['args'][0]['Move'], bindings, anchors) == (sources, 3)]
    found, = [invoked for name, invoked in by_destination.values()
              if name == 'find' and origin(invoked['args'][0]['Move'], bindings, anchors) == (local(iter_result),)]
    selected, = [invoked for name, invoked in by_destination.values()
                 if name == 'ok_or' and invoked['args'][0] == {'Move': found['dest']}]
    selected_gate, = [key for key, _ in guards if key in by_destination
                      and by_destination[key][1]['args'] == [{'Move': selected['dest']}]]
    name, clone = by_destination[record]
    clone_root = origin(clone['args'][0]['Move'], bindings, anchors)
    if clone_root != (selected_gate, 0):
        raise ValueError('the cloned result must come from the original selected mutable row')
    writes = [(place, value) for step in steps if 'Assign' in step
              for place, value in [step['Assign']] if 'Projection' in place['kind']]
    (place, value), = [(place, value) for place, value in writes
                       if origin(place, bindings, anchors) == (*clone_root, 1)]
    packed = local(value['Use'][0]['Move'])
    aggregate, = [value['Aggregate'] for step in steps if 'Assign' in step
                  for destination, value in [step['Assign']]
                  if destination['kind'] == {'Local': packed}]
    some = declaration(translated['type_decls'], 'core::option::Option')
    some_id, = [row['id'] for row in some['kind']['Enum'] if row['name'] == 'Some']
    adt = aggregate[0]['Adt']
    if (name != 'clone' or adt[0]['id'] != some['def_id'] or adt[1] != some_id
            or [origin(arg['Move'], bindings, anchors) for arg in aggregate[1]] != [(names['observation'],)]):
        raise ValueError('the original observation must be written before cloning the returned record')
    dictionary = call['func']['Regular']['generics']['trait_refs'][0]['Value'][1]
    impl = indexed(translated['trait_impls'], dictionary['kind']['TraitImpl']['id'])
    method, = impl['methods']
    adapter = indexed(fns, method['skip_binder']['id'])
    if (method['kind'] != {'TraitMethod': [trait, 0]} or adapter['item_meta']['opacity'] != 'Transparent'
            or list(regular_calls(adapter['body']['Structured']['body'])) != [save]):
        raise ValueError('the concrete dictionary must forward to the original source save')
    return dict(parent=function['def_id'], wrapper=wrapper, begin=begin, save=save,
                fields=list(map(list, arguments)), guard_values=[value for _, value in guards])


def admit(ir, generated, cfg, target, mode):
    root = PARENT if mode == 'parent' else OBSERVE
    translated = prepare.checked_translation(ir, cfg, target, root, RETAIN if mode == 'parent' else None)
    if mode == 'parent':
        return parent_link(translated)
    fns = translated['fun_decls']
    helper = declaration(fns, finish.HELPER)
    commit = prepare.method(fns, prepare.TRANSACTION, 'commit')['def_id']
    save_trait = declaration(translated['trait_decls'], prepare.WORK + '::SaveSourceInputs')['def_id']
    crash_trait = declaration(translated['trait_decls'], prepare.CRATE + '::CrashInjector')['def_id']
    references = {value[0]: value[1] for value in ir_values(translated, 'Value')
                  if isinstance(value, list) and len(value) == 2
                  and isinstance(value[1], dict) and 'trait_decl_ref' in value[1]}
    effects = []
    for invocation in ir_values(helper['body']['Structured']['body'], 'Call'):
        kind = call_kind(invocation['call'])
        if 'Trait' in kind:
            reference, method = kind['Trait']
            payload = reference['Value'][1] if 'Value' in reference else references[reference['Deduplicated']]
            trait_id = payload['trait_decl_ref']['skip_binder']['id']
            free = 0 if trait_id == save_trait else 1
            if (method != 0 or trait_id not in (save_trait, crash_trait)
                    or payload['kind'] != {'Clause': {'Free': free}}):
                raise ValueError('the original save and crash dictionaries are required')
            effects.append(trait_id)
        elif kind.get('Fun') == commit:
            effects.append(commit)
    expected = [save_trait, crash_trait, crash_trait, commit] if mode == 'reordered' else [
        save_trait, crash_trait, commit, crash_trait]
    if effects != expected:
        raise ValueError('the actual save, hooks and commit order must match the selected mode')
    wrapper = declaration(fns, OBSERVE)
    body = wrapper['body']['Structured']['body']
    callback, = [row['call'] for row in ir_values(body, 'Call')
                 if call_kind(row['call']).get('Fun') == helper['def_id']]
    if len(callback['args']) != 4:
        raise ValueError('the exact original save tail invocation is required')
    functions = json.loads((generated / 'translation.json').read_bytes())['functions']
    names = {}
    for label, function in (('helper', helper), ('wrapper', wrapper)):
        mapping, = [row for row in functions if row['def_id'] == function['def_id']]
        if (function['item_meta']['opacity'] != 'Transparent' or mapping['is_opaque'] is not False
                or mapping['source']['file'] != prepare.SOURCE):
            raise ValueError('both actual generated observation tail definitions are required')
        names[label] = mapping['lean_name']
    return dict(helper=helper['def_id'], wrapper=wrapper['def_id'], commit=commit, names=names)


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
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], cwd=ROOT, text=True).strip() != TREE:
        raise ValueError('the exact integration tree is required')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source_commit=SOURCE, source_tree=TREE, commands=[], complete=False,
                  driver_sha256=digest(Path(__file__)), cpus=cpus)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as handle:
            subprocess.run(['git', 'archive', SOURCE], cwd=ROOT, stdout=handle, check=True)
        if digest(archive) != ARCHIVE:
            raise ValueError('the fixed source archive differs')
        with tarfile.open(archive) as contents:
            contents.extractall(source, filter='data')
        for name, expected in prepare.SOURCE_INPUTS.items():
            if digest(source / name) != expected:
                raise ValueError('the original source input differs: ' + name)
        path = source / prepare.SOURCE
        path.write_text(factor(path.read_text(), reordered=mode == 'reordered'))
        report['candidate_source_sha256'] = digest(path)
        cfg = configuration(mode)
        selectors = [argument for flag, key in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                                  ('--include', 'include'), ('--opaque', 'opaque'))
                     for value in cfg[key] for argument in (flag, value)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                                *selectors, '--dest-file', ir, '--', '--manifest-path',
                                'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']], 240)]
        if mode != 'parent':
            commands.append(('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                                       '-split-files', '-emit-json', '-namespace', cfg['namespace'],
                                       '-subdir', cfg['subdir'], '-dest', generated, ir], 120))
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
            raise ValueError('the same captured observation IR is required')
        report['admission'] = admit(ir, generated, cfg, fixed['target'], mode)
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
