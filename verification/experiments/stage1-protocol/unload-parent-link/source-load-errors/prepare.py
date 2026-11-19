"""Materialize the original first-error proof on the audited source guard closure."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
from tempfile import TemporaryDirectory

import extract
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact
from revision_capture import ir_values
from frontier_capture import call_kind

HERE = Path(__file__).resolve().parent
ROOTS = {'original': 'SourceLoadErrors', 'early-count-guard': 'SourceLoadErrorsNegative'}


def prepare(actual, prior, audit, output, packages, service):
    result = json.loads((actual / 'result.json').read_bytes())
    variant = result['variant']
    if (result['complete'] is not True or result['source'] != extract.metadata.SOURCE
            or result['tree'] != extract.metadata.TREE or result['options'] != extract.metadata.CFG
            or result['driver'] != extract.metadata.sql.digest(Path(extract.__file__))
            or variant not in ROOTS):
        raise ValueError('the exact completed source-load capture is required')
    if [(s['label'], s['exit_code'], s['bounded_stop']) for s in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed tools must complete normally')
    archive = actual / 'source.tar'
    read_exact(archive, extract.metadata.ARCHIVE)
    for command in result['commands']:
        read_exact(actual / (command['label'] + '.log'), command['log_sha256'])
    with TemporaryDirectory(prefix='source-load-errors-') as fresh:
        source = Path(fresh)
        codec = extract.metadata.reconstruct_codec(source, archive, 'original').decode()
        if variant == 'early-count-guard':
            codec = extract.early_count_guard(codec)
        if read_exact(actual / 'source' / extract.metadata.sql.CODEC,
                      result['staged'][str(extract.metadata.sql.CODEC)]) != codec.encode():
            raise ValueError('the original metadata caller or sole early guard differs')
        if read_exact(actual / 'source' / extract.CONFIG,
                      result['staged'][str(extract.CONFIG)]) != (source / extract.CONFIG).read_bytes():
            raise ValueError('the original source guard differs')
    ir = actual / 'actual.llbc'
    read_exact(ir, result['ir'])
    gen = actual / 'generated'
    expected = {'translation.json', *(extract.NAMESPACE + '/' + name + '.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if set(result['generated']) != expected or {str(p.relative_to(gen)) for p in gen.rglob('*') if p.is_file()} != expected:
        raise ValueError('the complete actual generation inventory differs')
    for name, wanted in result['generated'].items():
        read_exact(gen / name, wanted)
    fixed = json.loads((extract.metadata.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    if extract.metadata.admit(ir, gen, fixed['target']) != result['admission']:
        raise ValueError('the original function mapping differs')
    raw = json.loads(ir.read_bytes())['translated']
    load = next(f for f in raw['fun_decls'] if f and f['def_id'] == result['admission'][extract.metadata.LOAD])
    used = [c for c in ir_values(load['body'], 'Call')
            if call_kind(c['call']) == {'Fun': result['admission']['valid']}]
    if len(used) != 1:
        raise ValueError('the actual metadata caller must invoke this exact guard once')
    functions = declarations((gen / extract.NAMESPACE / 'Funs.lean').read_bytes().removesuffix(
        b'\n\nend ' + extract.NAMESPACE.encode() + b'\n'))
    body = functions['controller.lifecycle_inbox.source_work.codec.load']
    view = (HERE / 'SourceLoadErrorsView.lean.in').read_bytes()
    if view.count(b'-- ACTUAL LOAD\n') != 1:
        raise ValueError('the sole exact loader insertion point is required')
    view = view.replace(b'-- ACTUAL LOAD\n', body + b'\n')

    old = json.loads((prior / 'plan.json').read_bytes())
    receipt = (audit / 'result.json').read_bytes()
    completed = json.loads(receipt)
    if (old['root'], old['source'], len(old['files'])) != ('SourceConfigGuards', extract.metadata.SOURCE, 58):
        raise ValueError('the audited original metadata parent is required')
    if (completed['complete'] is not True or completed['source_sha256'] != old['files']
            or completed['physical_files'] != 58 or len(completed['audited']) != 175
            or any(completed[name] for name in ('errors', 'warnings', 'rejected'))):
        raise ValueError('the complete original metadata audit is required')
    read_exact(audit / 'service.log', completed['service_log_sha256'])
    if closure(prior, ['SourceConfigGuards.lean'], active_view='lakefile.toml') != old['files']:
        raise ValueError('the original parent closure differs')
    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        link = output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    for name, wanted in old['files'].items():
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(read_exact(prior / name, wanted))
    support = {}
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        payload = (extract.metadata.sql.ROOT / name).read_bytes()
        if payload != (prior / name).read_bytes():
            raise ValueError('the original service support differs')
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(payload)
        support[name] = sha256(payload).hexdigest()
    modules = ('SourceLoadErrorsView', 'SourceLoadErrorsSpec', ROOTS[variant])
    lake = (prior / 'lakefile.toml').read_text()
    for module in modules:
        payload = view if module == 'SourceLoadErrorsView' else (HERE / (module + '.lean.in')).read_bytes()
        (output / (module + '.lean')).write_bytes(payload)
        lake += f'\n[[lean_lib]]\nname = "{module}"\n'
    (output / 'lakefile.toml').write_text(lake)
    files = closure(output, [ROOTS[variant] + '.lean'], active_view='lakefile.toml')
    if files.keys() != set(old['files']) | {m + '.lean' for m in modules} or any(
            files[name] != wanted for name, wanted in old['files'].items()):
        raise ValueError('the exact inherited and new closure is required')
    plan = dict(root=ROOTS[variant], variant=variant, source=extract.metadata.SOURCE, actual=result,
                prior=sha256((prior / 'plan.json').read_bytes()).hexdigest(),
                audit=sha256(receipt).hexdigest(), support=support, actual_load=sha256(body).hexdigest(),
                files=files, strict_files=sorted(files), modules={n: {'sha256': h} for n, h in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOTS[variant], len(files), 'exact physical Lean files')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('actual', 'prior', 'audit', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(*(getattr(args, name.replace('-', '_')).resolve(strict=name != 'output') for name in
              ('actual', 'prior', 'audit', 'output', 'lake-packages', 'lean-mcp')))
