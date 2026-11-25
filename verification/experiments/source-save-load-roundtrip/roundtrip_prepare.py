"""Materialize the exact same original types/functions with the historical actual wrong step."""

import json
import re
import shutil

from roundtrip_inputs import HERE, retention
from rows_prepare import declarations
from load_prepare import read_exact


def wrong_view(parent, generated):
    parent_plan = json.loads((parent / 'plan.json').read_bytes())
    old = read_exact(parent, parent_plan, 'EncoderPorts.lean').decode()
    prefix = old[old.index('section\nvariable {Keys'):old.index('@[rust_loop_body]')]
    funs = declarations(generated / 'Funs.lean')
    name = 'controller.lifecycle_inbox.source_work.codec.encode_candidate_records'
    ns = 'SandboxModel.Stage1Protocol.SaveRoundTripWrong'
    header = """import SaveRoundTripEncoder

open Aeneas Aeneas.Std Result ControlFlow SandproofSaveRetentionRows
open controller.lifecycle_inbox.source_work controller.lifecycle_inbox.source_work.codec
set_option autoImplicit false
set_option linter.style.longLine false

namespace SandboxModel.Stage1Protocol.SaveRoundTripWrong
noncomputable section

"""
    return (header + prefix + funs[name + '_loop.body']
            + f'\n\nlocal notation "{name}_loop.body" =>\n  {name}_loop.body ports\n\n'
            + funs[name + '_loop'] + f'\n\nlocal notation "{name}_loop" =>\n  {name}_loop ports\n\n'
            + funs[name] + f'\n\nend\n\nabbrev encodeCandidate := @{name}\nabbrev encodeLoop := @{name}_loop\n'
            + f'abbrev encodeStep := @{name}_loop.body\n\nend\nend {ns}\n')


def materialize(out, parent, bad):
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
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(parent / name, path)
    files = {
        'SaveRoundTripWrong': wrong_view(parent, bad),
    }
    for name, source in files.items():
        (out / (name + '.lean')).write_text(source)
    local = ('SaveRoundTripEncoder', 'SaveRoundTripShared', 'SaveRoundTripWitness')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in (*files, *local))
    (out / 'lakefile.toml').write_text(lake)
    root = 'SaveRoundTripWitness'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
