"""Restore the exact previously checked wrong decoder, without editing generated bodies."""

import json
import re
import shutil

from decision_inputs import HERE, retention
from load_prepare import read_exact


def materialize(out, parent, old):
    from prepare_world_begin import closure

    out.mkdir(mode=0o700)
    for relative in ('.lake/packages', '.tools/lean-mcp'):
        link = out / relative
        link.parent.mkdir()
        link.symlink_to((parent / relative).resolve(strict=True))
    prior = json.loads((parent / 'plan.json').read_bytes())
    for name in prior['files']:
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_exact(parent, prior, name))
    historical = json.loads((old / 'plan.json').read_bytes())
    for name in ('SaveLoadCollectorWrong.lean', 'SaveLoadCollectorWitness.lean'):
        selected = read_exact(old, historical, name)
        target = out / name
        if target.exists() and target.read_bytes() != selected:
            raise ValueError('retained previous declaration differs: ' + name)
        target.write_bytes(selected)
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(parent / name, path)
    local = ('SaveLoadDecision', 'SaveLoadDecisionWitness')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    for name in ('SaveLoadCollectorWrong', 'SaveLoadCollectorWitness'):
        if f'name = "{name}"' not in lake:
            raise ValueError('the old actual library must already be registered locally: ' + name)
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in local)
    (out / 'lakefile.toml').write_text(lake)
    root = 'SaveLoadDecisionWitness'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
