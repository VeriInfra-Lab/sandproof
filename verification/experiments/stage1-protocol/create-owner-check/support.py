"""Actual V3 owner guards: full original inputs, transparent roots, unchanged contracts."""
import importlib.util
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-owner-check')
SOURCE = 'controller-service/src/controller/lifecycle_inbox.rs'
GEN = Path('verification/aeneas/stage1-protocol/owner-mapping/SandproofStage1OwnerMapping')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
PREFIX = 'sandproof_controller_service::controller::lifecycle_inbox::'
NAMES = ('check_existing_create_owner', 'check_new_create_owner', 'owner_for', 'create_owner_successor')
ROOTS = [PREFIX+'proof_'+name for name in NAMES]
INCLUDE = ['sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service']
NAMESPACE = 'SandproofStage1OwnerMappingRust'
LIBRARY = 'SandproofStage1OwnerMapping'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


legacy = module('owner_check_existing_successor_support', ROOT/HERE.parent/'create-owner-successor/support.py')
owner = module('owner_check_original_owner_admission', ROOT/HERE.parent/'adopted-create/reuse/admission.py')
parent, mapping, journal, sha = legacy.parent, legacy.mapping, legacy.journal, legacy.sha
wrapper = legacy.wrapper


def inventory():
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for group, prefix in [('source_inputs', ROOT), ('providers', ROOT),
                          ('templates', ROOT/HERE), ('drivers', ROOT/HERE)]:
        for name, digest in data[group].items():
            if sha(prefix/name) != digest:
                raise ValueError('reviewed '+group+' differs: '+name)
    return data


def prepare(project, variant, data, append_root=True):
    path = project/SOURCE
    if sha(path) != data['source_inputs'][SOURCE]:
        raise ValueError('actual production source differs')
    plan = data['variants'][variant]
    mutation = plan['mutation']
    if mutation is not None:
        text = path.read_text()
        start = text.index(plan['function_marker'])
        brace = text.index('{', start)
        depth, end = 0, None
        for index in range(brace, len(text)):
            depth += int(text[index] == '{') - int(text[index] == '}')
            if depth == 0:
                end = index+1
                break
        if end is None:
            raise ValueError('unclosed actual function')
        body = text[start:end]
        if body.count(mutation['before']) != 1:
            raise ValueError('mutation is not unique in actual business function')
        path.write_text(text[:start]+body.replace(mutation['before'], mutation['after'], 1)+text[end:])
    before_root = sha(path)
    if append_root:
        with path.open('ab') as stream:
            stream.write((ROOT/HERE/'root.rs.in').read_bytes())
    return dict(path=SOURCE, original_sha256=data['source_inputs'][SOURCE],
                variant_sha256=before_root, derived_sha256=sha(path), mutation=mutation,
                actual_production_base=True, private_business_replacement=False)


def admit_ir(raw, opaque):
    options = raw['translated']['options']
    if (raw['has_errors'] or options['start_from'] != ROOTS or options['include'] != INCLUDE
            or opaque != ['rusqlite::error::Error'] or options['opaque'] != opaque
            or options['exclude'] or options['skip_borrowck'] or options['no_typecheck']
            or options['monomorphize'] or options['remove_unused_clauses']
            or {t['key'] for t in raw['translated']['target_information']} != {'x86_64-unknown-linux-gnu'}):
        raise ValueError('incomplete or unreviewed capture')
    journal.admit(raw)
    functions = [f for f in raw['translated']['fun_decls'] if f]

    def select(name):
        path = [{'Ident': [n, 0]} for n in (PREFIX+name).split('::')]
        values = [f for f in functions if f['item_meta']['name'] == path]
        if (len(values) != 1 or values[0]['item_meta']['opacity'] != 'Transparent'
                or 'Structured' not in values[0]['body']):
            raise ValueError('missing or external business function '+name)
        return values[0]

    def forward(name):
        root, actual = select('proof_'+name), select(name)
        calls, assignments = [], []

        def walk(node):
            if isinstance(node, dict):
                if 'Call' in node:
                    calls.append(node['Call']['call'])
                if 'Assign' in node:
                    assignments.append(node['Assign'])
                for child in node.values():
                    walk(child)
            elif isinstance(node, list):
                for child in node:
                    walk(child)

        walk(root['body'])
        if root['body']['Structured']['locals']['arg_count'] != 3 or len(calls) != 1 or len(assignments) != 3:
            raise ValueError('incomplete forwarding root')
        call = calls[0]
        if (call['func']['Regular']['kind']['Fun'] != actual['def_id']
                or call['dest']['kind'] != {'Local': 0} or len(call['args']) != 3):
            raise ValueError('different business call')
        for argument, (destination, expression), original in zip(call['args'][:2], assignments[:2], [1, 2]):
            reference = expression.get('Ref', {})
            projection = reference.get('place', {}).get('kind', {}).get('Projection')
            if (argument.get('Move', {}).get('kind') != destination['kind']
                    or reference.get('kind') != 'Shared' or not projection
                    or projection[0]['kind'] != {'Local': original} or projection[1] != 'Deref'):
                raise ValueError('changed original complete shared borrow')
        destination, expression = assignments[2]
        if (call['args'][2].get('Move', {}).get('kind') != destination['kind']
                or expression.get('Use') != [{'Copy': {'kind': {'Local': 3}, 'ty': destination['ty']}}, 'Yes']):
            raise ValueError('changed original complete third value')
        return dict(root=root['def_id'], actual=actual['def_id'],
                    two_original_shared_borrows=True, original_complete_third_copy=True)

    return dict(owner=owner.admit_ir(raw), successor=forward('create_owner_successor'),
                existing=forward('check_existing_create_owner'), new=forward('check_new_create_owner'))


def capture(project, folder, command, fixed):
    ir = Path('artifacts/original-enum.llbc')
    opaque = ['rusqlite::error::Error']
    argv = [str(mapping.VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
    for name in INCLUDE:
        argv += ['--include', name]
    for name in opaque:
        argv += ['--opaque', name]
    for name in ROOTS:
        argv += ['--start-from', name]
    argv += ['--dest-file', str(mapping.VIRTUAL/ir), '--', '--manifest-path',
             str(HERE.parent/'inbox-record/Cargo.toml'), '--lib', '--locked', '--offline', '--target', fixed['target']]
    command(project, folder, 'original-enum-charon', wrapper(project)+argv)
    raw = json.loads((project/ir).read_text())
    return raw, ir, admit_ir(raw, opaque)


def admit_generated(generated, data, translated=None):
    translated = translated or json.loads((generated/'translation.json').read_text())
    functions = [{k: f[k] for k in ('lean_name', 'rust_name', 'is_opaque', 'lean_file')}
                 for f in translated['functions']]
    if functions != data['generated_functions']:
        raise ValueError('changed complete generated function closure')
    if [f['lean_name'] for f in functions if f['is_opaque']] != ['alloc.string.String.Insts.CoreCmpPartialEqString.eq']:
        raise ValueError('changed external library or business opacity')
    for name in NAMES:
        full = NAMESPACE+'.'+(PREFIX+name).replace('::', '.')
        values = [f for f in functions if f['lean_name'] == full]
        if len(values) != 1 or values[0]['is_opaque'] or values[0]['rust_name'] != PREFIX+name:
            raise ValueError('missing or external generated business root')
        text = (generated/values[0]['lean_file']).read_text()
        short = full.removeprefix(NAMESPACE+'.')
        if (len(re.findall(r'^namespace '+NAMESPACE+'$', text, re.M)) != 1
                or len(re.findall(r'^end '+NAMESPACE+'$', text, re.M)) != 1
                or len(re.findall(r'^def\s+'+re.escape(short)+r'\b', text, re.M)) != 1):
            raise ValueError('actual generated business body absent')
    return translated


def admission_rejections(raw, generated, data):
    from copy import deepcopy
    translated = json.loads((generated/'translation.json').read_text())
    rejected = []
    for label in ('reordered-new-inputs', 'projected-new-binding', 'external-new-body',
                  'missing-new-body', 'missing-generated-new', 'external-generated-new'):
        ir, meta = deepcopy(raw), deepcopy(translated)
        functions = ir['translated']['fun_decls']
        path = lambda name: [{'Ident': [n, 0]} for n in (PREFIX+name).split('::')]
        root = next(f for f in functions if f and f['item_meta']['name'] == path('proof_check_new_create_owner'))
        actual = next(f for f in functions if f and f['item_meta']['name'] == path('check_new_create_owner'))
        if label == 'external-new-body':
            actual['item_meta']['opacity'] = 'Opaque'
        elif label == 'missing-new-body':
            functions[functions.index(actual)] = None
        elif label.endswith('generated-new'):
            name = NAMESPACE+'.'+(PREFIX+'check_new_create_owner').replace('::', '.')
            if label.startswith('missing'):
                meta['functions'] = [f for f in meta['functions'] if f['lean_name'] != name]
            else:
                next(f for f in meta['functions'] if f['lean_name'] == name)['is_opaque'] = True
        else:
            def mutate(node):
                if isinstance(node, dict):
                    if label == 'reordered-new-inputs' and 'Call' in node:
                        args = node['Call']['call']['args']
                        args[0], args[1] = args[1], args[0]
                    if label == 'projected-new-binding' and 'Assign' in node:
                        _, expression = node['Assign']
                        if 'Use' in expression:
                            expression['Use'][0]['Copy']['kind'] = {'Local': 2}
                    for child in node.values():
                        mutate(child)
                elif isinstance(node, list):
                    for child in node:
                        mutate(child)
            mutate(root['body'])
        try:
            admit_ir(ir, ['rusqlite::error::Error'])
            admit_generated(generated, data, meta)
        except (ValueError, AssertionError):
            rejected.append(label)
        else:
            raise ValueError('admission accepted '+label)
    return rejected
