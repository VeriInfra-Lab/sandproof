"""Recheck #986's exact captures and append the original row-layout contracts."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'source-row-usage'))
_spec = importlib.util.spec_from_file_location('source_row_usage_prepare', HERE.parent / 'source-row-usage/prepare.py')
usage = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(usage)
from prepare_world_begin import closure, read_exact

ROOTS = {'original': 'SourceRowLayout', 'drop-terminator-byte': 'SourceRowLayoutNegative'}


def prepare(actual, prior, audit, charge_prior, charge_audit, output, packages, service):
    old = json.loads((prior / 'plan.json').read_bytes())
    variant = old['variant']
    if variant not in ROOTS or (old['root'], old['source'], len(old['files'])) != (
            usage.ROOTS[variant], usage.extract.metadata.SOURCE, 58):
        raise ValueError('the exact original #986 parent is required')
    receipt = (audit / 'result.json').read_bytes()
    completed = json.loads(receipt)
    if (completed['complete'] is not True or completed['source_sha256'] != old['files']
            or completed['physical_files'] != 58 or len(completed['audited']) != 197
            or any(completed[name] for name in ('errors', 'warnings', 'rejected'))):
        raise ValueError('the complete original #986 service audit is required')
    read_exact(audit / 'service.log', completed['service_log_sha256'])
    if closure(prior, [old['root'] + '.lean'], active_view='lakefile.toml') != old['files']:
        raise ValueError('the exact #986 input closure differs')
    # Run the unchanged parent's full source reconstruction, tool receipts,
    # mapping/type/callee admission and materializer. No new Rust capture is
    # claimed when the complete old inputs and evidence are reused here.
    with TemporaryDirectory(prefix='source-row-layout-') as temporary:
        fresh = Path(temporary) / 'parent'
        usage.prepare(actual, charge_prior, charge_audit, fresh, packages, service)
        if (fresh / 'plan.json').read_bytes() != (prior / 'plan.json').read_bytes():
            raise ValueError('the original parent capture or complete plan differs')
        for name, wanted in old['files'].items():
            if read_exact(prior / name, wanted) != (fresh / name).read_bytes():
                raise ValueError('the original complete #986 view differs: ' + name)
    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        link = output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    for name, wanted in old['files'].items():
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(read_exact(prior / name, wanted))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        payload = (usage.extract.metadata.sql.ROOT / name).read_bytes()
        if payload != (prior / name).read_bytes():
            raise ValueError('the fixed tool and service support differs')
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(payload)
    modules = ('SourceRowLayoutSpec', ROOTS[variant])
    lake = (prior / 'lakefile.toml').read_text()
    for module in modules:
        (output / (module + '.lean')).write_bytes((HERE / (module + '.lean.in')).read_bytes())
        lake += f'\n[[lean_lib]]\nname = "{module}"\n'
    (output / 'lakefile.toml').write_text(lake)
    files = closure(output, [ROOTS[variant] + '.lean'], active_view='lakefile.toml')
    if files.keys() != set(old['files']) | {m + '.lean' for m in modules} or any(files[n] != h for n, h in old['files'].items()):
        raise ValueError('the complete inherited and new closure is required')
    plan = dict(root=ROOTS[variant], variant=variant, source=usage.extract.metadata.SOURCE,
                prior=sha256((prior / 'plan.json').read_bytes()).hexdigest(),
                audit=sha256(receipt).hexdigest(), actual=old['actual'], files=files,
                strict_files=sorted(files), modules={n: {'sha256': h} for n, h in files.items()},
                build_order=[n.removesuffix('.lean') for n in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOTS[variant], len(files), 'exact physical Lean files')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('actual', 'prior', 'audit', 'charge-prior', 'charge-audit', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(*(getattr(args, n.replace('-', '_')).resolve(strict=n != 'output') for n in
              ('actual', 'prior', 'audit', 'charge-prior', 'charge-audit', 'output', 'lake-packages', 'lean-mcp')))
