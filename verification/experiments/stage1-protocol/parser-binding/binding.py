"""Inspect exact original decoder dispatch; complete LLBC remains authoritative.

Finite source admission only: no evaluation/translation of arbitrary Rust.
"""
import importlib.util
from pathlib import Path

_PARENT = Path(__file__).resolve().parents[1] / 'serde-binding/binding.py'
_SPEC = importlib.util.spec_from_file_location('parser_original_ir', _PARENT)
_original = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_original)
Original, digest, require = _original.Original, _original.digest, _original.require


def calls(original, node):
    if isinstance(node, dict):
        if 'Call' in node:
            call = node['Call']['call']
            yield dict(callee=original.callee(call['func']), original_call=call)
        for value in node.values():
            yield from calls(original, value)
    elif isinstance(node, list):
        for value in node:
            yield from calls(original, value)


def report(raw):
    original = Original(raw)
    bodies = {}
    for function in original.functions.values():
        name = function['item_meta']['name']
        if name[0] != {'Ident': ['sandproof_protocol', 0]} or name[-1] != {'Ident': ['deserialize', 0]}:
            continue
        implementation = original.impls[[p['Impl']['Trait'] for p in name
            if 'Impl' in p and 'Trait' in p['Impl']][-1]]
        self_type = original.type_name(implementation['impl_trait']['generics']['types'][0])
        if self_type.startswith('rust-builtin:'):
            continue  # included in complete derived subtree below
        require(function['item_meta']['opacity'] == 'Transparent' and
                set(function['body']) == {'Structured'}, 'original own Deserialize body missing')
        derived = [f for f in original.functions.values()
                   if f['item_meta']['name'][:len(name)] == name]
        require(self_type not in bodies, 'duplicate original decoder')
        bodies[self_type] = dict(function_id=function['def_id'],
            function_sha256=digest(function), implementation_sha256=digest(implementation),
            calls=list(calls(original, function['body'])),
            derived_subtree=[dict(name=f['item_meta']['name'], sha256=digest(f),
                calls=list(calls(original, f['body']))) for f in derived])
    operation = bodies['sandproof_protocol::lifecycle::request::LifecycleOperation']
    names = [c['callee'] for c in operation['calls']]
    require(names.count('serde_core::de::Deserializer::deserialize_any') == 1 and
            names.count('serde::private::de::content::<impl>::deserialize_any') == 5,
            'original five-way buffered operation dispatch differs')
    unknown = [row for row in operation['derived_subtree']
               if any(c['callee'] == 'serde_core::de::Error::unknown_field' for c in row['calls'])]
    # Exactly the map-key string and byte visitors of all five operation branches.
    # This is read from actual generated Rust, not the serde attribute spelling.
    require(len(unknown) == 10, 'operation unknown-field rejection visitor missing')
    expected = bodies['sandproof_protocol::lifecycle::identity::ExpectedSession']
    require([c['callee'] for c in expected['calls']] == ['serde_core::de::Deserializer::deserialize_struct'],
            'ExpectedSession is not the original adjacent struct visitor')
    return dict(kind='original-parser-dispatch-source-report', whole_parser_proved=False,
        bodies=bodies, operation_unknown_visitors=len(unknown), original_llbc_sha256=digest(raw))
