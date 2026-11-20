"""Strict current capture and complete, previously audited project inputs."""
from hashlib import sha256
import json
from pathlib import Path
import re
import subprocess
import tarfile
from tempfile import TemporaryDirectory

import extract
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact

ROOT = 'controller.lifecycle_inbox.source_work.codec.clear_retired_source_record'
EQUALITY = 'controller.lifecycle_inbox.source_work.SourceWorkRecord.Insts.CoreCmpPartialEqSourceWorkRecord'
OLD_NAMESPACE = 'SandproofSourceRetentionProviders'
SUPPORT = ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh')


def digest(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('one regular input file is required: ' + str(path))
    return sha256(path.read_bytes()).hexdigest()


def dependencies(packages, manifest):
    locked = json.loads(manifest.read_bytes())['packages']
    result = {}
    for package in locked:
        if package['type'] != 'git':
            raise ValueError('the original fixed Git dependency inventory is required')
        location = packages / package['name']
        actual = subprocess.check_output(['git', '-C', str(location), 'rev-parse', 'HEAD'], text=True).strip()
        dirty = subprocess.check_output(
            ['git', '-C', str(location), 'status', '--porcelain', '--untracked-files=no'], text=True).strip()
        if actual != package['rev'] or dirty:
            raise ValueError('fixed Lean dependency changed: ' + package['name'])
        result[package['name']] = actual
    if len(result) != len(locked):
        raise ValueError('the fixed Lean dependency inventory contains duplicate names')
    return result


def parsed(data, namespace):
    return declarations(data.removesuffix(b'\n\nend ' + namespace.encode() + b'\n'))


def capture(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = (extract.SOURCE, extract.TREE, variant, extract.OPTIONS,
                digest(Path(extract.__file__)), digest(Path(extract.ports.__file__)), True, False)
    found = tuple(result[k] for k in ('source', 'tree', 'variant', 'options', 'driver',
                                     'ports_driver', 'complete', 'public_acceptance'))
    if found != expected:
        raise ValueError('the exact completed current retirement capture is required')
    if [(r['label'], r['exit_code'], r['bounded_stop']) for r in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed extraction tools must finish successfully')
    read_exact(directory / 'source.tar', extract.ARCHIVE)
    for command in result['commands']:
        read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    if set(result['staged']) != {str(path) for path in extract.STAGED}:
        raise ValueError('the complete five-file original staging inventory is required')
    with TemporaryDirectory(prefix='source-retire-input-', dir=directory.parent) as temporary:
        fresh = Path(temporary)
        extract.reconstruct(directory / 'source.tar', fresh, variant)
        with tarfile.open(directory / 'source.tar') as archive:
            members = {m.name: m for m in archive.getmembers() if m.isfile()}
        actual = {str(p.relative_to(directory / 'source')) for p in (directory / 'source').rglob('*')
                  if p.is_file()}
        if actual != set(members):
            raise ValueError('the entire captured production source inventory must remain present')
        for name, member in members.items():
            original = fresh / name
            captured = directory / 'source' / name
            if (digest(captured) != digest(original)
                    or bool(captured.stat().st_mode & 0o111) != bool(member.mode & 0o111)):
                raise ValueError('captured production source differs from exact reconstruction: ' + name)
        for name, wanted in result['staged'].items():
            read_exact(directory / 'source' / name, wanted)
    read_exact(directory / 'actual.llbc', result['ir'])
    generated = directory / 'generated'
    names = {'translation.json', *(extract.NAMESPACE + '/' + name + '.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (set(result['generated']) != names
            or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != names):
        raise ValueError('the complete actual generation inventory is required')
    for name, wanted in result['generated'].items():
        read_exact(generated / name, wanted)
    fixed = json.loads((extract.REPO / 'verification/foundations-environment.json').read_bytes())
    if extract.admit(directory / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('the actual transparent business/library mapping changed')
    sources = {name: (generated / extract.NAMESPACE / (name + '.lean')).read_bytes()
               for name in ('Types', 'Funs')}
    sources['External'] = (generated / extract.NAMESPACE / 'FunsExternal_Template.lean').read_bytes()
    return result, sources


def service_result(row):
    response = row['result']
    for _ in range(3):
        if 'success' in response:
            return response
        response = response.get('structuredContent', response.get('result', {}))
    return response


def prior(project, audit, root, physical_files, audits):
    plan = json.loads((project / 'plan.json').read_bytes())
    result = json.loads((audit / 'result.json').read_bytes())
    if (plan['root'] != root or len(plan['files']) != physical_files
            or result['complete'] is not True or result['physical_files'] != physical_files
            or any(result.get(k) for k in ('warnings', 'errors', 'own_warnings', 'rejected'))
            or result['source_sha256'] != plan['files'] or len(result['audited']) != audits):
        raise ValueError('the entire successful original project and audit are required: ' + root)
    read_exact(audit / 'service.log', result['service_log_sha256'])
    for name, wanted in plan['files'].items():
        read_exact(project / name, wanted)
    if closure(project, [root + '.lean'], active_view='lakefile.toml') != plan['files']:
        raise ValueError('the original complete project import closure differs')
    requests = [name.replace('.', '/') + '.lean' for name in plan['build_order']]
    if set(requests) != set(plan['files']) or len(requests) != physical_files:
        raise ValueError('the original physical-file audit plan is incomplete')
    records = []
    for line in (audit / 'service.log').read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get('request', {}).get('tool') == 'lean_diagnostic_messages':
            records.append(row)
    if len(records) != physical_files:
        raise ValueError('all original actual-file service responses are required')
    audited = []
    for name, row in zip(requests, records):
        request = row['request']
        wanted = plan['files'][name]
        response = service_result(row)
        if (request['arguments']['file_path'] != name or request['expected_sha256'] != wanted
                or row['stale'] is not False or row['revision']['file_sha256'] != wanted
                or response.get('success') is not True or response.get('partial') is not False
                or response.get('timed_out') is not False or response.get('failed_dependencies')
                or response.get('still_elaborating_lines') not in (None, [])):
            raise ValueError('original actual-file service response is incomplete: ' + name)
        expected = set(re.findall(r'^#audit_controller_proof (\S+)$', (project / name).read_text(), re.M))
        observed = []
        for item in response.get('items', []):
            if item.get('severity') in ('error', 'warning'):
                raise ValueError('original actual-file diagnostics contain an issue: ' + name)
            match = re.fullmatch(r'Checked ([\w.]+): \d+ standard logical dependencies',
                                 str(item.get('message', item.get('data', ''))))
            if match:
                observed.append(match[1])
        if expected != set(observed) or len(observed) != len(set(observed)):
            raise ValueError('the complete original actual-file theorem audit differs: ' + name)
        audited.extend(observed)
    if len(audited) != audits or len(audited) != len(set(audited)):
        raise ValueError('the original whole-project audit inventory differs')
    return plan, dict(plan=digest(project / 'plan.json'), audit=digest(audit / 'result.json'),
                      service=result['service_log_sha256'])


def select(reference, old_project):
    current = {group: parsed(reference[group], extract.NAMESPACE) for group in ('Types', 'Funs')}
    old = {group: parsed((old_project / OLD_NAMESPACE / (group + '.lean')).read_bytes(), OLD_NAMESPACE)
           for group in ('Types', 'Funs')}
    shared = {}
    for group, expected in (('Types', 93), ('Funs', 105)):
        common = set(current[group]) & set(old[group])
        if len(common) != expected or any(current[group][n] != old[group][n] for n in common):
            raise ValueError('all original complete record declarations must remain identical: ' + group)
        shared[group] = sorted(common)
    data = current['Types'] | current['Funs']
    previous = old['Types'] | old['Funs']
    external_source = reference['External'].decode()
    external = set(re.findall(r'axiom\s+(\S+)', external_source))
    pattern = re.compile(r'(?<![\w.])(' + '|'.join(map(re.escape,
        sorted(set(data) | external, key=len, reverse=True))) + r')(?![\w.])')

    def reached(root):
        found, reused, libraries, pending = set(), set(), set(), [root]
        while pending:
            name = pending.pop()
            if name in found | reused | libraries:
                continue
            if name in previous:
                reused.add(name)
            elif name in external or name == 'controller.lifecycle_inbox.sha256.digest':
                libraries.add(name)
            else:
                found.add(name)
                pending.extend(n for n in pattern.findall(data[name].decode()) if n != name)
        return found, reused, libraries

    all_nodes, reused, libraries = reached(ROOT)
    comparisons, _, _ = reached(EQUALITY)
    types = [n for n in current['Types'] if n in all_nodes and not n.startswith('rusqlite.')]
    equalities = [n for n in current['Funs'] if n in comparisons]
    functions = [n for n in current['Funs'] if n in all_nodes and n not in comparisons]
    if (len(types), len(equalities), len(functions)) != (23, 28, 102):
        raise ValueError('the entire current retirement dependency slice is required')
    return dict(data=data, shared=shared, types=types, equalities=equalities, functions=functions,
                reused=sorted(reused), libraries=sorted(libraries), pattern=pattern)


def declaration(reference, name, body):
    raw = reference['Types'] if name in parsed(reference['Types'], extract.NAMESPACE) else reference['Funs']
    at = raw.find(body)
    if at < 0 or raw.count(body) != 1:
        raise ValueError('one complete original generated declaration is required: ' + name)
    begin = raw.rfind(b'\n\n/--', 0, at)
    prefix = raw[begin:at]
    # Preserve the tool's reducible aliases/dictionaries. Rust registration
    # attributes are omitted in the namespace view; bodies remain verbatim.
    return (b'@[reducible]\n' if re.search(rb'@\[reducible\]\s*$', prefix) else b'') + body
