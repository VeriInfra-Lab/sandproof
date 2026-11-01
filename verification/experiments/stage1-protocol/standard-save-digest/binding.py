"""Exact parent SHA transfer checks, with a strict original Save receiver selector.
This is source/MIR admission only, not a library or compiler refinement proof.
"""
from pathlib import Path
import importlib.util
import json

ROOT=Path(__file__).resolve().parents[4]
HERE=Path('verification/experiments/stage1-protocol/standard-save-digest')
spec=importlib.util.spec_from_file_location('save_current_full_SHA_interfaces',ROOT/HERE.parent/'standard-digest/binding.py')
parent=importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)
Original,calls,interface=parent.Original,parent.calls,parent.interface


def single(raw,name,argc):
    if argc==1:
        return parent.single(raw,name,argc)
    if name!='digest' or argc!=2:
        raise ValueError('only the actual original Save method is selected')
    observer=Original(raw)
    expected='&mut sandproof_controller_service::controller::lifecycle_inbox::codec::StandardInboxSave'
    matches=[f for f in observer.functions.values()
        if f['item_meta']['name'][-1]=={'Ident':['digest',0]}
        and f['item_meta']['opacity']=='Transparent'
        and 'Structured' in f.get('body',{})
        and f['body']['Structured']['locals']['arg_count']==2
        and observer.type_name(f['signature']['inputs'][0])==expected]
    if len(matches)!=1:
        raise ValueError('missing or ambiguous actual original Save receiver')
    function=matches[0]
    if (function['signature']['is_unsafe']
            or function['signature']['is_variadic']
            or observer.type_name(function['signature']['inputs'][1])!='&[U8]'
            or observer.type_name(function['signature']['output'])!='[U8;32]'):
        raise ValueError('changed complete original Save method signature')
    return observer,function


def admit(raw, original_method=False):
    if raw['has_errors']:
        raise ValueError('incomplete original compiler output')
    observer, function = single(raw, 'digest' if original_method else 'sha256_bytes',
                                2 if original_method else 1)
    operations = calls(function)
    expected = json.loads((ROOT/HERE.parent/'standard-digest/fixed-interfaces.json').read_text())
    observed = [interface(observer, call) for call in operations]
    if observed != expected or len(operations) != 2:
        raise ValueError('actual fixed SHA/GenericArray/AsRef interface differs')
    aliases = {2 if original_method else 1:'entire-readonly-input'}
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
        complete_array32=True, original_receiver_unused=original_method,
        extra_object_access_or_business_branch=False, library_conformance_proved=False,
        kernel_implementation_refinement=False)
