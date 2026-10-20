"""Admission for the real Status guard extraction, not an async Rust proof."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import sys

try:
    from tools.check_foundations_extraction import llbc_digest
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest

ENVIRONMENT = 'verification/schema-domains-environment.json'
GUARD_LLBC = 'verification/aeneas/llbc/sandproof_schema_domains.llbc'
GENERATED = 'verification/aeneas/schema-domains'
ROOTS = {
    'sandproof_internal::status::{sandproof_internal::status::StatusRequest}::validate',
    'sandproof_internal::status::{sandproof_internal::status::HostStatus}::validate_for',
}
STRING_EQ = ('alloc::string::{impl core::cmp::PartialEq<alloc::string::String> '
             'for alloc::string::String}::eq')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_environment(root):
    return json.loads((root / ENVIRONMENT).read_text())


def check_sources(root, environment):
    if (environment.get('schema') != 1
            or environment.get('contract_id') != 'status-schema-domains-v0'
            or environment.get('proof_kind') !=
            'original-synchronous-guards-plus-conditional-fixed-json-derive-binding'):
        raise ValueError('unreviewed schema contract scope')
    for name, expected in environment['source_files'].items():
        if not re.fullmatch('[0-9a-f]{64}', expected) or sha(root / name) != expected:
            raise ValueError('production source changed: ' + name)
    schemas = environment['schemas']
    expected_types = {'StatusRequest', 'HostStatus', 'NodeIdentity', 'Resources'}
    if len(schemas) != len(expected_types) or {s['name'] for s in schemas} != expected_types:
        raise ValueError('complete Status schema family missing')
    for schema in schemas:
        source = (root / schema['file']).read_text()
        match = re.search(r'#\[serde\(deny_unknown_fields\)\]\s*pub struct '
                          + re.escape(schema['name']) + r'\s*\{([^}]+)\}', source)
        if match is None:
            raise ValueError('standard struct or deny_unknown_fields changed')
        body = re.sub(r'//[^\n]*', '', match[1])
        actual = [(name, re.sub(r'\s+', '', ty)) for name, ty in
                  re.findall(r'pub\s+(\w+)\s*:\s*([^,\n]+)', body)]
        expected = [(field['name'], field['rust_type']) for field in schema['fields']]
        if actual != expected:
            raise ValueError('complete field/type/order mismatch: ' + schema['name'])
    model = (root / 'SandboxModel/Boundaries/SchemaDomains/Model.lean').read_text()
    if re.search(r'^import\s+(?:SandproofSchemaDomains\.Funs|.*SchemaDomains\.Guard)\b',
                 model, re.M):
        raise ValueError('independent contract imports the implementation being checked')
    guard = (root / 'SandboxModel/Boundaries/SchemaDomains/Guard.lean').read_text()
    actual_theorems = {'SandboxModel.Boundaries.SchemaDomains.' + name for name in
                       re.findall(r'^theorem\s+(\w+)', guard, re.M)}
    if set(environment['theorems']) != actual_theorems:
        raise ValueError('guard theorem inventory mismatch')
    audits = (root / 'SandboxModel/Boundaries/SchemaDomains/Audit.lean').read_text()
    guard_audits = re.findall(r'^#audit_controller_proof\s+(\S+)', audits, re.M)
    if len(guard_audits) != len(actual_theorems) or set(guard_audits) != actual_theorems:
        raise ValueError('guard audit inventory incomplete or duplicated')
    return len(schemas)



CATALOGUE = 'verification/experiments/schema-domains/catalogue.json'
PRIMARY_SCHEMA_SOURCES = {
    'sandbox-protocol/src/internal.rs',
    'sandbox-protocol/src/internal/lifecycle.rs',
    'sandbox-protocol/src/internal/window.rs',
    'sandbox-internal/src/status.rs',
    'sandbox-internal/src/cache.rs',
    'sandbox-internal/src/template.rs',
}


def check_catalogue(root, catalogue=None):
    """Source-bound finite classification; not a compiler census or a proof."""
    catalogue = catalogue if catalogue is not None else json.loads((root / CATALOGUE).read_text())
    instances = catalogue['instances']
    if catalogue.get('schema') != 1 or catalogue.get('primary_schema_count') != 22:
        raise ValueError('unreviewed finite schema classification')
    observed = {(item['source'], item['name']) for item in instances}
    if len(observed) != len(instances) or len(instances) != 22:
        raise ValueError('missing or duplicated primary schema instance')
    if {item['source'] for item in instances} != PRIMARY_SCHEMA_SOURCES:
        raise ValueError('primary schema source inventory changed')
    expected = set()
    for relative in PRIMARY_SCHEMA_SOURCES:
        source = (root / relative).read_text()
        for declaration in re.finditer(r'pub (struct|enum) (\w+)\b', source):
            prefix = source[:declaration.start()]
            derives = list(re.finditer(r'#\[derive\(([^)]*)\)\]', prefix))
            if derives and 'Serialize' in {v.strip() for v in derives[-1][1].split(',')}:
                expected.add((relative, declaration[2]))
    if observed != expected:
        raise ValueError('classification omits a real serializable primary type')
    for relative, expected_hash in catalogue['sources'].items():
        if sha(root / relative) != expected_hash:
            raise ValueError('catalogue source changed: ' + relative)
    for item in instances:
        definition = item['definition_source']
        source = (root / item['source']).read_text()
        if (source.count(definition) != 1
                or hashlib.sha256(definition.encode()).hexdigest() != item['definition_sha256']):
            raise ValueError('original schema definition snapshot changed')
        if re.search(r'pub struct ' + item['name'] + r'\s*\{', definition):
            body = definition[definition.index('{') + 1:definition.rindex('}')]
            body = re.sub(r'//[^\n]*', '', body)
            actual = [(name, re.sub(r'\s+', '', ty)) for name, ty in
                      re.findall(r'pub\s+(\w+)\s*:\s*([^,\n]+)', body)]
            wanted = [(f['name'], re.sub(r'\s+', '', f['rust_type'])) for f in item['fields']]
            if actual != wanted:
                raise ValueError('catalogue field type/order changed: ' + item['name'])
    if set(catalogue['byte_contexts']) != {
            'status', 'probe', 'shared-control-header', 'lifecycle-request',
            'result-confirmation', 'window-close', 'template-cache',
            'template-readiness', 'template-artifact', 'validation-evidence'}:
        raise ValueError('actual byte context inventory incomplete')
    return len(instances)


def check_guards(root, llbc, inventory, environment=None):
    environment = load_environment(root) if environment is None else environment
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon errors or missing completion status')
    translated = llbc['translated']
    options = translated['options']
    if (translated['crate_name'] != 'sandproof_internal'
            or options['start_from'] != ['crate::status::_::validate', 'crate::status::_::validate_for']
            or options['include'] != ['sandproof_internal', 'sandproof_protocol']
            or options['dest_file'] != GUARD_LLBC
            or options['opaque'] or options['exclude'] or options['monomorphize']
            or options['skip_borrowck'] or options['no_typecheck']):
        raise ValueError('unreviewed guard extraction options')
    if {t['key'] for t in translated['target_information']} != {environment['target']}:
        raise ValueError('guard extraction target mismatch')
    declarations = {item['def_id']: item for item in translated['fun_decls'] if item}
    functions = inventory['functions']
    if len({item['def_id'] for item in functions}) != len(functions):
        raise ValueError('duplicate translated function')
    if {item['rust_name'] for item in functions if item['is_local']} != ROOTS:
        raise ValueError('missing or additional local guard root')
    if {item['rust_name'] for item in functions if item['is_opaque']} != {STRING_EQ}:
        raise ValueError('unreviewed generated external surface')
    actual_owned = set()
    for item in functions:
        if not item['rust_name'].startswith(('sandproof_internal::', 'sandproof_protocol::')):
            continue
        actual_owned.add(item['rust_name'])
        body = declarations.get(item['def_id'], {}).get('body')
        if (item['is_opaque'] or not isinstance(body, dict) or set(body) != {'Structured'}
                or declarations[item['def_id']]['item_meta']['opacity'] != 'Transparent'
                or item['lean_file'] != 'SandproofSchemaDomains/Funs.lean'):
            raise ValueError('project-owned function body hidden: ' + item['rust_name'])
        text = (root / GENERATED / item['lean_file']).read_text()
        name = item['lean_name'].removeprefix('SandproofSchemaDomainsRust.')
        if not re.search(r'\bdef\s+' + re.escape(name) + r'\b', text):
            raise ValueError('generated project definition missing')
    if actual_owned != set(environment['guard_extraction']['transparent_owned_functions']):
        raise ValueError('project-owned function inventory changed')
    embedded = [f for f in translated['files'] if f and f['name'] ==
                {'Local': 'sandbox-internal/src/status.rs'}]
    if (len(embedded) != 1 or not isinstance(embedded[0]['contents'], str)
            or hashlib.sha256(embedded[0]['contents'].encode()).hexdigest()
            != environment['source_files']['sandbox-internal/src/status.rs']):
        raise ValueError('full original guard source not bound to extraction')
    if llbc_digest(llbc) != environment['guard_extraction']['full_llbc_sha256']:
        raise ValueError('complete guard LLBC changed; preserve and review the new artifact')
    for relative, expected in environment['guard_extraction']['generated_sha256'].items():
        if sha(root / GENERATED / relative) != expected:
            raise ValueError('generated output changed: ' + relative)
    return len(ROOTS)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--llbc', type=Path)
    parser.add_argument('--inventory', type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    environment = load_environment(root)
    check_sources(root, environment)
    if (environment.get('catalogue') != CATALOGUE
            or sha(root / CATALOGUE) != environment.get('catalogue_sha256')):
        raise ValueError('source classification is not the registered snapshot')
    check_catalogue(root)
    llbc = json.loads((args.llbc or root / GUARD_LLBC).read_text())
    inventory = json.loads((args.inventory or root / GENERATED / 'translation.json').read_text())
    count = check_guards(root, llbc, inventory, environment)
    print(f'{count} real Status guards admitted; byte and async binding remain separately conditional')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f'Status schema admission failed: {error}', file=sys.stderr)
        sys.exit(1)
