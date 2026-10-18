"""Admit eight real identifier Deserialize leaves and their finite A6 library view."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

try:
    from tools.check_json_counter_extraction import check as check_counter, check_environment, nodes
    from tools.check_foundations_extraction import llbc_digest
except ModuleNotFoundError:
    from check_json_counter_extraction import check as check_counter, check_environment, nodes
    from check_foundations_extraction import llbc_digest

ENVIRONMENT = 'verification/json-identifiers-environment.json'
ENTRY = 'verification/extract-foundations/src/json_ids.rs'
SOURCE = 'sandbox-protocol/src/lib.rs'
DEST = 'verification/aeneas/foundations'
LLBC = 'verification/aeneas/llbc/sandproof_foundations.llbc'
KINDS = {
    'tenant': 'TenantId', 'sandbox': 'SandboxId', 'operation': 'OperationId',
    'logical_action': 'LogicalActionId', 'resource': 'ResourceId',
    'client_request': 'ClientRequestId', 'deployment': 'DeploymentId', 'node': 'NodeId',
}
OWNED = {
    f'sandproof_protocol.{identity}.Insts.{method}'
    for identity in KINDS.values()
    for method in ('Serde_coreDeDeserialize.deserialize',
                   'CoreConvertTryFromStringContractError.try_from')
}
OWNED |= {
    f'sandproof_protocol.DeserializeDe{identity}.deserialize.closure.Insts.'
    'CoreOpsFunctionFnOnceTupleContractErrorClause0_Error.call_once'
    for identity in KINDS.values()
}
OWNED |= {f'json_ids.{kind}_decode' for kind in KINDS} | {'json_ids.string_primitive'}
OWNED.add('sandproof_protocol.ContractError.Insts.CoreFmtDisplay.fmt')
LIBRARY = {
    'alloc.string.String.Insts.Serde_coreDeDeserialize.deserialize',
    *{'serde_core.de.impls.StringVisitor.Insts.Serde_coreDeVisitorDeString.' + method
      for method in ('expecting', 'visit_str', 'visit_string')},
}
FOCUSED = OWNED | LIBRARY


def check(root, llbc, inventory):
    # Preserve the old complete V0 and counter admission; no alternative green path.
    check_counter(root, llbc, inventory)
    environment = json.loads((root / ENVIRONMENT).read_text())
    if set(environment['functions']) != FOCUSED:
        raise ValueError('identifier function registry changed')
    functions = {
        f['lean_name'].removeprefix('SandproofFoundationsRust.'): f
        for f in inventory['functions']
        if f['lean_name'].removeprefix('SandproofFoundationsRust.') in FOCUSED
    }
    if set(functions) != FOCUSED:
        raise ValueError('missing real identifier function')
    declarations = {f['def_id']: f for f in llbc['translated']['fun_decls'] if f is not None}
    string_drop = environment['string_drop_glue']
    destructor = declarations[string_drop]
    name = destructor['item_meta']['name']
    if (name[:3] != [{'Ident': [part, 0]} for part in ('alloc', 'string', 'String')]
            or len(name) != 5 or name[-1] != {'Builtin': ['DropGlue', 0]}
            or destructor['item_meta']['is_local'] or destructor['body'] != 'Opaque'):
        raise ValueError('unreviewed String drop glue')
    for name, function in functions.items():
        if function['is_opaque'] or function['lean_file'] != 'SandproofFoundations/Funs.lean':
            raise ValueError('identifier-owned or reviewed primitive body hidden: ' + name)
        body = declarations[function['def_id']].get('body')
        if not isinstance(body, dict) or set(body) != {'Structured'}:
            raise ValueError('identifier structured body missing: ' + name)
        if name in OWNED:
            expected = ENTRY if name.startswith('json_ids.') else SOURCE
            prefix = root / 'verification/extract-foundations' if name.startswith('json_ids.') else root
            if (prefix / function['source']['file']).resolve() != (root / expected).resolve():
                raise ValueError('identifier source rebound: ' + name)
        cleanups = []
        for node in nodes(body):
            if any(tag in node for tag in ('Error', 'Loop')):
                raise ValueError('unreviewed identifier loop or error body: ' + name)
            if 'Drop' in node:
                drop = node['Drop']
                conditional = name.startswith('json_ids.') and drop['kind'] == 'Conditional'
                string_cleanup = (
                    name.startswith('sandproof_protocol.')
                    and name.endswith(('CoreConvertTryFromStringContractError.try_from',
                                       'Serde_coreDeDeserialize.deserialize'))
                    and drop['kind'] == 'Precise'
                    and drop['fn_ptr']['kind'] == {'Fun': string_drop}
                )
                if not (conditional or string_cleanup):
                    raise ValueError('unreviewed identifier Drop: ' + name)
                cleanups.append(drop)
        if cleanups != environment['cleanup'].get(name, []):
            raise ValueError('identifier cleanup changed: ' + name)
    # These are real call-throughs, with the exact type and argument: no copied validator.
    entry = (root / ENTRY).read_text().split('#[cfg(test)]', 1)[0]
    for kind, identity in KINDS.items():
        pattern = (r"pub fn " + kind + r"_decode<'de, D: Deserializer<'de>>\(\s*"
                   r"input: D,?\s*\) -> Result<sandproof_protocol::" + identity +
                   r", D::Error> \{\s*sandproof_protocol::" + identity +
                   r"::deserialize\(input\)\s*\}")
        if len(re.findall(pattern, entry)) != 1:
            raise ValueError('identifier call-through changed: ' + kind)
    fmt_name = 'sandproof_protocol.ContractError.Insts.CoreFmtDisplay.fmt'
    body = declarations[functions[fmt_name]['def_id']]['body']
    digest = hashlib.sha256(json.dumps(body, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    if digest != environment['contract_error_format_body_sha256']:
        raise ValueError('finite formatting callsite changed; review bytecode and callback domain')
    if environment['format_domain'] != {
        'literal': 'unsupported protocol version ', 'prefix_length': 29,
        'suffix_bytes': [192, 0], 'arguments': ['u16:Display:default'],
    }:
        raise ValueError('unsupported formatting domain')
    # The counter gate freezes the same full LLBC; pin this leaf's independent record too.
    if llbc_digest(llbc) != environment['llbc_sha256']:
        raise ValueError('identifier full LLBC changed')
    return len(FOCUSED)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--tool-root', type=Path)
    args = parser.parse_args(argv)
    try:
        llbc = json.loads((args.root / LLBC).read_text())
        inventory = json.loads((args.root / DEST / 'translation.json').read_text())
        count = check(args.root, llbc, inventory)
        if args.tool_root is not None:
            check_environment(args.root, args.tool_root)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print('JSON identifier extraction failed: ' + str(error), file=sys.stderr)
        return 1
    print(f'JSON identifiers: {count} transparent real functions; eight checked leaves.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
