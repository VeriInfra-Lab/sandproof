"""Assemble actual record-byte and digest views from fixed admitted Rust declarations."""

import json
import re
import shutil

from canonical_inputs import HERE, retention
from rows_prepare import declarations
from load_prepare import read_exact


def canonical_view(generated):
    types=declarations(generated/'Types.lean')
    funs=declarations(generated/'Funs.lean')
    root='controller.lifecycle_inbox.source_work.codec'
    checked=root+'.checked_source_payload'; anchor=root+'.encode_load_anchor'
    digest='controller.lifecycle_inbox.source_work.digest'; constructor='sandproof_protocol.lifecycle.digest.ContentDigest.from_sha256'
    closure=checked+'.closure'; once=closure+'.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError'
    scope='SandboxModel.Stage1Protocol.SaveLoadCanonicalView'
    head=f"""import SaveLoadCollectorComposition

open Aeneas Aeneas.Std Result ControlFlow SandproofSaveRetentionRows
open controller.lifecycle_inbox.source_work
set_option autoImplicit false
set_option linter.style.longLine false

namespace {scope}
open SaveLoadCollectorView SaveLoadCursorView
noncomputable section

structure Codec where
  parse : Slice U8 → Result (core.result.Result (Option LoadedRecord) serde_json.error.Error)
  serialize : Option LoadedRecord → Result (core.result.Result Bytes serde_json.error.Error)
  checksum : Slice U8 → Result (Array U8 32#usize)
  anchorJson : SourceAnchor → Result (core.result.Result Bytes serde_json.error.Error)
  workHash : Slice U8 → Result (Array U8 32#usize)

local notation "Global" => Unit
local notation "Slice.Insts.CoreCmpPartialEqSlice" => SaveLoadSql.bytesCompare
local notation "alloc.vec.Vec.Insts.CoreCmpPartialEqShared0Slice.ne" => SaveLoadSql.vecBytesDiffer
local notation "JournalErrorFromJson" => SandproofStage1RequestRust.sandproof_controller_service.JournalErrorFromJson

"""
    head+=types[closure]+'\n\n'+funs[constructor]+'\n\n'+'set_option linter.unusedVariables false in\n'+funs[once+'.call_once']+'\n\n'+funs[once]+f"""

section
variable (codec : Codec)
local notation "{root}.decode_source_payload" => codec.parse
local notation "{root}.encode_source_payload" => codec.serialize
local notation "{root}.digest_source_payload" => codec.checksum
local notation "{root}.encode_anchor_json" => codec.anchorJson
local notation "controller.lifecycle_inbox.sha256.digest" => codec.workHash

"""
    head+=funs[digest]+'\n\n'+funs[anchor]+'\n\n'+funs[checked]+f"""

end
abbrev checked := @{checked}
abbrev anchor := @{anchor}
abbrev workDigest := @{digest}

def collector (codec : Codec) : Ports where
  checked := checked codec
  anchor := anchor codec
  digest := workDigest codec

end
end {scope}
"""
    return head


def wrong_view(generated):
    name='controller.lifecycle_inbox.source_work.codec.checked_source_payload'
    body=declarations(generated/'Funs.lean')[name]
    head="""import SaveLoadCanonicalView

open Aeneas Aeneas.Std Result ControlFlow SandproofSaveRetentionRows
set_option autoImplicit false
set_option linter.style.longLine false

namespace SandboxModel.Stage1Protocol.SaveLoadCanonicalWrong
open SaveLoadCanonicalView SaveLoadCollectorView SaveLoadCursorView
noncomputable section

section
variable (codec : Codec)
local notation "controller.lifecycle_inbox.source_work.codec.decode_source_payload" => codec.parse
local notation "controller.lifecycle_inbox.source_work.codec.digest_source_payload" => codec.checksum
local notation "Slice.Insts.CoreCmpPartialEqSlice" => SaveLoadSql.bytesCompare
local notation "controller.lifecycle_inbox.source_work.codec.checked_source_payload.closure.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError" =>
  SaveLoadCanonicalView.controller.lifecycle_inbox.source_work.codec.checked_source_payload.closure.Insts.CoreOpsFunctionFnOnceTupleErrorJournalError

"""
    return head+body+f'\n\nend\nabbrev checked := @{name}\nend\nend SandboxModel.Stage1Protocol.SaveLoadCanonicalWrong\n'


def materialize(out, parent, good, bad):
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
        'SaveLoadCanonicalView': canonical_view(good),
        'SaveLoadCanonicalWrong': wrong_view(bad),
    }
    for name, source in files.items():
        (out / (name + '.lean')).write_text(source)
    local = ('SaveLoadCanonicalProof', 'SaveLoadCanonicalComposition', 'SaveLoadCanonicalWitness')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in (*files, *local))
    (out / 'lakefile.toml').write_text(lake)
    root = 'SaveLoadCanonicalWitness'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
