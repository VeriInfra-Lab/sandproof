"""Capture the concrete #963 scan and observation comparison dictionaries."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import observation_slot_capture as slot
from source_first_match import borrowed_paths
from source_retention_providers import only_call


HERE = Path(__file__).with_name('observation-comparators')
PROVIDERS = slot.ROOT / 'verification/experiments/stage1-protocol'
NAMESPACE = 'SandproofObservationComparators'
SCAN = slot.WORK + '::observation_slot_full'
GUARD = slot.WORK + '::observation_slot_raw'
PROBE = 'crate::proof_source_work_eq'
RAW_TYPE = 'sandproof_controller_service::capture::source::RawSourceObservation'
MODES = ('core', 'skip-executable')
WORK = Path(slot.finish.prepare.SOURCE)
RAW = Path('controller-service/src/capture/source.rs')
LIB = Path('controller-service/src/lib.rs')
INPUTS = {
    **slot.finish.prepare.SOURCE_INPUTS,
    str(RAW): '862c749de515fbe35595b6f00043455935d9581e22ae10554f639579c1833fbc',
    str(LIB): 'bc15a0989bca7001b2927e6c912601832bf01601cf83e3cfac34351e4fbd6f73',
}
INCLUDES = ['sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service', 'core::tuple']


def templates():
    return (HERE / 'probes.rs.in', HERE / 'skip-executable.rs.in',
            PROVIDERS / 'source-work-match/probe.rs.in')


def transformed(name, original, mode):
    if name == WORK:
        return slot.factor(original, 'core') + (HERE / 'probes.rs.in').read_text()
    if name == LIB:
        return original + (PROVIDERS / 'source-work-match/probe.rs.in').read_text()
    if name == RAW and mode == 'skip-executable':
        derive = ('#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]\n'
                  '#[serde(deny_unknown_fields)]\npub struct RawSourceObservation {')
        if original.count(derive) != 1:
            raise ValueError('unique original raw observation derive required')
        return original.replace(derive, derive.replace('PartialEq, ', '')) + (HERE / 'skip-executable.rs.in').read_text()
    return original


def stage(source, mode):
    if mode not in MODES:
        raise ValueError('unknown comparator mode')
    for name, sha in INPUTS.items():
        if slot.digest(source / name) != sha:
            raise ValueError('exact original input differs: ' + name)
    for name in (WORK, LIB, RAW):
        path = source / name
        text = path.read_text()
        candidate = transformed(name, text, mode)
        if candidate != text:
            path.write_text(candidate)
    return {str(name): slot.digest(source / name) for name in (WORK, LIB, RAW)}


def reference(translated, call, trait):
    refs = call['func']['Regular']['generics']['trait_refs']
    entry, = (item['Value'][1] for item in refs if 'Value' in item
              and item['Value'][1]['trait_decl_ref']['skip_binder']['id'] == trait)
    return slot.indexed(translated['trait_impls'], entry['kind']['TraitImpl']['id'])


def compare_member(translated, implementation, trait, ordinal, mode='TraitMethod'):
    entry, = (entry for entry in implementation['methods'] if entry['kind'] == {mode: [trait, ordinal]})
    return slot.indexed(translated['fun_decls'], entry['skip_binder']['id'])


def symbol(row):
    return row['item_meta']['name']


def admit(ir, generated, target):
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    if (raw['has_errors'] is not False or options['start_from'] != [SCAN, GUARD]
            or options['start_from_if_exists'] != [PROBE] or options['include'] != INCLUDES
            or options['exclude'] or options['opaque']
            or any(options[name] is not False for name in (
                'skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or {row['key'] for row in translated['target_information']} != {target}):
        raise ValueError('the actual comparator capture options differ')
    functions, types, traits = (translated[key] for key in ('fun_decls', 'type_decls', 'trait_decls'))
    scan, guard, work_probe = (slot.declaration(functions, name) for name in (SCAN, GUARD, 'sandproof_controller_service::proof_source_work_eq'))
    scan_body, lookup = only_call(scan)
    guard_body, decision = only_call(guard)
    for body, call, helper in ((scan_body, lookup, slot.FINDER), (guard_body, decision, slot.DECISION)):
        if (slot.call_kind(call) != {'Fun': slot.declaration(functions, helper)['def_id']}
                or borrowed_paths(body, call) != [(1,), (2,)]):
            raise ValueError('concrete dictionaries must use the same original helper and full operands')
    scan_trait, eq_trait = (slot.declaration(traits, name)['def_id'] for name in (slot.TRAIT, 'core::cmp::PartialEq'))
    work_impl = reference(translated, lookup, scan_trait)
    raw_impl = reference(translated, decision, eq_trait)
    work_member = compare_member(translated, work_impl, scan_trait, 0)
    raw_eq = compare_member(translated, raw_impl, eq_trait, 0)
    body, complete = only_call(work_member)
    probe_body, probe_call = only_call(work_probe)
    eq_call = slot.call_kind(complete)
    shared_eq = slot.indexed(functions, slot.call_kind(probe_call)['Fun'])
    shared_prefix = [{'Ident': [name, 0]} for name in ('core', 'cmp', 'impls')]
    probe_eq = compare_member(translated, reference(translated, probe_call, eq_trait), eq_trait, 0)
    if (eq_call != {'Fun': probe_eq['def_id']} or symbol(shared_eq)[:3] != shared_prefix
            or symbol(shared_eq)[-1] != {'Ident': ['eq', 0]}
            or borrowed_paths(probe_body, probe_call) != [(1,), (2,)]
            or borrowed_paths(body, complete) != [(1, 0), (2,)]):
        raise ValueError('the current scan must select the existing complete SourceWork provider')
    work_eq = slot.indexed(functions, eq_call['Fun'])
    expected = ['work_binding', 'resource', 'source', 'configuration', 'executable_sha256',
                'kernel_sha256', 'kernel_bytes', 'disk_bytes']
    actual = slot.declaration(types, RAW_TYPE)
    if [field['name'] for field in actual['kind']['Struct']] != expected:
        raise ValueError('the actual raw observation field inventory changed')
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    for function in (scan, guard, work_probe, work_member, work_eq, raw_eq):
        if (function['item_meta']['opacity'] != 'Transparent' or 'Structured' not in function['body']
                or not any(not row['is_opaque'] for row in mapping if row['def_id'] == function['def_id'])):
            raise ValueError('the original comparison requires the actual generated provider body')
    return dict(scan=scan['def_id'], guard=guard['def_id'], lookup=slot.call_kind(lookup)['Fun'],
                work_adapter=work_member['def_id'], work_eq=work_eq['def_id'], raw_eq=raw_eq['def_id'],
                raw_fields=expected)


def capture(mode, output, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or min(shutil.disk_usage(slot.ROOT).free, shutil.disk_usage(output.parent).free) < 80 * slot.GIB):
        raise ValueError('formal role and original proof storage budget required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    fixed = json.loads((slot.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(slot.digest(path) != fixed[name]['binary_sha256'] for name, path in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('fixed proof tools required')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source_commit=slot.finish.SOURCE, source_tree=slot.finish.TREE,
                  complete=False, commands=[], driver_sha256=slot.digest(Path(__file__)), cpus=cpus,
                  templates={str(path.relative_to(slot.ROOT)): slot.digest(path) for path in templates()})
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', slot.finish.SOURCE], cwd=slot.ROOT, stdout=stream, check=True)
        if slot.digest(archive) != slot.finish.ARCHIVE:
            raise ValueError('the original fixed integration archive differs')
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        report['staged'] = stage(source, mode)
        ir, generated = output / 'actual.llbc', output / 'generated'
        includes = [flag for name in INCLUDES for flag in ('--include', name)]
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        '--start-from', SCAN, '--start-from', GUARD, '--start-from-if-exists', PROBE,
                        *includes, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                        '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, command, limit in commands:
                result = slot.run(name, command, limit, source, output, env)
                report['commands'].append(result)
                if result['exit_code'] != 0 or result['bounded_stop']:
                    raise ValueError(name + ' did not complete')
                if name == 'charon':
                    report['ir_sha256'] = slot.digest(ir)
        if slot.digest(ir) != report['ir_sha256']:
            raise ValueError('actual original IR changed during capture')
        report['admission'] = admit(ir, generated, fixed['target'])
        report['generated_sha256'] = {str(path.relative_to(generated)): slot.digest(path)
                                      for path in sorted(generated.rglob('*')) if path.is_file()}
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(mode, report['admission'])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', required=True, choices=MODES)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
