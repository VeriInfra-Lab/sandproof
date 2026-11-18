"""Capture the original first-Unload save and commit caller in a disposable archive."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import frontier_capture as frontier
import prepare_transaction_capture as prepare
from probe_prepare_iterators import GIB, REQUEST_CARRIERS, ROOT, digest, rewrite, run
from revision_capture import declaration, ir_values, regular_calls


SOURCE = '7b27876458a7212feb98a2aaf36560fe6443fb73'
TREE = '630ae844a52d4b6f2ab83ac41a6737e416e7b76c'
ARCHIVE = '78516a6e3b1adc7b1c87c7c01ac3e4a90eb7752d99f78a2d6b5fe790fbf0b4c7'
HELPER = prepare.WORK + '::finish_source_work'
MODULES = {'tail': 'SandproofUnloadFinish', 'parent': 'SandproofUnloadFinishParent',
           'reordered': 'SandproofUnloadFinish'}
SAVE = '        codec::save(&tx, &sources, inbox.config.max_sessions)?;\n'
BEFORE = '        crashes.hit(CrashPoint::BeforeCommit)?;\n'
COMMIT = '        tx.commit()?;\n'
AFTER = '        crashes.hit(CrashPoint::AfterCommit)?;\n'
HEADER = '''trait SaveSourceInputs<Rows> {
    fn save(connection: &rusqlite::Connection, sources: &Rows, max_sessions: u16)
        -> Result<(), JournalError>;
}

struct SaveSourceInputsToJournal;

impl SaveSourceInputs<codec::Sources> for SaveSourceInputsToJournal {
    fn save(
        connection: &rusqlite::Connection,
        sources: &codec::Sources,
        max_sessions: u16,
    ) -> Result<(), JournalError> {
        codec::save(connection, sources, max_sessions)
    }
}

fn finish_source_work<Rows, Save: SaveSourceInputs<Rows>, C: CrashInjector>(
    tx: super::WorldTransaction<'_>,
    sources: &Rows,
    max_sessions: u16,
    crashes: &mut C,
) -> Result<(), JournalError> {
'''
CALL = '''        finish_source_work::<_, SaveSourceInputsToJournal, _>(
            tx, &sources, inbox.config.max_sessions, crashes,
        )?;
'''


def factor(text, *, reordered=False):
    text = frontier.factor(rewrite(text, 'json-boundary'))
    opening = '    pub fn prepare_lifecycle_source_work'
    following = '    /// Inspect the original finite responsibilities'
    head, tail = text.split(opening, 1)
    parent, rest = tail.split(following, 1)
    final = SAVE + BEFORE + COMMIT + AFTER
    if parent.count(final) != 1 or head.count('fn encode_prepared_source_work') != 1:
        raise ValueError('exact original save and finish block required')
    store = SAVE.replace('codec::save(&tx, &sources, inbox.config.max_sessions)', 'Save::save(&tx, sources, max_sessions)')
    lines = store + BEFORE + (AFTER + COMMIT if reordered else COMMIT + AFTER)
    helper = HEADER + ''.join(line[4:] + '\n' for line in lines.splitlines()) + '    Ok(())\n}\n\n'
    head = head.replace('fn encode_prepared_source_work', helper + 'fn encode_prepared_source_work')
    return head + opening + parent.replace(final, CALL) + following + rest


def configuration(mode):
    profile = ROOT / 'verification/experiments/stage1-protocol/unload-startup-source/capture-profile.json'
    cfg = prepare.configuration(json.loads(profile.read_bytes())['parent'])
    name = MODULES[mode]
    if mode == 'parent':
        cfg['include'].extend(REQUEST_CARRIERS)
        cfg = frontier.configure(cfg, 'parent')
        cfg['include'].extend([HELPER, prepare.WORK + '::*::save'])
        cfg['opaque'] = [*cfg['opaque'], prepare.WORK + '::codec::save']
    else:
        cfg.update(roots=[HELPER], retained_roots=[], include=[HELPER, prepare.CRATE + '::CrashPoint'])
    return dict(cfg, namespace=name, subdir=name)


def check_parent_return(body, callback, translated):
    block, = [rows for rows in ir_values(body, 'statements') if any(
        isinstance(row['kind'], dict) and row['kind'].get('Call', {}).get('call') == callback for row in rows)]
    kinds = [row['kind'] for row in block if isinstance(row['kind'], dict)]
    branch_call, = [kind['Call']['call'] for kind in kinds if 'Call' in kind
                    and kind['Call']['call']['args'] == [{'Move': callback['dest']}]]
    functions = translated['fun_decls']
    branch = frontier.indexed(functions, frontier.call_kind(branch_call)['Fun'])
    if branch['item_meta']['name'][-1] != {'Ident': ['branch', 0]}:
        raise ValueError('the parent must branch on the actual finish result')
    switch, = [kind['Switch'] for kind in kinds if 'Switch' in kind
               and kind['Switch']['data']['scrutinee'] == {'Discriminant': branch_call['dest']}]
    control = declaration(translated['type_decls'], 'core::ops::control_flow::ControlFlow')
    variants = {row['id']: row['name'] for row in control['kind']['Enum']}
    paths = {}
    for operand, index in switch['data']['branches']:
        discriminant, variant = operand['Value'][1][0]['Discriminant']
        if discriminant['id'] != control['def_id'] or variants[variant] in paths:
            raise ValueError('ambiguous finish return path')
        paths[variants[variant]] = switch['branches'][index]
    if switch['data']['fallback'] is not None or set(paths) != {'Continue', 'Break'}:
        raise ValueError('both real finish return paths are required')
    rust_result = declaration(translated['type_decls'], 'core::result::Result')
    ok, = [row['id'] for row in rust_result['kind']['Enum'] if row['name'] == 'Ok']
    success, = [value for place, value in ir_values(paths['Continue'], 'Assign') if place['kind'] == {'Local': 0}]
    adt = success['Aggregate'][0]['Adt']
    error, = [row['call'] for row in ir_values(paths['Break'], 'Call')
              if row['call']['dest']['kind'] == {'Local': 0}]
    residual = frontier.indexed(functions, frontier.call_kind(error)['Fun'])
    if (adt[0]['id'] != rust_result['def_id'] or adt[1] != ok
            or residual['item_meta']['name'][-1] != {'Ident': ['from_residual', 0]}
            or any(place['kind'] == {'Local': 0} for place, _ in ir_values(paths['Break'], 'Assign'))):
        raise ValueError('the produced record must be returned only after successful finish')


def admit(ir, generated, cfg, target, mode):
    root, = cfg['roots']
    translated = prepare.checked_translation(ir, cfg, target, root, prepare.RETAIN if mode == 'parent' else None)
    functions = translated['fun_decls']
    helper = declaration(functions, HELPER)
    if helper['item_meta']['opacity'] != 'Transparent':
        raise ValueError('actual finish body is required')
    commit = prepare.method(functions, prepare.TRANSACTION, 'commit')['def_id']
    save_trait = declaration(translated['trait_decls'], prepare.WORK + '::SaveSourceInputs')['def_id']
    crash_trait = declaration(translated['trait_decls'], prepare.CRATE + '::CrashInjector')['def_id']
    expected = [save_trait, crash_trait, crash_trait, commit] if mode == 'reordered' else [
        save_trait, crash_trait, commit, crash_trait]
    calls = []
    seen_references = {}
    for row in ir_values(helper['body']['Structured']['body'], 'Call'):
        kind = frontier.call_kind(row['call'])
        if 'Trait' in kind:
            reference, member = kind['Trait']
            if 'Value' in reference:
                key, payload = reference['Value']
                seen_references[key] = payload
            else:
                payload = seen_references[reference['Deduplicated']]
            trait = payload['trait_decl_ref']['skip_binder']['id']
            free = 0 if trait == save_trait else 1
            if (member != 0 or trait not in (save_trait, crash_trait)
                    or payload['kind'] != {'Clause': {'Free': free}}):
                raise ValueError('unrecognized source save or crash hook')
            calls.append(trait)
        elif kind.get('Fun') == commit:
            calls.append(commit)
    if calls != expected:
        raise ValueError('actual save, hook and commit order differs')
    names = json.loads((generated / 'translation.json').read_bytes())['functions']
    mapping, = [row for row in names if row['def_id'] == helper['def_id']]
    if (mapping['lean_name'] != MODULES[mode] + '.controller.lifecycle_inbox.source_work.finish_source_work'
            or mapping['is_opaque'] is not False or mapping['source']['file'] != prepare.SOURCE):
        raise ValueError('actual generated finish body differs')
    parent_link = {}
    if mode == 'parent':
        parent = prepare.method(functions, prepare.WORK, 'prepare_lifecycle_source_work')
        body = parent['body']['Structured']['body']
        callback, = [row['call'] for row in ir_values(body, 'Call')
                     if frontier.call_kind(row['call']).get('Fun') == helper['def_id']]
        dict_ref = callback['func']['Regular']['generics']['trait_refs'][0]['Value'][1]
        writer = frontier.indexed(translated['trait_impls'], dict_ref['kind']['TraitImpl']['id'])
        method, = writer['methods']
        adapter = frontier.indexed(functions, method['skip_binder']['id'])
        original = declaration(functions, prepare.WORK + '::codec::save')['def_id']
        if (method['kind'] != {'TraitMethod': [save_trait, 0]}
                or adapter['item_meta']['opacity'] != 'Transparent'
                or list(regular_calls(adapter['body']['Structured']['body'])) != [original]
                or commit in regular_calls(body) or original in regular_calls(body)):
            raise ValueError('the parent must delegate to the original source save')
        check_parent_return(body, callback, translated)
        parent_link = dict(parent=parent['def_id'], save_adapter=adapter['def_id'], actual_save=original)
    return dict(helper=helper['def_id'], commit=commit, lean=mapping['lean_name'], **parent_link)


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
        for name, expected in prepare.SOURCE_INPUTS.items():
            if digest(source / name) != expected:
                raise ValueError('original capture source differs: ' + name)
        path = source / prepare.SOURCE
        path.write_text(factor(path.read_text(), reordered=mode == 'reordered'))
        report['candidate_source_sha256'] = digest(path)
        cfg = configuration(mode)
        selectors = [item for flag, key in [('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                     ('--include', 'include'), ('--opaque', 'opaque')] for value in cfg[key] for item in (flag, value)]
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
                            '-split-files', '-emit-json', '-namespace', cfg['namespace'], '-subdir', cfg['subdir'],
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
            raise ValueError('original finish IR changed')
        report['admission'] = admit(ir, generated, cfg, fixed['target'], mode)
        report['generated_sha256'] = {str(path.relative_to(generated)): digest(path)
                                      for path in generated.rglob('*') if path.is_file()}
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(mode, report['candidate_source_sha256'], report['admission'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=tuple(MODULES), required=True)
    for name in ('output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
