"""Assemble actual ordered collector views from the admitted Rust declarations."""

import json
import re
import shutil

from collector_inputs import HERE, retention
from rows_prepare import declarations
from load_prepare import read_exact


def collector_view(old, generated):
    functions = declarations(generated / 'Funs.lean')
    root='controller.lifecycle_inbox.source_work.codec'
    decode=root+'.decode_loaded_record'
    loop=root+'.read_loaded_rows_loop'
    reader=root+'.read_loaded_rows'
    ns='SandboxModel.Stage1Protocol.SaveLoadCollectorView'
    head=f"""import SaveLoadCursorComposition
import Aeneas.Std.VecIter

open Aeneas Aeneas.Std Result ControlFlow SandproofSaveRetentionRows
open controller.lifecycle_inbox.source_work
set_option autoImplicit false
set_option linter.style.longLine false

namespace {ns}
open SaveLoadCursorView
noncomputable section

abbrev LoadedRecord := SourceWorkRecord
structure Ports where
  checked : Slice U8 → Slice U8 → Result (core.result.Result (Option LoadedRecord) JournalError)
  anchor : SourceAnchor → Result (core.result.Result Bytes JournalError)
  digest : Slice U8 → Result sandproof_protocol.lifecycle.digest.ContentDigest

"""
    support=old[old.index('def sliceKeyEq'):old.index('def fromSql')]
    view=head+support+f"""
section
variable (ports : Ports)
local notation "Global" => Unit
local notation "alloc.vec.Vec.as_slice" => asSlice
local notation "Slice.Insts.CoreCmpPartialEqArray.eq" => sliceKeyEq
local notation "{root}.checked_source_payload" => ports.checked
local notation "{root}.encode_load_anchor" => ports.anchor
local notation "controller.lifecycle_inbox.source_work.digest" => ports.digest

"""+functions[decode]+f'\n\nlocal notation "{decode}" => {decode} ports\n\n'
    view+=functions[loop+'.body']+f'\n\nlocal notation "{loop}.body" => {loop}.body ports\n\n'
    view+=functions[loop]+f'\n\nlocal notation "{loop}" => {loop} ports\n\n'
    view+=functions[reader]+f"""

end

abbrev decode := @{decode}
abbrev reader := @{reader}
abbrev readLoop := @{loop}
abbrev readStep := @{loop}.body

end
end {ns}
"""
    return view


def wrong_view(generated):
    root = 'controller.lifecycle_inbox.source_work.codec'
    name = root + '.decode_loaded_record'
    bad = generated
    ns='SandboxModel.Stage1Protocol.SaveLoadCollectorWrong'
    body=declarations(bad/'Funs.lean')[name]
    head=f"""import SaveLoadCollectorView

open Aeneas Aeneas.Std Result SandproofSaveRetentionRows
set_option autoImplicit false
set_option linter.style.longLine false

namespace {ns}
open SaveLoadCollectorView
noncomputable section
section
variable (ports : Ports)

local notation "{root}.checked_source_payload" => ports.checked
local notation "{root}.encode_load_anchor" => ports.anchor
local notation "controller.lifecycle_inbox.source_work.digest" => ports.digest

"""
    return head + body + f'\n\nend\nabbrev decode := @{name}\nend\nend {ns}\n'


def materialize(out, parent, reference, good, bad):
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
    baseline = json.loads((reference / 'plan.json').read_bytes())
    old = read_exact(reference, baseline, 'SourceLoadView.lean').decode()
    files = {
        'SaveLoadCollectorView': collector_view(old, good),
        'SaveLoadCollectorWrong': wrong_view(bad),
    }
    for name, source in files.items():
        (out / (name + '.lean')).write_text(source)
    local = ('SaveLoadCollectorProof', 'SaveLoadCollectorComposition', 'SaveLoadCollectorWitness')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in (*files, *local))
    (out / 'lakefile.toml').write_text(lake)
    root = 'SaveLoadCollectorWitness'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
