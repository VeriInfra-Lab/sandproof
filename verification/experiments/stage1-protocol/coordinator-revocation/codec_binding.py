"""Bind each StandardRecordCodec read to its public trait slot and original typed JSON call."""
from copy import deepcopy

from codec_receipt import LIB, GENERATED_PREFIX
from journal_entry import CODEC, RECORDING, type_args
from wiring import SERVICE, name, require

TRAIT = RECORDING + '::RecordCodec'
IMPLEMENTATION = '{impl ' + TRAIT + ' for ' + CODEC + '}'
SOURCE = 'controller-service/src/controller/lifecycle_inbox/recording.rs'
READS = {
    'decode_maintenance': SERVICE + '::controller::checkpoint::config::MaintenanceConfig',
    'decode_inbox': SERVICE + '::controller::lifecycle_inbox::model::Inbox',
}
EXTERNAL_TYPES = {'rusqlite.Connection', 'serde_json.error.Error',
                  'controller.checkpoint.config.MaintenanceConfig', 'controller.lifecycle_inbox.model.Inbox'}
EXTERNAL_FUNCTIONS = {'serde_json.de.from_slice',
                      'controller.checkpoint.config.MaintenanceConfig.Insts.Serde_coreDeDeserialize.deserialize',
                      'controller.lifecycle_inbox.model.Inbox.Insts.Serde_coreDeDeserialize.deserialize'}


def methods(view):
    """Match trait and implementation methods by their bound item id in this capture."""
    impl, = [row for row in view.impls.values()
             if name(view.trait_decls[row['impl_trait']['id']]) == TRAIT
             and type_args(view, row['impl_trait']['generics']) == [CODEC]]
    trait = view.trait_decls[impl['impl_trait']['id']]
    declared = {tuple(m['kind']['TraitMethod']): m['skip_binder']['name'] for m in trait['methods']}
    result = {}
    for method in impl['methods']:
        slot = tuple(method['kind']['TraitMethod'])
        field = declared[slot]
        row = view.functions[method['skip_binder']['id']]
        source = row['src']['TraitImpl']
        require(name(row) == RECORDING+'::{impl}::'+field and source['impl_ref']['id'] == impl['def_id']
                and slot == (trait['def_id'], source['item_id']) and field not in result,
                'concrete codec method differs from its trait slot')
        result[field] = row
    require(set(result) == {*READS, 'encode_inbox'}, 'incomplete concrete codec dictionary')
    return result


def source_span(view, method, contents):
    loc = method['item_meta']['span']['Untagged']['data']
    source = view.data['files'][loc['file_id']]
    require(source['name'] == {'Local': SOURCE} and source['contents'] == contents,
            'codec method source differs')
    return loc['beg'], loc['end'], method['item_meta']['source_text']


def check_read(view, method, expected_type):
    body = method['body']['Structured']
    kinds = [statement['kind'] for statement in body['body']['statements']]
    tags = [next(iter(kind)) if isinstance(kind, dict) else kind for kind in kinds]
    require(method['item_meta']['opacity'] == 'Transparent' and body['locals']['arg_count'] == 3
            and tags == ['StorageLive', 'StorageLive', 'Assign', 'Call', *['StorageDead']*4, 'Return'],
            'unexpected concrete decoder body')
    dest, rhs = kinds[2]['Assign']
    borrow = rhs['Ref']
    data, deref = borrow['place']['kind']['Projection']
    length, metadata = borrow['ptr_metadata']['Copy']['kind']['Projection']
    require(borrow['kind'] == 'Shared' and deref == 'Deref' and data['kind'] == {'Local': 3}
            and metadata == 'PtrMetadata' and length['kind'] == data['kind'], 'decoder slice or length differs')
    call = kinds[3]['Call']['call']
    fun = call['func']['Regular']
    require(dest['kind'] == {'Local': 4} and [a['Move']['kind'] for a in call['args']] == [dest['kind']]
            and call['dest']['kind'] == {'Local': 0}
            and name(view.functions[fun['kind']['Fun']]) == 'serde_json::de::from_slice',
            'decoder argument, callee or return differs')
    generics = fun['generics']
    ref, = generics['trait_refs']
    dictionary = view.dictionary(ref)
    require(type_args(view, generics) == [expected_type] and dictionary['trait'] == 'serde_core::de::Deserialize'
            and dictionary['types'] == [expected_type] and dictionary['implementation']['self_type'] == expected_type
            and view.direct_name(dictionary['implementation']['source_name'][:-2]) == expected_type.rsplit('::', 1)[0],
            'decoder type or original derive dictionary differs')


def admit(view, public, mapping, original, public_source):
    readers, dispatch = methods(view), methods(public)
    selected = {}
    for field, expected in READS.items():
        row = readers[field]
        require(source_span(view, row, original) == source_span(public, dispatch[field], public_source),
                'public journal and decoder select different production methods')
        check_read(view, row, expected)
        translated, = [f for f in mapping['functions'] if f['def_id'] == row['def_id']]
        require(translated['is_opaque'] is False and translated['lean_file'] == LIB+'/Funs.lean'
                and translated['lean_name'] == GENERATED_PREFIX+field, 'missing generated concrete read')
        selected[field] = translated['lean_name']
    for kind, expected in [('functions', EXTERNAL_FUNCTIONS), ('types', EXTERNAL_TYPES)]:
        observed = {f['lean_name'] for f in mapping[kind] if f['lean_file'].endswith('External_Template.lean')}
        require(observed == expected, 'unexpected decoder external '+kind)
    return selected


def check_refusals(view):
    method = methods(view)['decode_inbox']
    wrong = deepcopy(method)
    statements = wrong['body']['Structured']['body']['statements']
    length = statements[2]['kind']['Assign'][1]['Ref']['ptr_metadata']['Copy']['kind']['Projection']
    length[0]['kind'] = {'Local': 2}
    for row, typ, diagnostic in [(wrong, READS['decode_inbox'], 'decoder slice or length differs'),
                                 (method, READS['decode_maintenance'], 'decoder type or original derive dictionary differs')]:
        try:
            check_read(view, row, typ)
        except ValueError as error:
            if str(error) == diagnostic:
                continue
            raise
        raise ValueError('decoder checker accepted an incorrect slice length or derive binding')
