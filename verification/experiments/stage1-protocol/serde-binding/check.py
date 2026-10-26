"""Protocol-specific source admission; does not certify Serde in Lean."""
import json
from pathlib import Path
from binding import snapshot, require

PREFIX = 'sandproof_protocol::'
SER = 'serde_core::ser::'
SELF = {'deref': {'self': True}}
IDENTITY = PREFIX + 'lifecycle::identity::'
AUTHORITY = PREFIX + 'lifecycle::authority::'
REQUEST = PREFIX + 'lifecycle::request::'
INTERNAL = PREFIX + 'internal::'
CREATE = INTERNAL + 'lifecycle::'
STRUCTS = {
    'NodeIdentity': INTERNAL, 'Resources': CREATE, 'CreateBinding': CREATE,
    'OperationKey': IDENTITY, 'OperationBinding': IDENTITY,
    'LifecycleRequest': REQUEST, 'CenterIdentity': AUTHORITY,
    'RetentionBinding': IDENTITY, 'AcceptedLifecycleIntent': AUTHORITY,
    'RecordedLifecycleIntent': AUTHORITY,
}
TRANSPARENT = {
    AUTHORITY + 'CenterId': PREFIX + 'DeploymentId',
    IDENTITY + 'SessionId': PREFIX + 'SandboxId',
    **{IDENTITY + name: INTERNAL + 'PositiveCounter' for name in (
        'IncarnationId', 'CaptureId', 'RequestWindowId', 'RequestSequence', 'RetentionWindowId')},
    PREFIX + 'lifecycle::digest::ContentDigest': '[U8;32]',
}
CONVERT = {**{PREFIX + name: 'alloc::string::String' for name in (
    'DeploymentId', 'SandboxId', 'NodeId', 'TenantId', 'ResourceId')},
    INTERNAL + 'PositiveCounter': 'U64'}


def short_type(name):
    if name.startswith('alloc::vec::Vec<'):
        return name.removeprefix('alloc::vec::').lower().replace('vec', 'Vec')
    if name in ('U8', 'U16', 'U32', 'U64', 'I64', '[U8;32]'):
        return name.lower()
    if name == 'Bool':
        return 'bool'
    return name.rsplit('::', 1)[-1]


def path_shape(path):
    return [{key: row[key] for key in ('subject', 'cases', 'fallback')} for row in path]


def continue_or_break(call, which):
    return dict(subject={'call_result': call['statement']},
                cases=['core::ops::control_flow::ControlFlow::' + which], fallback=False)


def original_field(name):
    return {'borrow': 'Shared', 'origin': {'field': name, 'of': SELF}}


def check_stages(calls, name, fields, prefix):
    """Each actual field call uses the same container after prior Ok results.

    Reject changed/extra calls, error swallowing, field aliases, and dictionary
    substitution. The untouched full function snapshots also bind return,
    cleanup and every other statement not summarized here.
    """
    success = [c for c in calls if c['callee'] in (
        SER + 'Serializer::serialize_struct', SER + 'SerializeStruct::serialize_field',
        SER + 'SerializeStruct::end')]
    require(len(success) == len(fields) + 2, 'missing/extra standard field call: ' + name)
    start, *middle, end = success
    require(start['callee'] == SER + 'Serializer::serialize_struct' and
            start['arguments'] == [{'serializer': True}, {'constant': name}, {'constant': len(fields)}],
            'wrong original container, serializer or field count: ' + name)
    require(end['callee'] == SER + 'SerializeStruct::end' and
            end['destination'] == {'Local': 0}, 'original container result not returned')
    consumed = {end['statement']}
    current = prefix
    container = None
    for index, call in enumerate([start, *middle]):
        require(path_shape(call['path']) == current, 'field call not after previous success')
        if index:
            key, origin, ty = fields[index-1]
            require(call['callee'] == SER + 'SerializeStruct::serialize_field' and
                    call['arguments'] == [{'borrow': 'TwoPhaseMut', 'origin': container},
                                          {'constant': key}, origin],
                    'original field/container reference changed: ' + name + '.' + key)
            require(len(call['types']) == 1 and short_type(call['types'][0]) == ty,
                    'field generic type changed: ' + name + '.' + key)
            dictionary, = call['dictionaries']
            require(dictionary['trait'] == SER + 'Serialize' and
                    dictionary['types'] == call['types'] and 'implementation' in dictionary,
                    'field uses a substituted Serialize dictionary')
        branches = [c for c in calls if c['callee'] == 'core::result::<impl>::branch' and
                    c['arguments'] == [{'call_result': call['statement']}] and
                    path_shape(c['path']) == current]
        require(len(branches) == 1, 'missing original Result branching')
        branch, = branches
        errors = [c for c in calls if c['callee'] == 'core::result::<impl>::from_residual' and
                  path_shape(c['path']) == current + [continue_or_break(branch, 'Break')]]
        require(len(errors) == 1, 'missing/extra original error return')
        error, = errors
        require(error['arguments'] == [{'field': 'Break._0', 'of': {'call_result': branch['statement']}}]
                and error['destination'] == {'Local': 0}, 'original error replaced/discarded')
        require(len(error['types']) == 3 and error['types'][1] == error['types'][2],
                'serializer error type converted to a different type')
        dictionary, = error['dictionaries']
        require(dictionary['trait'] == 'core::convert::From' and
                dictionary['types'] == [error['types'][1]] * 2 and
                dictionary['implementation']['source_name'][:2] == [
                    {'Ident': ['core', 0]}, {'Ident': ['convert', 0]}],
                'unreviewed nonidentity error conversion')
        consumed.update((call['statement'], branch['statement'], error['statement']))
        current = current + [continue_or_break(branch, 'Continue')]
        if not index:
            container = {'field': 'Continue._0', 'of': {'call_result': branch['statement']}}
    require(path_shape(end['path']) == current and end['arguments'] == [container],
            'end is not the original complete container')
    require(consumed == {c['statement'] for c in calls}, 'extra normal-path call in derived body')
    return dict(fields=len(fields), error_returns=len(fields)+1, original_container=True)


def check(raw, schema):
    report = snapshot(raw)
    bodies = report['bodies']
    names = {prefix + name for name, prefix in STRUCTS.items()}
    # Accepted-only extraction does not reach Recorded; every other member is
    # needed by both concrete envelopes.
    if AUTHORITY + 'RecordedLifecycleIntent' not in bodies:
        names.remove(AUTHORITY + 'RecordedLifecycleIntent')
    expected = names | set(TRANSPARENT) | set(CONVERT) | {
        IDENTITY + 'ExpectedSession', REQUEST + 'LifecycleOperation'}
    require(set(bodies) == expected, 'missing or unexpected actual Serialize implementation')
    checked = {}
    for name, prefix in STRUCTS.items():
        if prefix + name not in bodies:
            continue
        fields = [(key, original_field(key), ty)
                  for key, ty in schema['structs'][name]['fields_in_declaration_order']]
        checked[prefix + name] = check_stages(bodies[prefix + name]['calls'], name, fields, [])
    operation = REQUEST + 'LifecycleOperation'
    calls = bodies[operation]['calls']
    require(all(c['path'] for c in calls), 'operation derive has calls before original match')
    for variant, fields in schema['operations'].items():
        prefix = [dict(subject=SELF, cases=[operation + '::' + variant], fallback=False)]
        selected = [c for c in calls if path_shape(c['path'][:1]) == prefix]
        fields = [('kind', {'borrow': 'Shared', 'origin': {'deref': {'constant': variant.lower()}}},
                   'rust-builtin:[{"Builtin": ["Str", 0]}]')] + [
            (key, original_field(variant + '.' + key), ty) for key, ty in fields]
        checked[operation + '::' + variant] = check_stages(selected, 'LifecycleOperation', fields, prefix)
    expected_type = IDENTITY + 'ExpectedSession'
    calls = bodies[expected_type]['calls']
    require(all(c['path'] for c in calls), 'expectation derive calls before original match')
    for index, variant in enumerate(('Absent', 'Version')):
        prefix = [dict(subject=SELF, cases=[expected_type + '::' + variant], fallback=False)]
        selected = [c for c in calls if path_shape(c['path'][:1]) == prefix]
        helper = {'borrow': 'Shared', 'origin': {'aggregate': 'serde::private::ser::AdjacentlyTaggedEnumVariant',
            'fields': [{'constant': 'ExpectedSession'}, {'constant': index}, {'constant': variant.lower()}]}}
        fields = [('state', helper, 'AdjacentlyTaggedEnumVariant')]
        if variant == 'Version':
            fields.append(('version', original_field('Version._0'), 'u64'))
        checked[expected_type + '::' + variant] = check_stages(selected, 'ExpectedSession', fields, prefix)
    for name, target in TRANSPARENT.items():
        call, = bodies[name]['calls']
        require(call['callee'].endswith('::serialize') and call['path'] == [] and
                call['arguments'] == [original_field('_0'), {'serializer': True}] and
                call['destination'] == {'Local': 0}, 'transparent field forwarding changed: ' + name)
        actual_target = call['target_self_type']
        require(actual_target == target or (target == '[U8;32]' and
                actual_target == '[parameter:{"Free": 0};32]'), 'wrong transparent Serialize target')
        checked[name] = dict(transparent_to=target)
    for name, target in CONVERT.items():
        clone, convert, serialize = bodies[name]['calls']
        require(all(c['path'] == [] for c in (clone, convert, serialize)), 'conditional ID output')
        require(clone['callee'].endswith('::clone') and clone['target_self_type'] == name and
                clone['arguments'] == [{'borrow': 'Shared', 'origin': SELF}], 'wrong original ID Clone')
        require(convert['callee'] == 'core::convert::<impl>::into' and
                convert['types'] == [name, target] and
                convert['arguments'] == [{'call_result': clone['statement']}], 'wrong original ID conversion')
        dictionary, = convert['dictionaries']
        require(dictionary['trait'] == 'core::convert::From' and dictionary['types'] == [target, name]
                and dictionary['implementation']['source_name'][0] == {'Ident': ['sandproof_protocol', 0]},
                'original own conversion dictionary missing')
        require(serialize['callee'] == 'serde_core::ser::impls::<impl>::serialize' and
                serialize['target_self_type'] == target and serialize['destination'] == {'Local': 0} and
                serialize['arguments'] == [{'borrow': 'Shared', 'origin': {'call_result': convert['statement']}},
                                           {'serializer': True}], 'wrong primitive output forwarding')
        checked[name] = dict(original_conversion=target)
    report['checked'] = checked
    return report


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[4]
    data = check(json.loads(args.llbc.read_text()), json.loads((root /
        'verification/experiments/stage1-protocol/envelope-schema/inventory.json').read_text()))
    with args.output.open('x') as stream:
        json.dump(data, stream, indent=2)
        stream.write('\n')
    print(json.dumps({'original_serialize_bodies': len(data['bodies']),
                      'source_checks': len(data['checked']), 'library_refinement_proved': False}))
