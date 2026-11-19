"""Review exact Configuration field calls using the existing Serde checker."""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PROTOCOL = HERE.parents[1]
sys.path.insert(0, str(PROTOCOL / 'serde-binding'))
from binding import Original, digest, require
from check import check_stages, original_field

spec = importlib.util.spec_from_file_location('metadata_parser_calls', PROTOCOL / 'parser-binding/binding.py')
parser = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parser)

CODEC = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::codec::Configuration'
LIMITS = 'sandproof_controller_service::capture::source::SourceObservationConfig'
NODE = 'sandproof_protocol::internal::NodeIdentity'
SCHEMA = {
    CODEC: [('config', LIMITS), ('node', NODE), ('max_sessions', 'U16')],
    LIMITS: [('max_work_bytes', 'U32'), ('max_configuration_bytes', 'U32'),
             ('max_executable_bytes', 'U64'), ('max_kernel_bytes', 'U64'),
             ('max_disk_bytes', 'U64'), ('max_material_bytes', 'U64')],
    NODE: [('deployment_id', 'sandproof_protocol::DeploymentId'),
           ('node_id', 'sandproof_protocol::NodeId'),
           ('node_generation', 'sandproof_protocol::internal::PositiveCounter')],
}


def check(raw):
    original = Original(raw)
    bodies = {}
    for function in original.functions.values():
        name = function['item_meta']['name']
        method = name[-1].get('Ident', [None])[0]
        if method not in ('serialize', 'deserialize'):
            continue
        implementations = [p['Impl']['Trait'] for p in name if 'Impl' in p and 'Trait' in p['Impl']]
        if not implementations:
            continue
        implementation = original.impls[implementations[-1]]
        self_type = original.type_name(implementation['impl_trait']['generics']['types'][0])
        if self_type not in SCHEMA:
            continue
        require((self_type, method) not in bodies, 'duplicate source Configuration dictionary')
        require(function['item_meta']['opacity'] == 'Transparent'
                and set(function['body']) == {'Structured'}, 'missing original Configuration body')
        bodies[self_type, method] = function, implementation
    require(set(bodies) == {(name, method) for name in SCHEMA for method in ('serialize', 'deserialize')},
            'incomplete Configuration serialization and decoding graph')
    reviewed = {}
    for name, fields in SCHEMA.items():
        function, implementation = bodies[name, 'serialize']
        reference = original.resolve(implementation['impl_trait']['generics']['types'][0], original.types)
        declared = original.type_decls[reference['Adt']['id']]
        require([(field['name'], original.type_name(field['ty'])) for field in declared['kind']['Struct']] == fields,
                'the complete original Configuration field declaration differs')
        expected = [(key, original_field(key), ty.lower() if ty in ('U16', 'U32', 'U64')
                     else ty.rsplit('::', 1)[-1]) for key, ty in fields]
        reviewed[name] = check_stages(original.calls(function['body']['Structured']),
                                     name.rsplit('::', 1)[-1], expected, [])
        decoder, _ = bodies[name, 'deserialize']
        direct = list(parser.calls(original, decoder['body']))
        require([call['callee'] for call in direct] == ['serde_core::de::Deserializer::deserialize_struct'],
                'original Configuration struct deserializer dispatch changed')
        prefix = decoder['item_meta']['name']
        subtree = [f for f in original.functions.values() if f['item_meta']['name'][:len(prefix)] == prefix]
        unknown = [f for f in subtree if any(c['callee'] == 'serde_core::de::Error::unknown_field'
                   for c in parser.calls(original, f['body']))]
        require(len(unknown) == 2, 'original string/byte unknown-field visitor missing')
        reviewed[name]['decoder'] = dict(function_sha256=digest(decoder), unknown_field_visitors=2,
            subtree=[dict(name=f['item_meta']['name'], sha256=digest(f)) for f in subtree])
        reviewed[name]['serialize_sha256'] = digest(function)
    return dict(kind='source-metadata-json-source-review', library_refinement_proved=False,
                reviewed=reviewed, raw_sha256=digest(raw), options=original.data['options'])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--ir', required=True, type=Path)
    cli.add_argument('--output', required=True, type=Path)
    args = cli.parse_args()
    report = check(json.loads(args.ir.read_bytes()))
    with args.output.open('x') as stream:
        stream.write(json.dumps(report, indent=2) + '\n')
    print('reviewed', len(report['reviewed']), 'complete structures')
