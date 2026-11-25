"""Join two exact checked source closures without remaking the actual programs."""
import re
import shutil

from full_inputs import HERE, retention
from load_prepare import read_exact


def materialize(out, parent, prior, old, old_plan):
    from prepare_world_begin import closure

    out.mkdir(mode=0o700)
    for relative in ('.lake/packages', '.tools/lean-mcp'):
        link = out / relative
        link.parent.mkdir()
        target = (parent / relative).resolve(strict=True)
        if target != (old / relative).resolve(strict=True):
            raise ValueError('the exact dependency provider must match: ' + relative)
        link.symlink_to(target)
    for name in old_plan['files'].keys() & prior['files'].keys():
        if read_exact(old, old_plan, name) != read_exact(parent, prior, name):
            raise ValueError('previous checked common source differs: ' + name)
    for project, plan in ((old, old_plan), (parent, prior)):
        for name in plan['files']:
            path = out / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(read_exact(project, plan, name))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if (parent / name).read_bytes() != (old / name).read_bytes():
            raise ValueError('different checked environment input: ' + name)
        shutil.copyfile(parent / name, path)
    local = ('SourceSaveFull', 'SourceSaveFullWitness')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    for name in re.findall(r'^name = "([^"\n]+)"$', (old / 'lakefile.toml').read_text(), re.M):
        if f'name = "{name}"' not in lake:
            raise ValueError('the exact prior library registration must be retained: ' + name)
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in local)
    (out / 'lakefile.toml').write_text(lake)
    root = 'SourceSaveFullWitness'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    if not old_plan['files'].keys() | prior['files'].keys() <= files.keys():
        raise ValueError('all previously checked actual files must stay in the full root')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
