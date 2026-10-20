"""Check a finite source/domain reference ledger; this does not prove Rust."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re

DIRECTORY = Path('verification/experiments/shared-field-bindings')
FROZEN = Path('verification/shared-field-bindings-inputs.sha256')
FAMILIES = ['D' + str(i) for i in range(1, 12)]
SCOPE = 'finite-shared-field-source-and-proof-references-not-new-program-proof'
STATES = {'existing-implementation-theorem', 'conditional-existing-leaf-binding',
          'source-reviewed-signature-not-preflighted'}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate JSON key: ' + key)
            result[key] = value
        return result
    return json.loads(path.read_text(), object_pairs_hook=unique)


def file(root, name):
    path = Path(name)
    if path.is_absolute() or '..' in path.parts or not path.parts:
        raise ValueError('escaping catalogue file')
    path = root / path
    if path.is_symlink() or not path.is_file():
        raise ValueError('missing/symlink source: ' + name)
    path.resolve(strict=True).relative_to(root.resolve(strict=True))
    return path


def manifest(root, name):
    values = {}
    for line in file(root, name).read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (.+)', line)
        if not match or match[2] in values:
            raise ValueError('malformed/duplicate frozen source')
        values[match[2]] = match[1]
    return values


def check_domains(data):
    d = data['domains']
    # These intentionally independent distinctions prevent changing the labels
    # while preserving a superficially plausible source/proof list.
    if (d['positive-u64']['minimum'] != 1 or d['ordinary-u64']['minimum'] != 0
            or d['positive-u64']['maximum'] != 2 ** 64 - 1
            or d['ordinary-u64']['maximum'] != 2 ** 64 - 1):
        raise ValueError('ordinary and nonzero U64 domains confused')
    if (d['i64']['signed'] is not True or d['i64']['minimum'] != -(2 ** 63)
            or d['i64']['maximum'] != 2 ** 63 - 1):
        raise ValueError('signed I64 domain lost')
    if (d['usize']['bits'] != 64 or d['usize']['target'] != 'x86_64-unknown-linux-gnu'
            or d['usize']['maximum'] != 2 ** 64 - 1):
        raise ValueError('unregistered machine-word target')
    if (d['opaque-id']['length_unit'] != 'utf8-bytes' or d['opaque-id']['maximum_bytes'] != 128
            or d['opaque-id']['minimum_bytes'] != 1 or d['opaque-id']['leading_zero'] != 'allowed'
            or d['opaque-id']['characters'] != 'ASCII alphanumeric or -_.'
            or d['opaque-id']['normalization'] != 'none'
            or d['canonical-u64']['leading_zero'] != 'rejected'
            or d['canonical-u64']['minimum'] != 1 or d['canonical-u64']['maximum'] != 2 ** 64 - 1
            or d['canonical-u64']['prefix'] != 'exact-original-utf8-prefix'):
        raise ValueError('byte/character or opaque/canonical identifier domains confused')
    if (d['bytes']['encoding'] != 'arbitrary-bytes-not-required-utf8'
            or d['bytes']['optional'] != 'None-distinct-from-Some-empty'
            or d['text']['encoding'] != 'valid-utf8'):
        raise ValueError('raw bytes, optional empty and UTF8 text confused')
    if (d['slots']['minimum'], d['slots']['maximum'], d['slots']['accepted_minimum'],
            d['slots']['accepted_maximum']) != (0, 65535, 1, 4096):
        raise ValueError('wire representation and status accepted domain confused')


def check_fragment(root, span, selected):
    for name in ('path', 'marker', 'occurrence', 'within'):
        if span[name] != selected[name]:
            raise ValueError('source selector changed: ' + selected['id'])
    lines = file(root, span['path']).read_text().splitlines(keepends=True)
    start, end = span['start_line'], span['end_line']
    if type(start) is not int or type(end) is not int or not 1 <= start <= end <= len(lines):
        raise ValueError('invalid original source span')
    raw = ''.join(lines[start - 1:end])
    if raw != span['text'] or hashlib.sha256(raw.encode()).hexdigest() != span['sha256']:
        raise ValueError('original source fragment changed')
    if span['marker'] not in lines[start - 1]:
        raise ValueError('source span does not start at its original symbol')
    matches = [i for i, line in enumerate(lines) if span['marker'] in line]
    if span['within']:
        begins = [i for i, line in enumerate(lines) if span['within'] in line]
        if len(begins) != 1:
            raise ValueError('ambiguous original implementation scope')
        begin = begins[0]
        finish = next(i for i in range(begin + 1, len(lines))
                      if lines[i].strip() == '}' and not lines[i][0].isspace())
        matches = [i for i in matches if begin < i < finish]
    if matches[span['occurrence']] != start - 1:
        raise ValueError('wrong original method occurrence')


def lexical_candidates(root, source_map, rules):
    result = []
    for path in sorted(source_map):
        if not path.endswith('.rs'):
            continue
        for line, text in enumerate(file(root, path).read_text().splitlines(), 1):
            for rule in rules:
                for match in re.finditer(rule['pattern'], text):
                    result.append(dict(path=path, line=line, column=match.start() + 1,
                        rule=rule['id'], text=text, source_sha256=source_map[path],
                        state='lexical-candidate-not-resolved-call',
                        profile='T0 source includes test/cfg text; not necessarily compiled in this unit'))
    return result


def check(root, data=None, selection=None):
    data = read(root / DIRECTORY / 'catalogue.json') if data is None else data
    selection = read(root / DIRECTORY / 'selection.json') if selection is None else selection
    if (data['schema'] != 1 or selection['schema'] != 1 or data['scope'] != SCOPE
            or data['baseline'] != selection['baseline']
            or list(data['families']) != FAMILIES or data['families'] != selection['families']):
        raise ValueError('unregistered finite scope/families')
    check_domains(data)
    for name, digest in data['sources'].items():
        if sha(file(root, name)) != digest:
            raise ValueError('frozen referenced source changed: ' + name)
    for name in {p['frozen_manifest'] for p in data['proofs'].values()}:
        for source, digest in manifest(root, name).items():
            if sha(file(root, source)) != digest:
                raise ValueError('existing proof frozen lineage changed: ' + source)
    for key, proof in data['proofs'].items():
        path, audit, frozen = (proof[n] for n in ('path', 'audit_path', 'frozen_manifest'))
        if not {path, audit, frozen} <= set(data['sources']):
            raise ValueError('proof reference outside source closure: ' + key)
        text = file(root, path).read_text()
        if not re.search(r'^theorem ' + re.escape(proof['declaration']) + r'\b', text, re.M):
            raise ValueError('missing exact existing theorem')
        if not proof['qualified_name'].endswith('.' + proof['declaration']):
            raise ValueError('qualified audit target differs from source theorem')
        if ('#audit_controller_proof ' + proof['qualified_name'] + '\n'
                not in file(root, audit).read_text()):
            raise ValueError('missing exact existing audit declaration')
        table = manifest(root, frozen)
        if any(table.get(n) != data['sources'][n] for n in (path, audit)):
            raise ValueError('theorem differs from its existing frozen source')
        if proof['status'] != 'existing-source-and-audit-reference-not-new-proof':
            raise ValueError('source reference cannot claim a new proof')
    wanted = {row['id']: row for row in selection['locations']}
    actual = {row['id']: row for row in data['entries']}
    if (len(wanted) != len(selection['locations']) or len(actual) != len(data['entries'])
            or set(actual) != set(wanted) or {x['family'] for x in actual.values()} != set(FAMILIES)):
        raise ValueError('missing/duplicate/extra reviewed location or family')
    t0 = data['t0']
    config = read(file(root, t0['config']))
    published = read(file(root, t0['catalogue']))
    if (t0['roots'] != [r['id'] for r in config['roots']] or t0['profiles'] != config['profiles']
            or t0['roots'] != ['center', 'host', 'window-reducer', 'guest', 'vm-cli']
            or t0['profiles'] != ['dev', 'release'] or t0['units'] != 60
            or len(published['units']) != t0['units'] or t0['packages'] != published['packages']
            or t0['capture_commit'] != published['capture_commit']
            or t0['published_evidence'] != published['evidence']
            or published['config_sha256'] != sha(root / t0['config'])):
        raise ValueError('T0 five-root/configuration/unit references changed')
    t0_sources = manifest(root, t0['manifest'])
    for row in actual.values():
        selected = wanted[row['id']]
        if (row['family'] != row['id'].split('.')[0] or row['kind'] != selected['kind']
                or row['domains'] != selected['domains'] or row['state'] not in STATES
                or row['state'] != selected['allowed_state']):
            raise ValueError('wrong location/domain/status label: ' + row['id'])
        if row['proof_refs'] != selected['proof_refs'] or row['extraction'] != selected['extraction']:
            raise ValueError('wrong reviewed theorem/generated binding: ' + row['id'])
        if not set(row['domains']) <= set(data['domains']):
            raise ValueError('unregistered domain')
        required = ('symbol', 'object', 'input_domain', 'preservation', 'acceptance',
                    'rejection_order', 'conditions', 'providers', 'acceptance_evidence')
        if any(not row.get(n) for n in required) or not set(row['proof_refs']) <= set(data['proofs']):
            raise ValueError('missing interface/acceptance/condition/provider')
        if not set(row['providers']) <= {'V' + str(i) for i in range(10)}:
            raise ValueError('unassigned downstream provider')
        check_fragment(root, row['source'], selected)
        path = row['source']['path']
        if path not in data['sources'] or t0_sources.get(path) != data['sources'][path]:
            raise ValueError('reviewed Rust is not the published T0 source')
        prefix = path.split('/')[0] + '/'
        units = [i for i, unit in enumerate(published['units']) if unit['source'].startswith(prefix)]
        if not units or row['t0_units'] != units:
            raise ValueError('wrong T0 crate/unit reference')
        if row['kind'] == 'struct':
            fields = {m[1]: m[2].rstrip(',').strip() for m in re.finditer(
                r'^\s*pub (\w+): (.+)$', row['source']['text'], re.M)}
            if list(fields) != row['complete_fields'] or fields != row['field_types']:
                raise ValueError('missing or changed full struct fields')
        extraction = row['extraction']
        if row['state'] == 'existing-implementation-theorem':
            if (not row['proof_refs'] or not extraction['functions']
                    or extraction['state'] != 'existing-transparent-generated-reference'):
                raise ValueError('no real existing implementation proof binding')
        if extraction['functions']:
            registry = extraction['registry']
            if registry not in data['sources']:
                raise ValueError('unfrozen generated reference')
            functions = read(file(root, registry))['functions']
            for name in extraction['functions']:
                found = [f for f in functions if f['rust_name'] == name]
                if len(found) != 1 or found[0]['is_opaque'] is not False:
                    raise ValueError('missing/opaque actual generated function')
        elif extraction != dict(state='unknown-not-preflighted-for-this-location', registry=None, functions=[]):
            raise ValueError('unknown signature must remain unknown')
    for record in selection['macro_sets'].values():
        if re.findall(record['pattern'], file(root, record['path']).read_text(), re.M) != record['expected']:
            raise ValueError('shared macro instances changed or omitted')
    # Compare the whole selected lexical census, including tests and ambiguous
    # same-named IDs. It must never be relabelled as a resolved production call.
    for name, digest in t0_sources.items():
        if name.endswith('.rs') and sha(file(root, name)) != digest:
            raise ValueError('T0 production source changed before candidate scan')
    if data['lexical_candidates'] != lexical_candidates(root, t0_sources, selection['lexical_rules']):
        raise ValueError('limited lexical candidates missing, changed or mislabelled')
    if ([{k: g[k] for k in ('id', 'entries', 'classification')} for g in data['gaps']]
            != selection['required_gaps']):
        raise ValueError('missing, duplicate or relabelled reviewed gap')
    for gap in data['gaps']:
        if (gap['status'] != 'source-only-not-preflighted'
                or not set(gap['entries']) <= set(actual) or not gap['minimum_signature']
                or not gap['next'] or not gap['provider']):
            raise ValueError('gap falsely resolved, unassigned or underspecified')
        if gap['classification'].startswith('candidate-public-') and 'V0' not in gap['provider']:
            raise ValueError('pending public connection cannot be reassigned only to business')
    check_capture_summary(root, data, selection, published)
    return dict(scope=SCOPE, locations=len(actual), families=len(FAMILIES),
                theorem_references=len(data['proofs']), lexical_candidates=len(data['lexical_candidates']),
                gaps=len(data['gaps']), new_theorems=0, new_extractions=0)


def check_capture_summary(root, data, selection, published):
    record = read(root / DIRECTORY / 'capture-summary.json')
    if (record['schema'] != 1 or record['kind'] != 'existing-t0-literal-header-locations-not-a-new-proof-or-extraction'
            or record['baseline'] != data['baseline']
            or record['receipt_sha256'] != published['evidence']['receipt_sha256']
            or record['index_sha256'] != published['evidence']['index_sha256']
            or record['source_commit'] != published['capture_commit']
            or record['all_units'] != len(published['units'])
            or record['source_count'] != published['source_inputs']):
        raise ValueError('original capture summary identity changed')
    selected = {n: v for n, v in data['sources'].items() if n.endswith('.rs')}
    if record['selected_source_sha256'] != selected or record['queries'] != selection['mir_queries']:
        raise ValueError('finite captured source or query differs')
    stages = [p + '-' + r for p in data['t0']['profiles'] for r in data['t0']['roots']]
    if record['original_stages'] != stages:
        raise ValueError('original five-root capture stage omitted')
    crates = {q['crate'] for q in selection['mir_queries']}
    keys = ('crate', 'source', 'cfg', 'codegen', 'mir_sha256')
    actual = [{k: u[k] for k in keys} for u in record['units']]
    expected = [{k: u[k] for k in keys[:-1]} | {'mir_sha256': u['sha256']}
                for u in published['units'] if u['crate'] in crates]
    canonical = lambda row: json.dumps(row, sort_keys=True)
    if sorted(map(canonical, actual)) != sorted(map(canonical, expected)):
        raise ValueError('raw captured units do not match all selected published variants')
    units = {u['receipt_unit']: u for u in record['units']}
    if len(units) != len(record['units']) or set(record['observations']) != {q['id'] for q in selection['mir_queries']}:
        raise ValueError('duplicate unit or missing finite MIR query')
    for query in selection['mir_queries']:
        found = record['observations'][query['id']]
        if not found:
            raise ValueError('missing finite MIR header')
        positions = set()
        for row in found:
            unit = units[row['receipt_unit']]
            position = row['receipt_unit'], row['line']
            if (position in positions or type(row['line']) is not int or row['line'] <= 0
                    or unit['crate'] != query['crate'] or row['mir_sha256'] != unit['mir_sha256']
                    or not row['header'].startswith('fn ')
                    or not all(token in row['header'] for token in query['tokens'])):
                raise ValueError('wrong/duplicate literal MIR location')
            positions.add(position)


def check_frozen(root):
    values = manifest(root, str(FROZEN))
    required = {str(DIRECTORY / n) for n in ('catalogue.json', 'selection.json', 'README.md', 'replay.py',
                                           'capture.py', 'capture-summary.json')}
    required |= {'tools/check_shared_field_bindings.py', 'tools/tests/test_shared_field_bindings.py'}
    required |= set(read(root / DIRECTORY / 'catalogue.json')['sources'])
    if not required <= set(values):
        raise ValueError('incomplete local frozen catalogue closure')
    for name, digest in values.items():
        if sha(file(root, name)) != digest:
            raise ValueError('frozen catalogue input changed: ' + name)
    return values


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    check_frozen(args.root.resolve(strict=True))
    print(json.dumps(check(args.root.resolve(strict=True)), ensure_ascii=False))
