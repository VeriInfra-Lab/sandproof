"""Select the original V5 begin method and check the V3 call boundary."""
import json
import re

from prepare_transaction_capture import (
    CONTROLLER, SOURCE_COMMIT, SOURCE_TREE, TRANSACTION, checked_translation, method,
)
from revision_capture import declaration, ir_values, regular_calls


SOURCE = 'controller-service/src/controller/transaction.rs'
SOURCE_INPUTS = {
    'Cargo.lock': '9a396f80ee12c627e488a60fff9ea89d05c50f4ac9c257de076358b9beec1a99',
    SOURCE: '100391ccee63bd0e481ac7828971c2ac8eb97e781330fc6eb92a399c9d5f6bde',
    'controller-service/src/controller/allocators.rs':
        '958a4669568e59c96a70aaa5c72e4ef146b04b3bd6814064eb0be9b1fd31ec7c',
    'controller-service/src/controller/checkpoint.rs':
        'f86709d33690765c3c283085387c146e69b515dcf02dd1206f9b09235640cede',
}
OWNER = TRANSACTION + '::WorldTransaction'
ROOT = TRANSACTION + '::{' + OWNER + '}::begin'
START_FROM = OWNER + '::begin'
RUST_METHOD = TRANSACTION + '::{' + OWNER + "<'a>}::begin"
SQLITE = 'rusqlite'
SQLITE_TRANSACTION = SQLITE + '::transaction'
BEHAVIOR = SQLITE_TRANSACTION + '::TransactionBehavior'
SQLITE_BEGIN = SQLITE_TRANSACTION + '::{' + SQLITE + '::Connection}::transaction_with_behavior'
COMMITTED_WORLD = TRANSACTION + '::{' + TRANSACTION + '::CommittedWorld}::world'
HIGH_WATER = CONTROLLER + '::allocators::input_high_water'
MODULE = 'SandproofUnloadWorldBegin'


def configuration(parent):
    return dict(
        roots=[START_FROM],
        include=[ROOT, HIGH_WATER, OWNER, TRANSACTION + '::CommittedWorld', BEHAVIOR],
        opaque=parent['opaque'],
        namespace=MODULE,
        subdir=MODULE,
    )


def transparent_body(function, name):
    body = function['body']
    if (function['item_meta']['opacity'] != 'Transparent'
            or not isinstance(body, dict) or 'Structured' not in body):
        raise ValueError(f'original {name} has no transparent structured body')
    return body['Structured']['body']


def check_generated_begin(generated, mappings, begin_id, calls):
    def entry(identifier):
        matches = [row for row in mappings if row['def_id'] == identifier]
        if len(matches) != 1:
            raise ValueError('exact generated function mapping required')
        return matches[0]

    mapped = entry(begin_id)
    if (mapped['rust_name'] not in (ROOT, RUST_METHOD) or mapped['is_opaque'] is not False
            or mapped['source']['file'] != SOURCE or mapped['lean_file'] != MODULE + '/Funs.lean'
            or not mapped['lean_name'].startswith(MODULE + '.')):
        raise ValueError('generated begin does not map to the original WorldTransaction method')
    if entry(calls['committed_world'])['rust_name'] != COMMITTED_WORLD:
        raise ValueError('generated begin does not read the original CommittedWorld')
    callee = entry(calls['sqlite_begin'])
    if callee['rust_name'] != SQLITE_BEGIN:
        raise ValueError('generated begin does not use the original rusqlite method')
    lean_name = mapped['lean_name'].removeprefix(MODULE + '.')
    functions = (generated / mapped['lean_file']).read_text()
    definitions = re.findall(
        r'^\s*def\s+' + re.escape(lean_name) + r'(\s.*?)(?=^/--|^def\s|\Z)',
        functions, re.M | re.S,
    )
    if len(definitions) != 1:
        raise ValueError('actual generated begin body is missing or repeated')
    sqlite_call = callee['lean_name'].removeprefix(MODULE + '.')
    calls = list(re.finditer(
        r'^[ \t]*(?:let [^\n]*←[ \t]*)?' + re.escape(sqlite_call) + r'(?=\s|$)',
        definitions[0], re.M,
    ))
    if len(calls) != 1:
        raise ValueError('actual generated SQLite begin call is missing or repeated')
    tail = definitions[0][calls[0].end():]
    arguments = re.split(r'^[ \t]*(?:let\b|match\b|if\b|ok\b|\|)', tail, maxsplit=1, flags=re.M)[0]
    behaviors = re.findall(r'\bTransactionBehavior\.(Deferred|Immediate|Exclusive)\b', arguments)
    if behaviors != ['Immediate']:
        raise ValueError('the original SQLite begin call must pass TransactionBehavior.Immediate')
    return mapped['lean_name']


def admit(ir, generated, cfg, target):
    translated = checked_translation(ir, cfg, target, START_FROM)
    functions = translated['fun_decls']
    begin = method(functions, TRANSACTION, 'begin')
    body = transparent_body(begin, 'begin')
    call_ids = list(regular_calls(body))
    direct = [function for function in functions if function and function['def_id'] in call_ids]
    selected = {
        'committed_world': method(direct, TRANSACTION, 'world'),
        'prepare_write': declaration(functions, CONTROLLER + '::checkpoint::prepare_write'),
        'sqlite_begin': method(functions, SQLITE_TRANSACTION, 'transaction_with_behavior'),
        'input_high_water': declaration(functions, HIGH_WATER),
        'from_started': method(functions, TRANSACTION, 'from_started_transaction'),
    }
    calls = {label: function['def_id'] for label, function in selected.items()}
    for label, identifier in calls.items():
        if call_ids.count(identifier) != 1:
            raise ValueError('actual begin call site differs: ' + label)
    high_water = transparent_body(selected['input_high_water'], 'input_high_water')
    read = declaration(functions, CONTROLLER + '::allocators::read')
    if list(regular_calls(high_water)).count(read['def_id']) != 1 or list(
            ir_values(high_water, 'Str')) != ['input', '']:
        raise ValueError('input frontier must use the original allocator kind and empty scope')
    behavior = declaration(translated['type_decls'], BEHAVIOR)
    if (behavior['item_meta']['opacity'] != 'Transparent'
            or [variant['name'] for variant in behavior['kind']['Enum']] != [
                'Deferred', 'Immediate', 'Exclusive',
            ]):
        raise ValueError('original rusqlite transaction behavior declaration differs')
    mappings = json.loads((generated / 'translation.json').read_bytes())['functions']
    lean_name = check_generated_begin(generated, mappings, begin['def_id'], calls)
    return dict(actual_root=ROOT, root_id=begin['def_id'], calls=calls, input_scope='',
                input_kind='input', sqlite_behavior='Immediate', lean_name=lean_name,
                source_inputs=SOURCE_INPUTS)
