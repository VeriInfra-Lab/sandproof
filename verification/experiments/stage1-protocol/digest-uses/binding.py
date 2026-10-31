"""Fixed two-call source/MIR admission; not kernel implementation refinement."""
from pathlib import Path
import importlib.util

ROOT = Path(__file__).resolve().parents[4]
provider = ROOT/'verification/experiments/stage1-protocol/serde-binding/binding.py'
spec = importlib.util.spec_from_file_location('fixed_sha_original_observer', provider)
original = importlib.util.module_from_spec(spec)
spec.loader.exec_module(original)


class Original(original.Original):
    # Current fixed Charon stores the optional subslice marker explicitly.
    # All other existing type/trait/constant observation code is reused.
    def type_name(self, reference):
        value = self.resolve(reference, self.types)
        if 'Slice' in value:
            element, marker = value['Slice']
            assert marker is None
            return '['+self.type_name(element)+']'
        return super().type_name(reference)


def path(name):
    return [{'Ident':[part,0]} for part in name.split('::')]


def function(observation, name):
    values = [f for f in observation.functions.values() if f['item_meta']['name'] == path(name)]
    assert len(values) == 1
    value = values[0]
    assert value['item_meta']['opacity'] == 'Transparent' and 'Structured' in value['body']
    assert not value['generics']['types'] and not value['generics']['const_generics']
    assert not value['generics']['trait_clauses']
    return value


def calls(function):
    return [row['kind']['Call']['call'] for row in function['body']['Structured']['body']['statements']
        if isinstance(row['kind'],dict) and 'Call' in row['kind']]


def call_interface(observation, call):
    generics = call['func']['Regular']['generics']
    assert not generics['const_generics']
    dictionaries = []
    for ref in generics['trait_refs']:
        value = observation.resolve(ref, observation.traits)
        assert 'TraitImpl' in value['kind']
        summary = observation.dictionary(ref)
        dictionaries.append(dict(trait=summary['trait'], types=summary['types']))
    return dict(callee=observation.callee(call['func']),
        types=[observation.type_name(t) for t in generics['types']], dictionaries=dictionaries)


def admit(raw, expected_interfaces):
    assert raw['has_errors'] is False
    options = raw['translated']['options']
    assert not any(options[n] for n in ['exclude','skip_borrowck','no_typecheck','monomorphize','remove_unused_clauses'])
    new = Original(raw)
    leaf = function(new,'sandproof_controller_service::controller::lifecycle_inbox::sha256::digest')
    actual_calls = calls(leaf)
    assert len(actual_calls) == 2
    new_interfaces = [call_interface(new,c) for c in actual_calls]
    assert new_interfaces == expected_interfaces
    assert [c['callee'] for c in new_interfaces] == ['digest::digest::<impl>::digest', 'core::convert::<impl>::into']
    assert new_interfaces[0]['types'][1] == '&[U8]'
    assert 'sha2::core_api::Sha256VarCore' in new_interfaces[0]['types'][0]
    assert 'sha2::OidSha256' in new_interfaces[0]['types'][0]
    assert new_interfaces[1]['types'][0].startswith('generic_array::GenericArray<U8,')
    assert new_interfaces[1]['types'][1] == '[U8;32]'
    assert new_interfaces[0]['dictionaries'][-1]['trait'] == 'core::convert::AsRef'
    assert [d['trait'] for d in new_interfaces[1]['dictionaries']] == ['core::convert::From']
    for interface in new_interfaces:
        assert all('parameter:' not in t for t in interface['types'])
        assert all('parameter:' not in t for d in interface['dictionaries'] for t in d['types'])
    body = leaf['body']['Structured']
    assert body['locals']['arg_count'] == 1
    assert new.type_name(leaf['signature']['inputs'][0]) == '&[U8]'
    assert new.type_name(leaf['signature']['output']) == '[U8;32]'
    aliases = {1: {'original_complete_readonly_input': True}}
    writes = []
    returned = False
    for row in body['body']['statements']:
        kind = row['kind']
        if isinstance(kind,str):
            assert kind == 'Return' and not returned
            returned = True
        elif set(kind) in ({'StorageLive'},{'StorageDead'}):
            pass
        elif 'Assign' in kind:
            assert not returned
            destination, expression = kind['Assign']
            assert set(destination['kind']) == {'Local'}
            assert set(expression) == {'Use'} and expression['Use'][1] == 'Yes'
            operand = expression['Use'][0]
            assert set(operand) == {'Copy'} and set(operand['Copy']['kind']) == {'Local'}
            assert new.type_name(operand['Copy']['ty']) == '&[U8]'
            assert new.type_name(destination['ty']) == '&[U8]'
            aliases[destination['kind']['Local']] = aliases[operand['Copy']['kind']['Local']]
            writes.append(destination['kind']['Local'])
        elif 'Call' in kind:
            assert not returned
            call = kind['Call']['call']
            assert len(call['args']) == 1 and set(call['args'][0]) == {'Move'}
            argument = call['args'][0]['Move']['kind']
            assert set(argument) == {'Local'} and set(call['dest']['kind']) == {'Local'}
            if call is actual_calls[0]:
                assert aliases[argument['Local']] == {'original_complete_readonly_input': True}
                aliases[call['dest']['kind']['Local']] = {'entire_digest_output': True}
            else:
                assert call is actual_calls[1]
                assert aliases[argument['Local']] == {'entire_digest_output': True}
                assert call['dest']['kind'] == {'Local':0}
                aliases[0] = {'entire_array32_conversion': True}
        else:
            raise ValueError('unexpected instruction, branch, side effect or projection in fixed leaf')
    assert returned and len(writes) == 1 and aliases[0] == {'entire_array32_conversion': True}
    return dict(complete=True,original_complete_readonly_input_forwarded=True,
        fixed_interfaces=new_interfaces,complete_array32_return=True,
        no_business_branch_object_access_or_extra_call=True,
        kernel_implementation_refinement=False,normal_library_conformance_explicit=True)
