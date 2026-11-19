"""Bind the real SQLite row consumer to the frozen load/save audit closure."""
import argparse
from hashlib import sha256
import json
from pathlib import Path

import source_load_binding_capture as capture
from prepare_source_key_history import structure_shape
from prepare_observation_suffix import replace_once
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).with_name('source-load-binding')
ROOT = 'SourceLoadBinding'
VIEWS = ('SourceLoadView', ROOT)
PACKAGE = 'controller.lifecycle_inbox.source_work.'
SUPPORT = ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py',
           'scripts/lean-mcp-server.sh')


def reader(directory, prior):
    report = json.loads((directory / 'result.json').read_bytes())
    if (report['source'], report['tree'], report['options'], report['driver'], report['complete']) != (
            capture.SOURCE, capture.TREE, capture.CFG, capture.sql.digest(Path(capture.__file__)), True):
        raise ValueError('the completed original loader capture is required')
    if [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('the fixed tools must finish normally')
    read_exact(directory / 'source.tar', capture.ARCHIVE)
    stage = read_exact(directory / 'source' / capture.sql.CODEC, report['codec'])
    if (stage.count(capture.SELECT.encode()) != 1
            or not stage.endswith((HERE / 'loader.rs.in').read_bytes())):
        raise ValueError('the original SELECT and exact staged consumer must remain')
    for command in report['commands']:
        read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    ir, gen = directory / 'actual.llbc', directory / 'generated'
    read_exact(ir, report['ir'])
    expected = {'translation.json', *(f'{capture.NAMESPACE}/{name}.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (report['generated'].keys() != expected
            or {str(file.relative_to(gen)) for file in gen.rglob('*') if file.is_file()} != expected):
        raise ValueError('the original capture file inventory differs')
    files = {name: read_exact(gen / name, report['generated'][name]) for name in expected}
    fixed = json.loads((capture.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    if capture.admit(ir, gen, fixed['target']) != report['admission']:
        raise ValueError('original parent, loop or JSON selection changed')
    type_bytes = files[f'{capture.NAMESPACE}/Types.lean']
    if structure_shape(type_bytes, PACKAGE + 'SourceWorkRecord') != structure_shape(
            (prior / 'ObservationSuffixTypes.lean').read_bytes(), PACKAGE + 'SourceWorkRecord'):
        raise ValueError('the complete original record shape differs')
    for name in ('capture.source.SourceObservationRequirement', PACKAGE + 'SourceWork'):
        inherited = structure_shape((prior / 'SourceKeyEncode.lean').read_bytes(), name)
        if structure_shape(type_bytes, name) != inherited.removesuffix(b'\nend'):
            raise ValueError('the original key path changed: ' + name)

    function_bytes = files[f'{capture.NAMESPACE}/Funs.lean']
    funs = declarations(function_bytes.removesuffix(b'\n\nend ' + capture.NAMESPACE.encode() + b'\n'))
    fragments = {b'-- ACTUAL DECODE': funs[PACKAGE + 'codec.decode_loaded_record'],
                 b'-- ACTUAL BODY': funs[PACKAGE + 'codec.read_loaded_rows_loop.body'],
                 b'-- ACTUAL LOOP': funs[PACKAGE + 'codec.read_loaded_rows_loop'],
                 b'-- ACTUAL READER': funs[PACKAGE + 'codec.read_loaded_rows']}
    view = (HERE / 'SourceLoadView.lean.in').read_bytes()
    for marker, content in fragments.items():
        view = replace_once(view, marker + b'\n', content + b'\n')
    return view, report, {name.decode(): sha256(content).hexdigest() for name, content in fragments.items()}


def prepare(actual, prior, audit, output, packages, service):
    view, extracted, fragments = reader(actual, prior)
    original = (prior / 'plan.json').read_bytes()
    plan = json.loads(original)
    receipt = (audit / 'result.json').read_bytes()
    results = json.loads(receipt)
    if (plan['root'], plan['source'], len(plan['files'])) != ('SourceKeyEncodeProof', capture.SOURCE, 44):
        raise ValueError('the original SQLite history view is required')
    if (results['complete'] is not True or results['source_sha256'] != plan['files']
            or results['physical_files'] != 44 or len(results['audited']) != 108
            or any(results[key] for key in ('errors', 'warnings', 'rejected'))):
        raise ValueError('the original complete history audit is required')
    read_exact(audit / 'service.log', results['service_log_sha256'])
    if closure(prior, ['SourceKeyEncodeProof.lean'], active_view='lakefile.toml') != plan['files']:
        raise ValueError('the original history proof closure differs')

    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        link = output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    for name, digest in plan['files'].items():
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(read_exact(prior / name, digest))
    root = HERE.parents[4]
    support = {}
    for name in SUPPORT:
        payload = (root / name).read_bytes()
        if (prior / name).read_bytes() != payload:
            raise ValueError('the original tool or service support changed: ' + name)
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(payload)
        support[name] = sha256(payload).hexdigest()
    configuration = (prior / 'lakefile.toml').read_text()
    for name in VIEWS:
        configuration += f'\n[[lean_lib]]\nname = "{name}"\n'
        template = view if name == 'SourceLoadView' else (HERE / (name + '.lean.in')).read_bytes()
        if name == ROOT and template.count(b'"' + capture.SELECT.encode() + b'"') != 1:
            raise ValueError('the statement used by the A4 snapshot must match the original SELECT')
        (output / (name + '.lean')).write_bytes(template)
    (output / 'lakefile.toml').write_text(configuration)

    files = closure(output, [f'{ROOT}.lean'], active_view='lakefile.toml')
    added = {name + '.lean' for name in VIEWS}
    if files.keys() != set(plan['files']) | added or any(files[key] != val for key, val in plan['files'].items()):
        raise ValueError('the materialized closure or an inherited file differs')
    delivered = dict(root=ROOT, source=capture.SOURCE, prior=sha256(original).hexdigest(),
                     audit=sha256(receipt).hexdigest(), actual=extracted, fragments=fragments,
                     support=support, files=files, strict_files=sorted(files),
                     modules={name: {'sha256': digest} for name, digest in files.items()},
                     build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(delivered, indent=2) + '\n')
    print(ROOT, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    names = ('actual', 'prior', 'audit', 'output', 'lake-packages', 'lean-mcp')
    for name in names:
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(*(getattr(args, name.replace('-', '_')).resolve(strict=name != 'output') for name in names))
