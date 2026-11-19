"""Check the normal ownership and Drop edges in the actual Charon output."""
from frontier_capture import call_kind, indexed
from observation_finish_capture import arm_condition, local, origin
from prepare_transaction_capture import CRATE, TRANSACTION, WORK, method
from revision_capture import declaration, ir_values, regular_calls


DROP = TRANSACTION + '::{impl core::ops::drop::Drop for ' + TRANSACTION + '::WorldTransaction}::drop'
MEMO = TRANSACTION + '::memo::rollback_if_live'


def named(fn):
    return [part['Ident'][0] for part in fn['item_meta']['name'] if 'Ident' in part]


def field(types, name, member):
    value = declaration(types, name)
    return next(index for index, entry in enumerate(value['kind']['Struct']) if entry['name'] == member)


def route_table(translated, body, tx, commit, save_trait, hook_trait, drop_glue):
    variants = {item['def_id']: {variant['id']: variant['name'] for variant in item['kind']['Enum']}
                for item in translated['type_decls'] if item and isinstance(item['kind'], dict)
                and 'Enum' in item['kind']}
    values = {item[0]: item[1][0] for item in ir_values(translated, 'Value')
              if isinstance(item, list) and len(item) == 2 and isinstance(item[1], list)
              and item[1] and isinstance(item[1][0], dict) and {'Discriminant', 'Bool'} & item[1][0].keys()}
    traits = {item[0]: item[1] for item in ir_values(translated, 'Value')
              if isinstance(item, list) and len(item) == 2 and isinstance(item[1], dict)
              and 'trait_decl_ref' in item[1]}
    routes = {}

    def walk(statements, guards, events, moved):
        for index, statement in enumerate(statements):
            step = statement['kind']
            if step == 'Return':
                if guards in routes:
                    raise ValueError('ambiguous normal finish route')
                routes[guards] = events
                return
            if not isinstance(step, dict):
                continue
            if 'Assign' in step:
                target, expression = step['Assign']
                if target['kind'] == {'Local': tx}:
                    raise ValueError('the original owner cannot be assigned a replacement')
                if any(value['kind'] == {'Local': tx} for value in ir_values(expression, 'Move')):
                    if moved is not None:
                        raise ValueError('the original owner moved more than once')
                    moved = local(target)
            if 'Call' in step:
                call = step['Call']['call']
                kind = call_kind(call)
                if any(value['kind'] == {'Local': tx} for value in ir_values(call['args'], 'Move')):
                    raise ValueError('the original owner must only move through its commit temporary')
                if kind.get('Fun') == commit:
                    (operand,) = call['args']
                    if moved is None or operand.get('Move', {}).get('kind') != {'Local': moved}:
                        raise ValueError('commit must consume the exact original owner')
                    events += ('commit',)
                if 'Trait' in kind:
                    reference, method = kind['Trait']
                    value = reference['Value'][1] if 'Value' in reference else traits[reference['Deduplicated']]
                    trait = value['trait_decl_ref']['skip_binder']['id']
                    if method != 0 or trait not in (save_trait, hook_trait):
                        raise ValueError('unexpected call on a normal finish route')
                    label = 'save' if trait == save_trait else 'hook'
                    events += (label,)
            if 'Drop' in step and step['Drop']['place']['kind'] == {'Local': tx}:
                drop = step['Drop']
                if drop['kind'] != 'Conditional' or drop['fn_ptr']['kind'] != {'Fun': drop_glue}:
                    raise ValueError('the actual conditional original-owner Drop is required')
                if moved is not None and 'commit' not in events:
                    raise ValueError('a precommit return lost the original owner')
                events += ('drop-live' if moved is None else 'drop-after-move',)
            if 'Switch' in step:
                switch = step['Switch']
                for branch, child in enumerate(switch['branches']):
                    condition = arm_condition(switch, branch, values, variants)[1]
                    walk(child['statements'] + statements[index + 1:], (*guards, condition), events, moved)
                return

    walk(body['statements'], (), (), None)
    expected = {
        ('Break',): ('save', 'drop-live'),
        ('Continue', 'Break'): ('save', 'hook', 'drop-live'),
        ('Continue', 'Continue', 'Break'): ('save', 'hook', 'commit', 'drop-after-move'),
        ('Continue', 'Continue', 'Continue', 'Break'): ('save', 'hook', 'commit', 'hook', 'drop-after-move'),
        ('Continue', 'Continue', 'Continue', 'Continue'): ('save', 'hook', 'commit', 'hook', 'drop-after-move'),
    }
    if routes != expected:
        raise ValueError('the exact normal save/BeforeCommit/commit/AfterCommit ownership routes changed')
    return [{'guards': sorted_route, 'events': routes[sorted_route]} for sorted_route in sorted(routes)]


def straight_parts(fn):
    assignments, calls, aliases = {}, [], {}
    for statement in fn['body']['Structured']['body']['statements']:
        step = statement['kind']
        if not isinstance(step, dict):
            continue
        if 'Assign' in step:
            place, value = step['Assign']
            if 'Local' not in place['kind']:
                continue
            assignments[local(place)] = value
            argument = value.get('Use', [{}])[0]
            ref = value.get('Ref', {}).get('place') or argument.get('Move') or argument.get('Copy')
            if ref is not None:
                aliases[local(place)] = ref
        if 'Call' in step:
            calls.append(step['Call']['call'])
    return assignments, calls, aliases


def check_drop(translated):
    functions, types, traits = (translated[key] for key in ('fun_decls', 'type_decls', 'trait_decls'))
    finish = declaration(functions, WORK + '::finish_source_work')
    structured = finish['body']['Structured']
    (tx,) = (row['index'] for row in structured['locals']['locals'] if row['name'] == 'tx')
    drop_sites = [row for row in ir_values(structured['body'], 'Drop') if row['place']['kind'] == {'Local': tx}]
    (glue,) = {row['fn_ptr']['kind']['Fun'] for row in drop_sites}
    drop = indexed(functions, glue)
    (implementation,) = list(regular_calls(drop['body']['Structured']['body']))
    concrete = indexed(functions, implementation)
    if (named(concrete) != [*TRANSACTION.split('::'), 'drop']
            or concrete['item_meta']['opacity'] != 'Transparent'):
        raise ValueError('the original Drop glue must call actual WorldTransaction::drop')
    commit = method(functions, TRANSACTION, 'commit')['def_id']
    save_trait = declaration(traits, WORK + '::SaveSourceInputs')['def_id']
    hook_trait = declaration(traits, CRATE + '::CrashInjector')['def_id']
    routes = route_table(translated, structured['body'], tx, commit, save_trait, hook_trait, glue)

    memo = declaration(functions, MEMO)['def_id']
    assignments, calls, aliases = straight_parts(concrete)
    (invoke,) = calls
    if call_kind(invoke) != {'Fun': memo}:
        raise ValueError('production Drop must call the original rollback core once')
    owner, cells, closure = [origin(arg['Move'], aliases, {1}) for arg in invoke['args']]
    transaction = TRANSACTION + '::WorldTransaction'
    if owner != (1, field(types, transaction, 'transaction')) or len(cells) != 1 or len(closure) != 1:
        raise ValueError('rollback must take the exact live transaction slot and original callback')
    tag, arguments = assignments[cells[0]]['Aggregate']
    fields = [origin(arg['Move'], aliases, {1}) for arg in arguments]
    if fields != [(1, field(types, transaction, 'committed'),
                   field(types, TRANSACTION + '::CommittedWorld', 'value')),
                  (1, field(types, transaction, 'candidate'))]:
        raise ValueError('the original committed and candidate cells must be used')
    cell_trait = declaration(traits, TRANSACTION + '::memo::Cells')['def_id']
    references = [item['Value'][1] for item in invoke['func']['Regular']['generics']['trait_refs'] if 'Value' in item]
    (cell_ref,) = (item for item in references if item['trait_decl_ref']['skip_binder']['id'] == cell_trait)
    (callback,) = (item for item in references if item is not cell_ref)

    def dictionary_method(reference, slot):
        instance = indexed(translated['trait_impls'], reference['kind']['TraitImpl']['id'])
        (entry,) = (item for item in instance['methods'] if item['kind']['TraitMethod'][1] == slot)
        return indexed(functions, entry['skip_binder']['id'])

    call_once = dictionary_method(callback, 0)
    _, calls, aliases = straight_parts(call_once)
    (rollback,) = calls
    called = indexed(functions, call_kind(rollback)['Fun'])
    if (named(called) != ['rusqlite', 'transaction', 'rollback']
            or [origin(arg['Move'], aliases, {2}) for arg in rollback['args']] != [(2, 0)]
            or local(rollback['dest']) != 0):
        raise ValueError('the original SQLite rollback must consume the exact callback transaction and return its result')

    store = dictionary_method(cell_ref, 0)
    _, calls, aliases = straight_parts(store)
    borrow, deref = calls
    steps = [row['kind']['Assign'] for row in store['body']['Structured']['body']['statements']
             if isinstance(row['kind'], dict) and 'Assign' in row['kind']]
    for operation in (borrow, deref):
        aliases[local(operation['dest'])] = operation['args'][0]['Move']
    writes = [(origin(dst, aliases, {1, 2}), origin(value['Use'][0]['Move'], aliases, {1, 2}))
              for dst, value in steps if 'Projection' in dst['kind']]
    if (tag['Adt'][0]['id'] != declaration(types, TRANSACTION + '::TransactionCells')['def_id']
            or writes != [((1, field(types, TRANSACTION + '::TransactionCells', 'committed')), (2,))]
            or [named(indexed(functions, call_kind(call)['Fun']))[-1] for call in calls] != ['borrow_mut', 'deref_mut']):
        raise ValueError('set_committed must write only the original supplied value to the original memo')
    return dict(routes=routes, owner=tx, glue=glue, drop=implementation, core=memo,
                callback=call_once['def_id'], cell_store=store['def_id'], owner_fields=fields)
