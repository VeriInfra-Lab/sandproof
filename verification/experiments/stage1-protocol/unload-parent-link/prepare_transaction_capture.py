"""Admission for the original first-Unload prepare method and its call boundary."""
import json
import re

from revision_capture import declaration, regular_calls


SOURCE_COMMIT = 'f26fc26a3fb6987a18ad69015c56bb99c0748e12'
SOURCE_TREE = 'a24d73244cf0c952271e0bc44ab4231d15036ed3'
SOURCE = 'controller-service/src/controller/lifecycle_inbox/source_work.rs'
SOURCE_INPUTS = {
    'Cargo.lock': '9a396f80ee12c627e488a60fff9ea89d05c50f4ac9c257de076358b9beec1a99',
    SOURCE: '61191bb746157427e97ba10b518a7afbb61f52e4515e5e53a20e832bb79a916a',
    'controller-service/src/controller/lifecycle_inbox/source_work/codec.rs':
        '87561cb050f40186d20be73b7eeaf91a93be2248d017316fcb886864a062a453',
    'controller-service/src/controller/lifecycle_inbox/source_work/snapshot.rs':
        'aa886d2cb586fa1e142bf3f6875f671634bd3b2de136e4bc2a3e89cf2c3bb288',
    'controller-service/src/controller/lifecycle_inbox.rs':
        '4d87f72906bb2ba5ec541b8969ecd90cf7caf74280dd42e5e6e6bbd0488888a8',
    'controller-service/src/controller/transaction.rs':
        '100391ccee63bd0e481ac7828971c2ac8eb97e781330fc6eb92a399c9d5f6bde',
}
CRATE = 'sandproof_controller_service'
CONTROLLER = CRATE + '::controller'
INBOX = CONTROLLER + '::lifecycle_inbox'
WORK = INBOX + '::source_work'
TRANSACTION = CONTROLLER + '::transaction'
ROOT = WORK + '::{' + CONTROLLER + '::ControllerJournal}::prepare_lifecycle_source_work'
MODULE = 'SandproofUnloadPrepareTransaction'
SHAPES = {
    WORK + '::SourceWorkRecord': ['work', 'observation', 'capture', 'restore', 'retirement', 'material_release'],
    WORK + '::codec::Sources': ['config', 'node', 'max_sessions', 'records'],
}


def configuration(parent):
    return dict(
        roots=[ROOT],
        include=[
            ROOT,
            CONTROLLER + '::ControllerJournal',
            CRATE + '::journal::Journal',
            TRANSACTION + '::WorldTransaction',
            CRATE + '::CrashPoint',
            CRATE + '::CrashInjector',
            CRATE + '::host_contract::HostEvidence',
            WORK + '::snapshot::CurrentSource',
            WORK + '::SourceWork',
            WORK + '::SourceAnchor',
            *SHAPES,
            INBOX + '::model::Inbox',
            INBOX + '::model::LifecycleInboxConfig',
            CRATE + '::capture::HostEpochObservation',
            CRATE + '::capture::source::SourceObservationConfig',
            CRATE + '::JournalError',
            'sandproof_protocol::lifecycle::authority::AcceptedLifecycleIntent',
        ],
        opaque=parent['opaque'],
        namespace=MODULE,
        subdir=MODULE,
    )


def method(rows, module, member):
    prefix = [{'Ident': [part, 0]} for part in module.split('::')]
    matches = []
    for row in rows:
        if row is None:
            continue
        name = row['item_meta']['name']
        if (len(name) == len(prefix) + 2 and name[:-2] == prefix
                and name[-1] == {'Ident': [member, 0]}
                and 'Ty' in name[-2].get('Impl', {})):
            matches.append(row)
    if len(matches) != 1:
        raise ValueError('expected one actual inherent method: ' + module + '::' + member)
    return matches[0]


def admit(ir, generated, cfg, target):
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    if (raw.get('has_errors') is not False
            or any(options[name] is not False for name in (
                'skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize',
            ))
            or options['exclude'] != []
            or any(options[key] != cfg[selector] for key, selector in (
                ('start_from', 'roots'), ('include', 'include'), ('opaque', 'opaque'),
            ))
            or cfg['roots'] != [ROOT]
            or {row['key'] for row in translated['target_information']} != {target}):
        raise ValueError('actual prepare capture options or target differ')

    functions = translated['fun_decls']
    prepare = method(functions, WORK, 'prepare_lifecycle_source_work')
    if (prepare['item_meta']['opacity'] != 'Transparent'
            or not isinstance(prepare['body'], dict) or 'Structured' not in prepare['body']):
        raise ValueError('original prepare has no transparent structured body')
    selected = {
        'begin': method(functions, TRANSACTION, 'begin'),
        'world': method(functions, TRANSACTION, 'world'),
        'commit': method(functions, TRANSACTION, 'commit'),
        'load_inbox': declaration(functions, INBOX + '::codec::load'),
        'authenticate': declaration(functions, INBOX + '::authenticated'),
        'load_sources': declaration(functions, WORK + '::codec::load'),
        'current': declaration(functions, WORK + '::snapshot::current'),
        'records_per_session': declaration(functions, WORK + '::codec::records_per_session'),
        'save_sources': declaration(functions, WORK + '::codec::save'),
    }
    call_ids = list(regular_calls(prepare['body']['Structured']['body']))
    calls = {label: row['def_id'] for label, row in selected.items()}
    for label, identifier in calls.items():
        if call_ids.count(identifier) != 1:
            raise ValueError('original prepare call site differs: ' + label)

    for name, expected in SHAPES.items():
        found = declaration(translated['type_decls'], name)
        if (found['item_meta']['opacity'] != 'Transparent'
                or [field['name'] for field in found['kind']['Struct']] != expected):
            raise ValueError('original prepare carrier differs: ' + name)

    generated_functions = json.loads((generated / 'translation.json').read_bytes())['functions']
    mapped = [row for row in generated_functions if row['def_id'] == prepare['def_id']]
    if (len(mapped) != 1 or mapped[0]['rust_name'] != ROOT or mapped[0]['is_opaque'] is not False
            or mapped[0]['source']['file'] != SOURCE or mapped[0]['lean_file'] != MODULE + '/Funs.lean'
            or not mapped[0]['lean_name'].startswith(MODULE + '.')):
        raise ValueError('generated prepare identity differs from original Rust root')
    lean_name = mapped[0]['lean_name'].removeprefix(MODULE + '.')
    if not re.search(r'^\s*def\s+' + re.escape(lean_name) + r'\s',
                     (generated / MODULE / 'Funs.lean').read_text(), re.M):
        raise ValueError('actual generated prepare definition is missing')
    return dict(actual_root=ROOT, root_id=prepare['def_id'], calls=calls,
                lean_name=mapped[0]['lean_name'], source_inputs=SOURCE_INPUTS)
