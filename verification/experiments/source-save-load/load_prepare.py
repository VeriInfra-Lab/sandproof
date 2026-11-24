"""Project the admitted actual loader onto the audited save and SQL types."""

import json
from pathlib import Path
import re
import shutil

from load_inputs import HERE, retention
from policy_prepare import declarations, read_exact

CODEC = 'controller.lifecycle_inbox.source_work.codec'
CAPACITY = 'controller.capacity.CapacityConfig'
CONFIG = CODEC + '.Configuration'
LOAD = CODEC + '.load'

def sql_view(historical, good_types):
    old = historical
    support, _ = old.split('structure Ports (Node', 1)
    support = support.replace('import SourceRecordBounds', 'import PolicyJoint')
    support = support.replace('SandproofSourceRowSqlSelect', 'SandproofSaveRetentionRows')
    support = support.replace(
        'open SandboxModel.Stage1Protocol.SourceRecordBoundsView (Capacity Config Limits)\n'
        'open SandboxModel.Stage1Protocol.SourceKeyEncode (Fields Row)\n', '')
    support = support.replace('set_option autoImplicit false', 'set_option autoImplicit false\nnoncomputable section')
    support = support.replace('namespace SandboxModel.Stage1Protocol.SourceMetadataView',
                              'namespace SandboxModel.Stage1Protocol.SaveLoadSql')
    support = support.replace('=> Limits', '=> SandproofSaveRetentionRows.capture.source.SourceObservationConfig')
    support = support.replace('=> Capacity', '=> SandproofSaveRetentionRows.controller.capacity.CapacityConfig')
    support = '\n'.join(line for line in support.splitlines() if not any(guard in line for guard in (
        '=> SourceRecordBoundsView.toUnsigned', '=> SourceRecordBoundsView.sqlConvert',
        '=> SourceLoadView.fromSql', '=> SourceSqlCursorView.blobDictionary'))) + '\n'
    support = support.replace(
        '  SourceRecordBoundsView.ArrayShared0P0.Insts.RusqliteParamsParams',
        '  SandproofStage1RequestRust.sandproof_controller_service.ArrayShared0P0.Insts.RusqliteParamsParams\n'
        'local notation "alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql" =>\n'
        '  SandproofStage1RequestRust.sandproof_controller_service.alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql')
    start = support.index('structure ' + CODEC + '.Sources')
    support = support[:start] + support[support.index('@[reducible]', start):]
    start = support.index('abbrev Sources (Node')
    support = support[:start] + support[support.index('abbrev currentVersion', start):]
    support = support.replace('structure Codec (Node : Type) where',
        good_types[CONFIG] + '\n\nabbrev StoredConfiguration := ' + CONFIG + '\n\nstructure Codec where')
    support = support.replace('Config Node', 'StoredConfiguration')
    support = support.replace('variable {Node : Type} (codec : Codec Node)', 'variable (codec : Codec)')
    support = support.replace('local notation "' + CONFIG + '" => StoredConfiguration\n', '')
    support = support.replace('set_option linter.style.whitespace false', 'set_option linter.style.whitespace false\nset_option linter.unusedVariables false')
    support += f"""
structure Ports where
  text : TextDecode
  codec : Codec
  capacity : Db → Result (SaveLoadSql.Reply (Option {CAPACITY}))
  sqlInteger : U64 → Result (SaveLoadSql.Reply I64)
  records : Db → StoredConfiguration → {CAPACITY} →
    Result (SaveLoadSql.Reply (alloc.vec.Vec controller.lifecycle_inbox.source_work.SourceWorkRecord))
"""
    return support + '\nend SandboxModel.Stage1Protocol.SaveLoadSql\n'


def loader_view(label, selected):
    forward, = (name for name in selected if '.JournalSourceSave.Insts.' in name and name.endswith('.read'))
    view = f"""import SaveLoadSql

open Aeneas Aeneas.Std Result
open SandproofSaveRetentionRows hiding toStr
open SandboxModel.Stage1Protocol.SaveLoadSql
set_option autoImplicit false
set_option maxRecDepth 4096
set_option linter.style.longLine false
set_option linter.style.whitespace false

namespace SandboxModel.Stage1Protocol.SaveLoad{label}

noncomputable section

private abbrev toStr (text : String)
(bounded : text.toByteArray.size ≤ U32.max := by rw [U32.max_eq]; decide) : Str :=
  SandproofSaveRetentionRows.toStr text bounded


section
variable (ports : Ports)

local notation "Dyn" => (fun (_ : Type → Type) => Unit)
local notation "ArrayShared0P0.Insts.RusqliteParamsParams" =>
  SandproofStage1RequestRust.sandproof_controller_service.ArrayShared0P0.Insts.RusqliteParamsParams
local notation "{CODEC}.schema" => SaveLoadSql.schema ports.text
local notation "{CODEC}.checked_source_configuration" => SaveLoadSql.checked ports.codec
local notation "{CODEC}.load_records" => ports.records
local notation "{CODEC}.sql" => ports.sqlInteger
local notation "controller.capacity.config" => ports.capacity
local notation "capture.source.SourceObservationConfig.valid" => SaveLoadSql.valid
local notation "core.ops.range.RangeInclusive.contains" => SaveLoadSql.rangeContains
def toUnsigned (value : I64) : Result (core.result.Result U64 core.num.error.TryFromIntError) :=
  if 0 ≤ value.val then .ok (.Ok (IScalar.hcast .U64 value)) else .ok (.Err ())

local notation "U64.Insts.CoreConvertTryFromI64TryFromIntError.try_from" => toUnsigned

"""
    view += selected[LOAD]
    view += f'\n\nlocal notation "{LOAD}" => {LOAD} ports\n\n'
    view += selected[forward]
    view += f'\n\nend\n\nabbrev load := @{LOAD}\nabbrev originalRead := @{forward}\n'
    view += f'\nend\nend SandboxModel.Stage1Protocol.SaveLoad{label}\n'
    return view


def materialize(out, parent, reference, good, bad):
    from prepare_world_begin import closure

    out.mkdir(mode=0o700)
    for relative in ('.lake/packages', '.tools/lean-mcp'):
        link = out / relative
        link.parent.mkdir()
        link.symlink_to((parent / relative).resolve(strict=True))
    plans = [json.loads((path / 'plan.json').read_bytes()) for path in (parent, reference)]
    for name in plans[0]['files']:
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_exact(parent, plans[0], name))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(parent / name, path)
    files = {
        'SaveLoadSql': sql_view(read_exact(reference, plans[1], 'SourceMetadataView.lean').decode(),
                               declarations(good / 'Types.lean')),
        'SaveLoadOriginal': loader_view('Original', declarations(good / 'Funs.lean')),
        'SaveLoadEarlyCount': loader_view('EarlyCount', declarations(bad / 'Funs.lean')),
    }
    for name, source in files.items():
        (out / (name + '.lean')).write_text(source)
    local = ('SaveLoadSpec', 'SaveLoadProof', 'SaveLoadWitness')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in (*files, *local))
    (out / 'lakefile.toml').write_text(lake)
    root = 'SaveLoadWitness'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
