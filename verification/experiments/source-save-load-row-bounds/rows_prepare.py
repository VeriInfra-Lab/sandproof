"""Assemble exact generated record guards in the existing typed save project."""

import json
import re
import shutil

from rows_inputs import HERE, retention
from payload_prepare import declarations
from load_prepare import read_exact


def records_view(label, old, gen):
    types, funs = (declarations(gen / name) for name in ('Types.lean','Funs.lean'))
    root = 'controller.lifecycle_inbox.source_work.codec.load_records'
    value = 'controller.lifecycle_inbox.source_work.codec.VALUE_LIMIT'
    scope = 'SandboxModel.Stage1Protocol.SaveLoadRows'
    header = f"""import SaveLoadRecordError

open Aeneas Aeneas.Std Result
open SandproofSaveRetentionRows hiding toStr
open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Policy PolicyRead)
set_option autoImplicit false
set_option maxRecDepth 4096
set_option linter.style.longLine false
set_option linter.unusedVariables false

namespace {scope}{label}

noncomputable section

private abbrev toStr (text : String)
    (bounded : text.toByteArray.size ≤ U32.max := by rw [U32.max_eq]; decide) : Str :=
  SandproofSaveRetentionRows.toStr text bounded

abbrev Bounds := I64 × I64 × I64 × I64 × I64 × I64
abbrev BoundsReply := core.result.Result Bounds rusqlite.error.Error
abbrev Returned := core.result.Result (alloc.vec.Vec controller.lifecycle_inbox.source_work.SourceWorkRecord) JournalError
abbrev Config := SaveLoadSql.StoredConfiguration
abbrev Capacity := controller.capacity.CapacityConfig
abbrev Scan := SaveRows.Db → Result Returned

local notation "Dyn" => (fun (_ : Type → Type) => Unit)
local notation "ArrayShared0P0.Insts.RusqliteParamsParams" =>
  SandproofStage1RequestRust.sandproof_controller_service.ArrayShared0P0.Insts.RusqliteParamsParams

abbrev toUnsigned := SaveLoadOriginal.toUnsigned
abbrev toSigned := SaveLoadCapacity.toSigned

"""
    support = old[old.index('def integerEq'):old.index('local notation "I64.Insts.CoreConvertTryFromU64TryFromIntError.try_from"')]
    support += old[old.index('local notation "TupleXWVUT.Insts.'):old.index('local notation "JournalError.Insts.CoreConvertFromError"')]

    selected = '\n\n'.join([types[name] for name in types if name.startswith(root + '.')] +
        [funs[name] for name in funs if name == value or name.startswith(root + '.')])
    selected += """
abbrev reservation := capture.source.SourceObservationConfig.required_bytes
abbrev sqlConvert := SaveLoadCapacity.actualSql
abbrev valueLimit := controller.lifecycle_inbox.source_work.codec.VALUE_LIMIT
abbrev reserved := SourceRecordCountView.count
"""
    selected += old[old.index('abbrev aggregateMapper'):old.index('abbrev PolicyRead :=')]

    wrapper = f"""
section
variable (policy : PolicyRead) (scanRows : Scan)
local notation "controller.lifecycle_inbox.source_work.codec.Configuration" => Config
local notation "controller.lifecycle_inbox.source_work.codec.reserved_records" => reserved policy
local notation "controller.lifecycle_inbox.source_work.codec.scan_loaded_records" => scanRows
local notation "controller.lifecycle_inbox.source_work.codec.sql" => sqlConvert

""" + funs[root] + f"""

end
abbrev loader := @{root}

end
end {scope}{label}
"""
    return header + support + selected + wrapper


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
    old = read_exact(reference, baseline, 'SourceRecordBoundsView.lean').decode()
    files = {
        'SaveLoadRowsView': records_view('View', old, good),
        'SaveLoadRowsWrong': records_view('Wrong', old, bad),
    }
    for name, source in files.items():
        (out / (name + '.lean')).write_text(source)
    local = ('SaveLoadRecordError', 'SaveLoadRowsSpec', 'SaveLoadRowsProof', 'SaveLoadRowsComposition',
             'SaveLoadRowsWitness', 'SaveLoadRowsNegative')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in (*files, *local))
    (out / 'lakefile.toml').write_text(lake)
    root = 'SaveLoadRowsNegative'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
