"""Existing complete fixed-library interface observer, applied to actual SSA.

This is a narrow source/MIR admission check, not a Lean compiler refinement.
"""
from pathlib import Path
import importlib.util
import json

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-digest')
provider = ROOT/HERE.parent/'serde-binding/binding.py'
spec = importlib.util.spec_from_file_location('standard_digest_original_observer', provider)
original = importlib.util.module_from_spec(spec)
spec.loader.exec_module(original)


class Original(original.Original):
    def type_name(self, reference):
        value = self.resolve(reference, self.types)
        if 'Slice' in value:
            element, marker = value['Slice']
            if marker is not None:
                raise ValueError('changed original slice representation')
            return '['+self.type_name(element)+']'
        return super().type_name(reference)


def single(raw, name, argc):
    observer = Original(raw)
    functions = [f for f in observer.functions.values()
        if f['item_meta']['name'] == [{'Ident':[part,0]} for part in name.split('::')]
        and f['item_meta']['opacity'] == 'Transparent'
        and 'Structured' in f.get('body', {})
        and f['body']['Structured']['locals']['arg_count'] == argc]
    if len(functions) != 1:
        raise ValueError('missing or ambiguous original transparent function')
    return observer, functions[0]


def calls(function):
    return [row['kind']['Call']['call'] for row in function['body']['Structured']['body']['statements']
            if isinstance(row['kind'], dict) and 'Call' in row['kind']]


def interface(observer, call):
    generics = call['func']['Regular']['generics']
    if generics['const_generics']:
        raise ValueError('unexpected original library const argument')
    dictionaries = []
    for reference in generics['trait_refs']:
        value = observer.resolve(reference, observer.traits)
        if 'TraitImpl' not in value['kind']:
            raise ValueError('generic or substituted original library dictionary')
        summary = observer.dictionary(reference)
        dictionaries.append(dict(trait=summary['trait'], types=summary['types']))
    return dict(callee=observer.callee(call['func']),
        types=[observer.type_name(t) for t in generics['types']], dictionaries=dictionaries)


def admit(raw):
    if raw['has_errors']:
        raise ValueError('incomplete original compiler output')
    observer, function = single(raw,
        'sandproof_controller_service::controller::lifecycle_inbox::sha256::digest', 1)
    operations = calls(function)
    expected = json.loads((ROOT/HERE/'fixed-interfaces.json').read_text())
    observed = [interface(observer, call) for call in operations]
    if observed != expected or len(operations) != 2:
        raise ValueError('actual fixed SHA/GenericArray/AsRef interface differs')
    aliases = {1:'entire-readonly-input'}
    writes, returned = [], False
    for row in function['body']['Structured']['body']['statements']:
        kind = row['kind']
        if isinstance(kind, str):
            if kind != 'Return' or returned:
                raise ValueError('extra control flow in library delegate')
            returned = True
        elif set(kind) in ({'StorageLive'}, {'StorageDead'}):
            pass
        elif 'Assign' in kind:
            destination, expression = kind['Assign']
            if (returned or set(destination['kind']) != {'Local'}
                    or set(expression) != {'Use'} or expression['Use'][1] != 'Yes'):
                raise ValueError('changed data transfer or object write')
            operand = expression['Use'][0]
            if (set(operand) != {'Copy'} or set(operand['Copy']['kind']) != {'Local'}
                    or observer.type_name(operand['Copy']['ty']) != '&[U8]'
                    or observer.type_name(destination['ty']) != '&[U8]'):
                raise ValueError('not the original complete shared slice')
            aliases[destination['kind']['Local']] = aliases[operand['Copy']['kind']['Local']]
            writes.append(destination['kind']['Local'])
        elif 'Call' in kind:
            call = kind['Call']['call']
            if (returned or len(call['args']) != 1 or set(call['args'][0]) != {'Move'}
                    or set(call['args'][0]['Move']['kind']) != {'Local'}
                    or set(call['dest']['kind']) != {'Local'}):
                raise ValueError('changed original library arguments or result')
            argument = call['args'][0]['Move']['kind']['Local']
            if call is operations[0]:
                if aliases[argument] != 'entire-readonly-input':
                    raise ValueError('library input omitted or projected')
                aliases[call['dest']['kind']['Local']] = 'entire-raw-digest'
            else:
                if (call is not operations[1] or aliases[argument] != 'entire-raw-digest'
                        or call['dest']['kind'] != {'Local':0}):
                    raise ValueError('digest conversion omitted or projected')
                aliases[0] = 'entire-array32'
        else:
            raise ValueError('extra branch, call, projection, effect or object access')
    if not returned or len(writes) != 1 or aliases.get(0) != 'entire-array32':
        raise ValueError('original complete return missing')
    return dict(complete=True, exact_fixed_interfaces=observed, complete_input=True,
        complete_array32=True, unique_production_leaf=True,
        extra_object_access_or_business_branch=False, library_conformance_proved=False,
        kernel_implementation_refinement=False)
