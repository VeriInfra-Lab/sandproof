"""Admit complete argument forwarding and retained carrier fields; no IO semantics."""
PREFIX = 'sandproof_controller_service::controller::lifecycle_inbox::'


def admit(raw):
    def name(path):
        return [{'Ident': [part, 0]} for part in path.split('::')]

    functions = [f for f in raw['translated']['fun_decls'] if f]

    def function(short):
        found = [f for f in functions if f['item_meta']['name'] == name(PREFIX+short)]
        if len(found) != 1 or found[0]['item_meta']['opacity'] != 'Transparent' or 'Structured' not in found[0]['body']:
            raise ValueError('missing or external actual parent/root '+short)
        return found[0]

    root, parent = function('proof_attach_create_using'), function('attach_create_using')
    calls, assignments = [], []

    def walk(node):
        if isinstance(node, dict):
            if 'Call' in node and isinstance(node['Call'], dict) and 'call' in node['Call']:
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
        raise ValueError('root does not forward exactly the three original arguments')
    call = calls[0]
    if call['func']['Regular']['kind']['Fun'] != parent['def_id'] or call['dest']['kind'] != {'Local': 0} or len(call['args']) != 3:
        raise ValueError('wrong actual target, return place or argument count')
    # The existing error-carrier interner intentionally collects only Adt and
    # Scalar. This call also needs original generic TypeVar references.
    type_table, trait_table = {}, {}
    def intern(node):
        if isinstance(node, dict):
            value = node.get('Value')
            if isinstance(value, list) and len(value) == 2 and isinstance(value[0], int):
                index, content = value
                if isinstance(content, dict) and set(content) in ({'Adt'},{'Scalar'},{'TypeVar'}):
                    type_table[index] = content
                if isinstance(content, dict) and set(content) == {'kind','trait_decl_ref'}:
                    trait_table[index] = content
            for child in node.values(): intern(child)
        elif isinstance(node, list):
            for child in node: intern(child)
    intern(raw)
    def resolve(value, table):
        if set(value) == {'Value'}: return value['Value'][1]
        if set(value) == {'Deduplicated'}: return table[value['Deduplicated']]
        raise ValueError('unreviewed interned argument')
    wanted = ['R','T','N','U','B','E']
    if [x['name'] for x in root['generics']['types']] != wanted or [x['name'] for x in parent['generics']['types']] != wanted:
        raise ValueError('original six generic parameters changed')
    if [x['index'] for x in root['generics']['types']] != list(range(6)) or [x['index'] for x in parent['generics']['types']] != list(range(6)):
        raise ValueError('generic parameter indices changed')
    arguments = call['func']['Regular']['generics']
    if (len(arguments['types']) != 6 or len(arguments['trait_refs']) != 6 or arguments['const_generics']
            or len(root['generics']['trait_clauses']) != 6 or len(parent['generics']['trait_clauses']) != 6):
        raise ValueError('lost or additional original generic/dictionary argument')
    for index, value in enumerate(arguments['types']):
        if resolve(value,type_table) != {'TypeVar':{'Free':index}}:
            raise ValueError('reordered, specialized or changed original type argument')
    for index, value in enumerate(arguments['trait_refs']):
        supplied = resolve(value,trait_table)
        declared = root['generics']['trait_clauses'][index]
        destination = parent['generics']['trait_clauses'][index]
        if supplied['kind'] != {'Clause':{'Free':index}} or declared['clause_id'] != index or destination['clause_id'] != index:
            raise ValueError('changed original dictionary clause')
        for reference in (supplied['trait_decl_ref'],declared['trait_'],destination['trait_']):
            declaration = reference['skip_binder']; generics = declaration['generics']
            if (reference['regions'] or declaration['id'] != supplied['trait_decl_ref']['skip_binder']['id']
                    or generics['regions'] or generics['const_generics'] or generics['trait_refs']
                    or len(generics['types']) != 1 or resolve(generics['types'][0],type_table) != {'TypeVar':{'Free':index}}):
                raise ValueError('changed dictionary declaration or complete Self type')
    forwarding = []
    for index, (argument, assignment) in enumerate(zip(call['args'], assignments), 1):
        destination, expression = assignment
        if argument.get('Move', {}).get('kind') != destination['kind']:
            raise ValueError('argument is not the original forwarding local')
        if 'Ref' in expression:
            reference = expression['Ref']
            projection = reference.get('place', {}).get('kind', {}).get('Projection')
            if (not projection or projection[0]['kind'] != {'Local': index} or projection[1] != 'Deref'
                    or reference['kind'] not in (('Mut','TwoPhaseMut') if index == 1 else ('Shared',))):
                raise ValueError('projected, reordered or changed complete borrow')
            forwarding.append(dict(input=index, complete_borrow=True, kind=reference['kind']))
        else:
            copy = expression.get('Use', [None, None])
            if copy[1] != 'Yes' or copy[0].get('Copy', {}).get('kind') != {'Local': index}:
                raise ValueError('projected or changed original reference copy')
            forwarding.append(dict(input=index, complete_original_copy=True))
    types = [t for t in raw['translated']['type_decls'] if t]
    retained = {}
    for path, expected in [
        (PREFIX+'CreateAttachment', ['intent','local','registered_template']),
        ('sandproof_controller_service::templates::TemplateBinding',
         ['template_id','revision','owner','kernel','rootfs','resources','envd_version','guest','startup']),
        ('sandproof_controller_service::templates::TemplateStartup', ['env_vars','user','workdir','argv']),
        ('sandproof_controller_service::templates::TemplateArtifact', ['path','sha256','size_bytes']),
        ('sandproof_controller_service::templates::ResourceShape', ['cpu_count','memory_mib','disk_bytes','host_memory_bytes'])]:
        found = [t for t in types if t['item_meta']['name'] == name(path)]
        if len(found) != 1 or 'Struct' not in found[0]['kind']:
            raise ValueError('missing actual complete carrier '+path)
        fields = found[0]['kind']['Struct']
        if [f['name'] for f in fields] != expected:
            raise ValueError('actual fields omitted or reordered in '+path)
        retained[path] = dict(def_id=found[0]['def_id'], fields=fields)
    return dict(root=root['def_id'], actual=parent['def_id'], forwarding=forwarding,
        retained_raw_types=retained, original_generic_order=wanted, all_six_original_types_and_dictionaries=True, raw_field_retention_only=True,
        pathbuf_btreemap_representation_proved=False, standard_instances_proved=False,
        equivalence_or_parent_semantics_proved=False)
