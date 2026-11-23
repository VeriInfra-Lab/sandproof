"""Check the original first-Unload admission block factored into a Rust call."""
import json
import re

import prepare_transaction_capture as prepare
from revision_capture import declaration, ir_values, regular_calls


HELPER = 'open_source_work_inputs'
FRONTIER = prepare.WORK + '::' + HELPER
MODULES = {'prefix': 'SandproofUnloadFrontier', 'parent': 'SandproofUnloadFactoredPrepare',
           'reordered': 'SandproofUnloadFrontier'}
OPEN = '''        let tx =
            super::WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
'''
BODY = '''        let inbox = super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, intent, context)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
'''
HEADER = '''trait ReadSourceInputs<Rows> {
    fn read(connection: &rusqlite::Connection) -> Result<Option<Rows>, JournalError>;
}

struct ReadSourceInputsFromJournal;

impl ReadSourceInputs<codec::Sources> for ReadSourceInputsFromJournal {
    fn read(connection: &rusqlite::Connection) -> Result<Option<codec::Sources>, JournalError> {
        codec::load(connection)
    }
}

fn open_source_work_inputs<'a, Rows, Reader: ReadSourceInputs<Rows>>(
    connection: &'a mut rusqlite::Connection,
    committed: &'a super::super::transaction::CommittedWorld,
    intent: &AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
) -> Result<(super::WorldTransaction<'a>, super::Inbox, Rows), JournalError> {
'''
CALL = '''        let (tx, inbox, mut sources) = open_source_work_inputs::<_, ReadSourceInputsFromJournal>(
            &mut self.journal.connection,
            &self.committed_world,
            intent,
            context,
        )?;
'''


def factor(text, *, reordered=False):
    before, suffix = text.split('    pub fn prepare_lifecycle_source_work', 1)
    if suffix.count(OPEN + BODY) != 1 or before.count('fn encode_prepared_source_work') != 1:
        raise ValueError('exact first-Unload admission block and encoder required')
    inbox, authenticate, sources = BODY.splitlines()
    sources = sources.replace('let mut sources = codec::load', 'let sources = Reader::read')
    order = (inbox, sources, authenticate) if reordered else (inbox, authenticate, sources)
    loads = ''.join(line[4:] + '\n' for line in order)
    helper = ('    let tx = super::WorldTransaction::begin(connection, committed)?;\n'
              + loads
              + '    Ok((tx, inbox, sources))\n}\n\n')
    before = before.replace('fn encode_prepared_source_work', HEADER + helper + 'fn encode_prepared_source_work')
    return before + '    pub fn prepare_lifecycle_source_work' + suffix.replace(OPEN + BODY, CALL)


def configure(cfg, mode):
    cfg = dict(cfg, namespace=MODULES[mode], subdir=MODULES[mode])
    if mode != 'parent':
        return dict(cfg, roots=[FRONTIER], retained_roots=[],
                    include=[FRONTIER, prepare.TRANSACTION + '::WorldTransaction',
                             prepare.CRATE + '::JournalError'])
    return dict(cfg, include=[*cfg['include'], FRONTIER, prepare.WORK + '::*::read'])


def call_kind(call):
    return call['func'].get('Regular', {}).get('kind', {})


def indexed(rows, identifier):
    row, = [row for row in rows if row is not None and row['def_id'] == identifier]
    return row


def original_reader(body, translated, trait, prefix):
    parent, = (call['call'] for call in ir_values(body, 'Call')
               if call_kind(call['call']).get('Fun') == prefix)
    reference, = parent['func']['Regular']['generics']['trait_refs']
    reference = reference['Value'][1]
    instance = indexed(translated['trait_impls'], reference['kind']['TraitImpl']['id'])
    method, = instance['methods']
    if (reference['trait_decl_ref']['skip_binder']['id'] != trait
            or method['kind'] != {'TraitMethod': [trait, 0]}):
        raise ValueError('parent must pass the original source reader dictionary')
    reader = indexed(translated['fun_decls'], method['skip_binder']['id'])
    load = declaration(translated['fun_decls'], prepare.WORK + '::codec::load')['def_id']
    if (reader['item_meta']['opacity'] != 'Transparent'
            or list(regular_calls(reader['body']['Structured']['body'])) != [load]):
        raise ValueError('actual reader must delegate to the original source loader')
    return parent, reader['def_id'], load


def check_parent_gate(body, translated, prefix_call, question):
    kinds = [row['kind'] for row in body['statements'] if isinstance(row['kind'], dict)]
    branch_call, = (kind['Call']['call'] for kind in kinds if 'Call' in kind
                    and kind['Call']['call']['args'] == [{'Move': prefix_call['dest']}])
    branch = indexed(translated['fun_decls'], call_kind(branch_call)['Fun'])
    if branch['item_meta']['name'][-1] != {'Ident': ['branch', 0]}:
        raise ValueError('the parent must branch on the actual prefix result')
    switch, = (kind['Switch'] for kind in kinds
               if 'Switch' in kind and kind['Switch']['data']['scrutinee'] == {'Discriminant': branch_call['dest']})
    flow = declaration(translated['type_decls'], 'core::ops::control_flow::ControlFlow')
    variants = {variant['id']: variant['name'] for variant in flow['kind']['Enum']}
    paths = {}
    for value, index in switch['data']['branches']:
        discriminant = value['Value'][1][0]['Discriminant']
        if discriminant[0]['id'] != flow['def_id']:
            raise ValueError('expected the original ControlFlow discriminant')
        name = variants[discriminant[1]]
        if name in paths:
            raise ValueError('duplicate parent branch')
        paths[name] = list(regular_calls(switch['branches'][index]))
    if (switch['data']['fallback'] is not None or set(paths) != {'Continue', 'Break'}
            or paths['Continue'].count(question) != 1 or question in paths['Break']):
        raise ValueError('only the successful original prefix branch may construct a source question')


def admit(ir, generated, cfg, target, mode):
    root = prepare.START_FROM if mode == 'parent' else FRONTIER
    retained = prepare.RETAIN if mode == 'parent' else None
    translated = prepare.checked_translation(ir, cfg, target, root, retained)
    fns = translated['fun_decls']
    prefix = declaration(fns, FRONTIER)
    sequence = [
        prepare.method(fns, prepare.TRANSACTION, 'begin'),
        declaration(fns, prepare.INBOX + '::codec::load'),
        declaration(fns, prepare.INBOX + '::authenticated'),
    ]
    prefix_body = prefix['body']['Structured']['body']
    trait = declaration(translated['trait_decls'], prepare.WORK + '::ReadSourceInputs')['def_id']
    calls = [call_kind(call['call']) for call in ir_values(prefix_body, 'Call')]
    ids = [row['def_id'] for row in sequence]
    read, = [reference for reference in calls if 'Trait' in reference]
    reference, member = read['Trait']
    reference = reference['Value'][1]
    expected = [{'Fun': identifier} for identifier in ids]
    expected.insert(2 if mode == 'reordered' else 3, read)
    if (prefix['item_meta']['opacity'] != 'Transparent'
            or reference['kind'] != {'Clause': {'Free': 0}}
            or reference['trait_decl_ref']['skip_binder']['id'] != trait or member != 0
            or [call for call in calls if call.get('Fun') in ids or call == read] != expected):
        raise ValueError('selected original or reordered prefix must reach all four exact call sites')
    selected = [prefix]
    details = {}
    if mode == 'parent':
        parent = prepare.method(fns, prepare.WORK, 'prepare_lifecycle_source_work')
        question = declaration(fns, prepare.WORK + '::snapshot::current')['def_id']
        body = parent['body']['Structured']['body']
        parent_calls = list(regular_calls(body))
        prefix_call, reader, load = original_reader(body, translated, trait, prefix['def_id'])
        if (parent['item_meta']['opacity'] != 'Transparent' or any(call in parent_calls for call in ids)
                or [call for call in parent_calls if call in (prefix['def_id'], question)]
                != [prefix['def_id'], question] or load in parent_calls):
            raise ValueError('whole parent must call the factored input block before source construction')
        check_parent_gate(body, translated, prefix_call, question)
        details = dict(reader=reader, load_sources=load, snapshot_current=question)
        selected.append(parent)
    module = MODULES[mode]
    mappings = json.loads((generated / 'translation.json').read_bytes())['functions']
    body = (generated / module / 'Funs.lean').read_text()
    for function in selected:
        native = next(row for row in mappings if row['def_id'] == function['def_id']
                      and '_loop' not in row['lean_name'])
        if (native['is_opaque'] is not False or native['source']['file'] != prepare.SOURCE
                or not re.search(r'^\s*def\s+' + re.escape(native['lean_name'].removeprefix(module + '.'))
                                 + r'\s', body, re.M)):
            raise ValueError('generated frontier definition differs')
    return dict(frontier=prefix['def_id'], prefix_calls=ids, module=module,
                lean_name=module + '.controller.lifecycle_inbox.source_work.' + HELPER, **details)
