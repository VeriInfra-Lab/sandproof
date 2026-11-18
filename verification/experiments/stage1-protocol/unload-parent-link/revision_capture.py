"""Selectors and output admission for the current unload revision captures."""
import json
from pathlib import Path
import re

SERVICE = 'sandproof_controller_service::controller::lifecycle_inbox'
READINESS = SERVICE + '::readiness'
BASE_ROOT = READINESS + '::snapshot::base_snapshot'
PARENT_ROOT = SERVICE + '::source_work::snapshot::current'
CANDIDATE = READINESS + '::snapshot::ReadyCandidate'
REVISION_SQL = 'SELECT revision FROM sandboxes WHERE sandbox_id=?1'
# The fixed Aeneas view distributes the original continuation across six arms.
REVISION_GENERATED_CALLS = 6
REVISION_SOURCE = Path('controller-service/src/controller/lifecycle_inbox/readiness/snapshot.rs')
REVISION_SOURCE_SHA256 = '75a9b0c4564df9716b051c020a5aa5f34b64b0f532e90e2e1186d7596a5cd1a6'


def configuration(parent, mode):
    if mode == 'parent':
        return parent
    if mode != 'base-revision':
        raise ValueError('one actual unload revision root is required')
    return dict(
        roots=[BASE_ROOT],
        include=[
            BASE_ROOT,
            READINESS + '::snapshot::first_entry_at_version',
            CANDIDATE,
            *(READINESS + '::' + name for name in (
                'LifecycleCreateObservation', 'NativeCreateSnapshot',
                'CreateReadiness', 'CreateReadinessPending',
            )),
            *(name for name in parent['include'] if name.startswith(SERVICE + '::model::')),
            'sandproof_controller_service::controller::internal_inbox::model::InternalLocalBinding',
            'sandproof_controller_service::JournalError',
            'sandproof_protocol::lifecycle::authority::AcceptedLifecycleIntent',
            'sandproof_protocol::lifecycle::request::LifecycleRequest',
            'sandproof_protocol::lifecycle::request::LifecycleOperation',
        ],
        opaque=parent['opaque'],
        namespace='SandproofUnloadBaseSnapshot',
        subdir='SandproofUnloadBaseSnapshot',
    )


def declaration(rows, name):
    expected = [{'Ident': [piece, 0]} for piece in name.split('::')]
    matches = [row for row in rows if row and row['item_meta']['name'] == expected]
    if len(matches) != 1:
        raise ValueError('expected one actual declaration: ' + name)
    return matches[0]


def ir_values(value, field):
    if isinstance(value, dict):
        if field in value:
            yield value[field]
        for child in value.values():
            yield from ir_values(child, field)
    elif isinstance(value, list):
        for child in value:
            yield from ir_values(child, field)


def regular_calls(value):
    for call in ir_values(value, 'Call'):
        function = call['call']['func']
        identifier = function.get('Regular', {}).get('kind', {}).get('Fun')
        if identifier is not None:
            yield identifier


def revision_query(structured, functions):
    if list(ir_values(structured, 'Str')) != [REVISION_SQL]:
        raise ValueError('original base_snapshot revision SQL differs')
    queries = list(re.finditer(r'^[ \t]+rusqlite\.Connection\.query_row[ \t]*$', functions, re.M))
    if len(queries) != REVISION_GENERATED_CALLS:
        raise ValueError('generated base_snapshot revision query count differs')
    next_step = re.compile(r'^[ \t]+(?:let|match)\b', re.M)
    for query in queries:
        end = next_step.search(functions, query.end())
        if end is None:
            raise ValueError('generated base_snapshot revision query arguments are missing')
        arguments = functions[query.end():end.start()]
        if re.findall(r'\(toStr\s+"([^"\n]*)"\)', arguments) != [REVISION_SQL]:
            raise ValueError('generated base_snapshot revision SQL differs')


def admit(ir, generated, cfg, target, mode):
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    if (raw.get('has_errors') is not False
            or any(options[name] is not False for name in (
                'skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize',
            ))
            or options['exclude'] != []
            or any(options[ir_key] != cfg[cfg_key] for ir_key, cfg_key in (
                ('start_from', 'roots'), ('include', 'include'), ('opaque', 'opaque'),
            ))
            or {row['key'] for row in translated['target_information']} != {target}):
        raise ValueError('actual capture options, target or complete IR differ')
    root, = cfg['roots']
    if root != (BASE_ROOT if mode == 'base-revision' else PARENT_ROOT):
        raise ValueError('wrong actual unload root')
    function = declaration(translated['fun_decls'], root)
    if function['item_meta']['opacity'] != 'Transparent' or not isinstance(function['body'], dict):
        raise ValueError('actual unload root has no transparent body')
    structured = function['body']['Structured']['body']
    candidate = declaration(translated['type_decls'], CANDIDATE)
    fields = [field['name'] for field in candidate['kind']['Struct']]
    if candidate['item_meta']['opacity'] != 'Transparent' or fields.count('native_revision') != 1:
        raise ValueError('actual ReadyCandidate revision field is missing or repeated')

    module = generated / cfg['subdir']
    types = (module / 'Types.lean').read_text()
    field_type = r'^[ \t]+native_revision[ \t]*:[ \t]*\(?Option[ \t]+(?:Std\.)?U64\)?[ \t]*$'
    if len(re.findall(field_type, types, re.M)) != 1:
        raise ValueError('generated ReadyCandidate revision type differs')
    report = dict(actual_root=root, root_id=function['def_id'],
                  ready_candidate_id=candidate['def_id'], ready_candidate_fields=fields,
                  native_revision_type='Option<u64>')
    if mode == 'base-revision':
        revision_query(structured, (module / 'Funs.lean').read_text())
        report.update(original_revision_sql=REVISION_SQL, generated_revision_queries=REVISION_GENERATED_CALLS)
    else:
        base = declaration(translated['fun_decls'], BASE_ROOT)
        if list(regular_calls(structured)).count(base['def_id']) != 1:
            raise ValueError('actual Unload parent no longer calls its original base_snapshot once')
    return report
