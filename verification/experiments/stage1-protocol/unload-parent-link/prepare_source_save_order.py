"""Admit the actual source-save caller and materialize its isolated Lean proof."""
import argparse
import json
from pathlib import Path
import re
import shutil

from frontier_capture import call_kind
from prepare_started_transaction import PROVIDER, materialize as materialize_provider
from prepare_world_begin import closure, read_exact
from revision_capture import declaration, ir_values, regular_calls
from source_save_order import ARCHIVE, CODEC, CODEC_ROOT, DIGESTS, HERE, METHODS, MODES, NAMESPACE
from source_save_order import ROOT, SAVE, SOURCE, TREE
from source_save_order import admit, configuration, digest


VIEW = 'verification/aeneas/source-save-order'
ROOTS = {'factored': 'SourceSaveOrder', 'reordered': 'SourceSaveOrderReordered'}
INPUTS = 'SaveOrderInputs'
GENERATED = ('Types.lean', 'Funs.lean')
DRIVER = HERE.parent / 'source_save_order.py'
FORWARDED = [('Shared',), ('Shared', 'Copy'), ('Shared',), ('Shared', 'Shared', 'Shared'), ('Shared', 'Move')]


def check_forwarded_call(function, target, expected):
    body = function['body']['Structured']['body']
    call, = (item['call'] for item in ir_values(body, 'Call'))
    assigned = list(ir_values(body, 'Assign'))
    if (call_kind(call) != {'Fun': target} or call['dest']['kind'] != {'Local': 0}
            or len(call['args']) != len(expected) or len(assigned) != len(expected)
            or list(ir_values(body, 'Switch')) or body['statements'][-1]['kind'] != 'Return'):
        raise ValueError('the actual adapter must forward all arguments and return the original result')
    for index, (operand, kind, pair) in enumerate(zip(call['args'], expected, assigned, strict=True), 1):
        place, value = pair
        if kind == 'Shared':
            reference = value['Ref']
            caller, projection = reference['place']['kind']['Projection']
            if (reference['kind'], projection) != ('Shared', 'Deref'):
                raise ValueError('the actual adapter must borrow the original argument')
        else:
            caller = value['Use'][0][kind]
        if operand != {'Move': place} or caller['kind'] != {'Local': index}:
            raise ValueError('the actual adapter must forward the original argument in place')


def check_parent_arguments(ir, admission):
    functions = json.loads(ir.read_bytes())['translated']['fun_decls']
    check_forwarded_call(declaration(functions, SAVE), admission['save'], ('Shared', 'Shared', 'Copy'))
    codec = [{'Ident': [name, 0]} for name in CODEC_ROOT.split('::')]
    for method, target, inputs in zip(METHODS, admission['original_calls'], FORWARDED, strict=True):
        adapter, = (row for row in functions if row and row['item_meta']['opacity'] == 'Transparent'
                    and row['item_meta']['name'][:len(codec)] == codec
                    and row['item_meta']['name'][-1] == {'Ident': [method, 0]}
                    and list(regular_calls(row['body']['Structured']['body'])) == [target])
        check_forwarded_call(adapter, target, inputs)


def captured(path, mode):
    report = json.loads((path / 'result.json').read_bytes())
    sha = DIGESTS['factored' if mode == 'parent' else mode]
    if (report['complete'] is not True or report['public_acceptance'] is not False
            or (report['mode'], report['source_commit'], report['source_tree'], report['codec_sha256'])
            != (mode, SOURCE, TREE, sha) or report['driver_sha256'] != digest(DRIVER)):
        raise ValueError('the complete current source save extraction is required')
    if ([row['label'] for row in report['commands']] != ['charon', 'aeneas']
            or any(type(row['exit_code']) is not int or row['exit_code'] != 0 or row['bounded_stop'] is not False
                   for row in report['commands'])):
        raise ValueError('both actual bounded extraction commands must have succeeded')
    read_exact(path / 'source.tar', ARCHIVE)
    read_exact(path / 'source' / CODEC, sha)
    for row in report['commands']:
        read_exact(path / (row['label'] + '.log'), row['log_sha256'])
    ir = path / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = path / 'generated'
    expected = {'translation.json', *(f'{NAMESPACE}/{name}' for name in (*GENERATED, 'TypesExternal_Template.lean'))}
    if mode == 'parent':
        expected.add(f'{NAMESPACE}/FunsExternal_Template.lean')
    if report['generated_sha256'].keys() != expected:
        raise ValueError('actual generated output inventory differs')
    bodies = {name: read_exact(generated / name, report['generated_sha256'][name]) for name in expected}
    env = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    if admit(ir, generated, configuration(mode), env['target'], mode) != report['admission']:
        raise ValueError('the actual source save admission differs')
    if mode == 'parent':
        check_parent_arguments(ir, report['admission'])
    return bodies, report


def body(lean):
    pattern = rb'^def controller\.lifecycle_inbox\.source_work\.codec\.save_using\n.*?(?=\n\n(?:/--|end ))'
    member, = re.findall(pattern, lean, re.M | re.S)
    return member


def materialize(capture, parent, correct, output, packages, mcp, mode):
    chosen, receipt = captured(capture, mode)
    whole, parent_receipt = captured(parent, 'parent')
    positive = chosen if mode == 'factored' else captured(correct, 'factored')[0]
    function = f'{NAMESPACE}/Funs.lean'
    if body(positive[function]) != body(whole[function]):
        raise ValueError('the actual native caller must use the same generated save function')
    if mode == 'reordered' and positive[f'{NAMESPACE}/Types.lean'] != chosen[f'{NAMESPACE}/Types.lean']:
        raise ValueError('the same generated types must be used in the reordered view')

    materialize_provider(output, packages)
    tools = output / '.tools'
    tools.mkdir()
    (tools / 'lean-mcp').symlink_to(mcp, target_is_directory=True)
    for name in ('tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, target)
    proof = ROOTS[mode]
    with (output / 'lakefile.toml').open('a') as view:
        for library in (NAMESPACE, INPUTS, proof):
            view.write(f'\n[[lean_lib]]\nname = "{library}"\nsrcDir = "{VIEW}"\n')
    target = output / VIEW / NAMESPACE
    target.mkdir(parents=True)
    for name in GENERATED:
        (target / name).write_bytes(chosen[f'{NAMESPACE}/{name}'])
    shutil.copyfile(HERE / 'TypesExternal.lean.in', target / 'TypesExternal.lean')
    for name in (INPUTS, proof):
        shutil.copyfile(HERE / (name + '.lean.in'), output / VIEW / (name + '.lean'))

    root = f'{VIEW}/{proof}.lean'
    strict = {f'{VIEW}/{NAMESPACE}/{name}' for name in (*GENERATED, 'TypesExternal.lean')}
    strict |= {f'{VIEW}/{INPUTS}.lean', root}
    reachable = closure(output, [root], active_view='lakefile.toml')
    provider = {row['path']: row['sha256'] for row in
                json.loads((PROVIDER / 'materialization.json').read_bytes())['modules'].values()}
    if strict - reachable.keys() or any(provider.get(name) != sha for name, sha in reachable.items()
                                         if name not in strict):
        raise ValueError('the save proof must use only exact original providers')
    plan = dict(source=SOURCE, root=proof, mode=mode, ir=receipt['ir_sha256'], parent_ir=parent_receipt['ir_sha256'],
                strict_files=sorted(strict), files=reachable,
                modules={name: {'sha256': sha} for name, sha in reachable.items()},
                build_order=[name.removesuffix('.lean') for name in reachable])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(f'{proof}: {len(reachable)} exact Lean inputs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=MODES, required=True)
    for name in ('capture', 'parent', 'correct', 'output', 'lake-packages', 'lean-mcp'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    materialize(args.capture.resolve(strict=True), args.parent.resolve(strict=True), args.correct.resolve(strict=True),
                args.output.resolve(), args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True),
                args.mode)
