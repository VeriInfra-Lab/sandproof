"""Inspect original standard Serialize bodies for a source-reviewed A6 binding.

This is an admission/reporting tool, not a Rust interpreter or proof translator.
It retains the complete LLBC separately; summaries never replace that input.
"""
import hashlib
import json


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


class Original:
    def __init__(self, raw):
        require(raw.get('has_errors') is False, 'incomplete Charon input')
        self.raw = raw
        self.data = raw['translated']
        self.types, self.traits, self.constants = {}, {}, {}
        self.functions = {d['def_id']: d for d in self.data['fun_decls'] if d}
        self.type_decls = {d['def_id']: d for d in self.data['type_decls'] if d}
        self.trait_decls = {d['def_id']: d for d in self.data['trait_decls'] if d}
        self.impls = {d['def_id']: d for d in self.data['trait_impls'] if d}
        self._intern(raw)

    def _intern(self, node):
        if isinstance(node, dict):
            if set(node) == {'Value'} and isinstance(node['Value'], list):
                index, value = node['Value']
                if isinstance(value, dict):
                    if set(value) == {'kind', 'trait_decl_ref'}:
                        self.traits[index] = value
                    elif len(value) == 1 and next(iter(value)) in {
                            'Adt', 'Ref', 'RawPtr', 'Scalar', 'TypeVar', 'TraitType',
                            'FnPtr', 'DynTrait', 'Never', 'Array', 'Slice'}:
                        self.types[index] = value
                elif (isinstance(value, list) and len(value) == 2
                      and isinstance(value[0], dict) and isinstance(value[1], dict)
                      and set(value[1]) in ({'Value'}, {'Deduplicated'})):
                    self.constants[index] = value
            for value in node.values():
                self._intern(value)
        elif isinstance(node, list):
            for value in node:
                self._intern(value)

    @staticmethod
    def resolve(reference, table):
        if set(reference) == {'Value'}:
            return reference['Value'][1]
        if set(reference) == {'Deduplicated'}:
            return table[reference['Deduplicated']]
        raise ValueError('unregistered interning format')

    @staticmethod
    def direct_name(parts):
        require(all(set(p) == {'Ident'} and p['Ident'][1] == 0 for p in parts),
                'expected original direct name')
        return '::'.join(p['Ident'][0] for p in parts)

    def type_name(self, reference):
        value = self.resolve(reference, self.types)
        if 'Adt' in value:
            adt = value['Adt']
            if isinstance(adt['id'], int):
                parts = self.type_decls[adt['id']]['item_meta']['name']
                name = (self.direct_name(parts) if all('Ident' in p for p in parts)
                        else 'rust-builtin:' + json.dumps(parts, sort_keys=True))
            else:
                name = json.dumps(adt['id'], sort_keys=True)
            args = [self.type_name(t) for t in adt['generics']['types']]
            consts = adt['generics']['const_generics']
            return name + ('<' + ','.join(args) + '>' if args else '') + (
                '[' + json.dumps(consts, sort_keys=True) + ']' if consts else '')
        if 'Ref' in value:
            _, target, mode = value['Ref']
            return '&' + ('mut ' if mode == 'Mut' else '') + self.type_name(target)
        if 'Array' in value:
            element, length, range_ = value['Array']
            require(range_ is None, 'unreviewed array range')
            return '[' + self.type_name(element) + ';' + str(self.constant(length)['constant']) + ']'
        if 'Slice' in value:
            return '[' + self.type_name(value['Slice']) + ']'
        if 'Scalar' in value:
            scalar = value['Scalar']
            return scalar if isinstance(scalar, str) else next(iter(scalar['Integer'].values()))
        if 'TypeVar' in value:
            return 'parameter:' + json.dumps(value['TypeVar'], sort_keys=True)
        raise ValueError('unreviewed field type: ' + repr(value))

    def dictionary(self, reference):
        value = self.resolve(reference, self.traits)
        declared = value['trait_decl_ref']['skip_binder']
        name = self.direct_name(self.trait_decls[declared['id']]['item_meta']['name'])
        result = {'trait': name, 'types': [self.type_name(t) for t in declared['generics']['types']]}
        if 'TraitImpl' in value['kind']:
            impl = self.impls[value['kind']['TraitImpl']['id']]
            result['implementation'] = dict(id=impl['def_id'], sha256=digest(impl),
                source_name=impl['item_meta']['name'], self_type=self.type_name(impl['impl_trait']['generics']['types'][0]))
        else:
            result['parameter'] = value['kind']
        return result

    def callee(self, function):
        require(set(function) == {'Regular'}, 'indirect function call needs independent review')
        call = function['Regular']
        if 'Trait' in call['kind']:
            reference, index = call['kind']['Trait']
            trait = self.resolve(reference, self.traits)
            declaration = trait['trait_decl_ref']['skip_binder']['id']
            name = self.direct_name(self.trait_decls[declaration]['item_meta']['name'])
            method = self.data['assoc_item_names'][declaration]['methods'][index]
            return name + '::' + method
        require(set(call['kind']) == {'Fun'}, 'unsupported call kind')
        name = self.functions[call['kind']['Fun']]['item_meta']['name']
        # Full original names, generic arguments and cleanup remain in snapshots.
        return '::'.join(p['Ident'][0] if 'Ident' in p else '<impl>' for p in name)

    def constant(self, reference):
        value, _ = self.resolve(reference, self.constants)
        if 'Str' in value:
            return {'constant': value['Str']}
        if 'Bool' in value:
            return {'constant': value['Bool']}
        if 'Integer' in value:
            _, number = next(iter(value['Integer'].values()))
            return {'constant': int(number)}
        return {'original_constant': value}

    def field_name(self, base, field):
        variant, index = field
        ty = self.resolve(base['ty'], self.types)
        require('Adt' in ty, 'field receiver is not the original ADT')
        declaration = self.type_decls[ty['Adt']['id']]
        if variant is None:
            return declaration['kind']['Struct'][index]['name']
        selected = declaration['kind']['Enum'][variant]
        return selected['name'] + '.' + selected['fields'][index]['name']

    def place(self, value, aliases):
        kind = value['kind']
        if 'Local' in kind:
            return aliases.get(kind['Local'], {'local': kind['Local']})
        require(set(kind) == {'Projection'}, 'unreviewed place expression')
        base, projection = kind['Projection']
        origin = self.place(base, aliases)
        if projection == 'Deref':
            if set(origin) == {'borrow', 'origin'}:
                return origin['origin']
            return {'deref': origin}
        require(isinstance(projection, dict) and set(projection) == {'Field'},
                'unreviewed field projection')
        return {'field': self.field_name(base, projection['Field']), 'of': origin}

    def operand(self, value, aliases):
        if 'Const' in value:
            return self.constant(value['Const'])
        require(set(value) in ({'Move'}, {'Copy'}), 'unreviewed operand')
        return self.place(next(iter(value.values())), aliases)

    def assigned(self, expression, aliases):
        if 'Ref' in expression:
            ref = expression['Ref']
            return {'borrow': ref['kind'], 'origin': self.place(ref['place'], aliases)}
        if 'Use' in expression:
            return self.operand(expression['Use'][0], aliases)
        if 'UnaryOp' in expression:
            operation, argument = expression['UnaryOp']
            value = self.operand(argument, aliases)
            if (operation == {'Cast': {'Scalar': ['Bool', {'Integer': {'Unsigned': 'Usize'}}]}}
                    and set(value) == {'constant'} and isinstance(value['constant'], bool)):
                return {'constant': int(value['constant'])}
        if 'BinaryOp' in expression:
            operation, left, right = expression['BinaryOp']
            values = [self.operand(v, aliases) for v in (left, right)]
            if operation == {'Add': 'Panic'} and all(set(v) == {'constant'} for v in values):
                total = sum(v['constant'] for v in values)
                require(0 <= total < 2**32, 'nonportable literal field-count expression')
                return {'constant': total}
        if 'Aggregate' in expression:
            aggregate, fields = expression['Aggregate']
            if 'Adt' in aggregate:
                adt, variant, union_field = aggregate['Adt']
                if variant is None and union_field is None:
                    declaration = self.type_decls[adt['id']]
                    return {'aggregate': self.direct_name(declaration['item_meta']['name']),
                            'fields': [self.operand(f, aliases) for f in fields]}
        # These expressions do not claim an evaluated result; the complete AST
        # is retained and their checksum is included for the source reviewer.
        return {'original_expression_sha256': digest(expression)}

    def branch_label(self, switch, index, aliases):
        data = switch['data']
        subject = data['scrutinee']
        if 'Discriminant' in subject:
            subject = self.place(subject['Discriminant'], aliases)
        elif 'Value' in subject:
            subject = self.operand(subject['Value'], aliases)
        else:
            raise ValueError('unreviewed branch condition')
        labels = []
        for constant, branch in data['branches']:
            if branch != index:
                continue
            value, _ = self.resolve(constant, self.constants)
            if 'Discriminant' in value:
                adt, variant = value['Discriminant']
                declaration = self.type_decls[adt['id']]
                labels.append(self.direct_name(declaration['item_meta']['name']) + '::' +
                              declaration['kind']['Enum'][variant]['name'])
            else:
                labels.append(self.constant(constant))
        return dict(subject=subject, cases=labels, fallback=data['fallback'] == index)

    def calls(self, body):
        rows = []

        def walk(block, aliases, path):
            aliases = dict(aliases)
            for statement in block['statements']:
                kind = statement['kind']
                if not isinstance(kind, dict):
                    continue
                if 'Assign' in kind:
                    place, expression = kind['Assign']
                    if set(place['kind']) == {'Local'}:
                        aliases[place['kind']['Local']] = self.assigned(expression, aliases)
                elif 'Call' in kind:
                    call = kind['Call']['call']
                    row = dict(statement=statement['id'], path=path, callee=self.callee(call['func']),
                        arguments=[self.operand(a, aliases) for a in call['args']],
                        destination=call['dest']['kind'],
                        types=[self.type_name(t) for t in call['func']['Regular']['generics']['types']],
                        dictionaries=[self.dictionary(t) for t in call['func']['Regular']['generics']['trait_refs']],
                        statement_sha256=digest(statement))
                    target = call['func']['Regular']['kind']
                    if 'Fun' in target:
                        declaration = self.functions[target['Fun']]
                        row['target_declaration_sha256'] = digest(declaration)
                        implementations = [p['Impl']['Trait'] for p in declaration['item_meta']['name']
                                           if 'Impl' in p and 'Trait' in p['Impl']]
                        if implementations:
                            implementation, = implementations
                            row['target_self_type'] = self.type_name(
                                self.impls[implementation]['impl_trait']['generics']['types'][0])
                    rows.append(row)
                    if set(call['dest']['kind']) == {'Local'}:
                        aliases[call['dest']['kind']['Local']] = {'call_result': statement['id']}
                elif 'Switch' in kind:
                    switch = kind['Switch']
                    for index, branch in enumerate(switch['branches']):
                        walk(branch, aliases, path + [dict(statement=statement['id'], branch=index,
                            data_sha256=digest(switch['data']),
                            **self.branch_label(switch, index, aliases))])
                elif 'Loop' in kind:
                    raise ValueError('loop in standard derive requires separate binding review')
        walk(body['body'], {1: {'self': True}, 2: {'serializer': True}}, [])
        return rows

    def serialize_bodies(self):
        output = {}
        for function in self.functions.values():
            name = function['item_meta']['name']
            if (name[0] != {'Ident': ['sandproof_protocol', 0]}
                    or name[-1] != {'Ident': ['serialize', 0]}):
                continue
            impl_id, = [p['Impl']['Trait'] for p in name if 'Impl' in p]
            implementation = self.impls[impl_id]
            trait = implementation['impl_trait']
            require(self.direct_name(self.trait_decls[trait['id']]['item_meta']['name']) ==
                    'serde_core::ser::Serialize', 'wrong actual derive dictionary')
            ty = trait['generics']['types'][0]
            self_type = self.type_name(ty)
            require(self_type not in output, 'duplicate Serialize body')
            require(function['item_meta']['opacity'] == 'Transparent' and
                    set(function['body']) == {'Structured'}, 'original Serialize body missing')
            output[self_type] = dict(function_id=function['def_id'], implementation_id=impl_id,
                function_sha256=digest(function), implementation_sha256=digest(implementation),
                signature_sha256=digest(function['signature']),
                calls=self.calls(function['body']['Structured']))
        require(output, 'no protocol Serialize bodies')
        return output


def snapshot(raw):
    original = Original(raw)
    return {'kind': 'original-protocol-serialize-source-binding',
            'library_refinement_proved': False,
            'bodies': original.serialize_bodies(),
            'full_raw_sha256': digest(raw), 'options': original.data['options']}
