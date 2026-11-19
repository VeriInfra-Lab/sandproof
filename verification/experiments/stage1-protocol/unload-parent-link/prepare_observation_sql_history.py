"""Attach the source-row history to the complete original Drop proof view."""
import argparse
from hashlib import sha256
import json
from pathlib import Path

from observation_drop_capture import SOURCE
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).with_name('observation-sql-history')
ROOT = 'ObservationSqlHistory'
VIEWS = ('SourceHistory', ROOT)
SUPPORT = ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py',
           'scripts/lean-mcp-server.sh')


def prepare(prior, audit, output, packages, service):
    prior_bytes = (prior / 'plan.json').read_bytes()
    audit_bytes = (audit / 'result.json').read_bytes()
    plan, receipt = json.loads(prior_bytes), json.loads(audit_bytes)
    if (plan['root'], plan['source'], len(plan['files'])) != ('ObservationDrop', SOURCE, 39):
        raise ValueError('the original ObservationDrop capture and exact closure are required')
    if (receipt['complete'] is not True or receipt['physical_files'] != 39
            or len(receipt['audited']) != 82 or receipt['source_sha256'] != plan['files']
            or any(receipt[field] for field in ('own_warnings', 'errors', 'rejected'))):
        raise ValueError('the original complete ObservationDrop Lean service receipt is required')
    read_exact(audit / 'service.log', receipt['service_log_sha256'])
    if closure(prior, ['ObservationDrop.lean'], active_view='lakefile.toml') != plan['files']:
        raise ValueError('the original scope or dependency closure changed')

    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        link = output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)

    for name, digest in plan['files'].items():
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_exact(prior / name, digest))
    support = {}
    project = HERE.parents[4]
    for name in SUPPORT:
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        content = (project / name).read_bytes()
        if (prior / name).read_bytes() != content:
            raise ValueError(f'original service or toolchain input differs: {name}')
        path.write_bytes(content)
        support[name] = sha256(content).hexdigest()

    added = {name + '.lean' for name in VIEWS}
    for name in added:
        (output / name).write_bytes((HERE / (name + '.in')).read_bytes())
    configuration = (prior / 'lakefile.toml').read_text()
    for name in VIEWS:
        configuration += f'\n[[lean_lib]]\nname = "{name}"\n'
    (output / 'lakefile.toml').write_text(configuration)
    support['lakefile.toml'] = sha256(configuration.encode()).hexdigest()
    files = closure(output, [f'{ROOT}.lean'], active_view='lakefile.toml')
    if ({name: value for name, value in files.items() if name not in added} != plan['files']
            or added - files.keys()):
        raise ValueError('unexpected history proof imports or changed inherited dependencies')
    result = dict(root=ROOT, source=SOURCE, prior_plan=sha256(prior_bytes).hexdigest(),
                  prior_service_receipt=sha256(audit_bytes).hexdigest(), support=support,
                  files=files, strict_files=sorted(files),
                  modules={name: {'sha256': digest} for name, digest in files.items()},
                  build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(result, indent=2) + '\n')
    print(ROOT, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    names = ('prior', 'audit', 'output', 'lake-packages', 'lean-mcp')
    for name in names:
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(*(getattr(args, name.replace('-', '_')).resolve(strict=name != 'output') for name in names))
