"""Assemble the fully checked loader project with the live provider bindings."""

import json
import re
import shutil

from payload_inputs import HERE, retention
from load_prepare import declarations, loader_view, read_exact


def capacity_bindings(types, funs):
    sql = 'controller.lifecycle_inbox.source_work.codec.sql'
    cap = 'controller.capacity.config'
    checkpoint = 'controller.checkpoint.config'
    standard = checkpoint + '.StandardConfigRead.Insts.Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects'
    path = 'SandboxModel.Stage1Protocol.SaveLoadCapacity'
    policy = 'SandproofStage1RequestRust.sandproof_controller_service'
    head = f"""import SaveLoadPayload

open Aeneas Aeneas.Std Result
open SandproofSaveRetentionRows
open SandboxModel.Stage1Protocol.SourcePolicyRead (policyRead actual_standard_read standardEffects)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DecodeSlice DeserializeMaintenance)
set_option autoImplicit false
set_option linter.style.longLine false
set_option linter.unusedVariables false

namespace {path}
noncomputable section

local notation "{checkpoint}.ReadConfigEffects" => {policy}.{checkpoint}.ReadConfigEffects
local notation "{checkpoint}.read_using" => {policy}.{checkpoint}.read_using
local notation "{checkpoint}.MaintenanceConfig" => SandproofSaveRetentionRows.{checkpoint}.MaintenanceConfig
local notation "{cap.replace('.config', '.CapacityConfig')}" =>
  SandproofSaveRetentionRows.{cap.replace('.config', '.CapacityConfig')}

"""
    wrappers = '\n\n'.join(funs[name] for name in (checkpoint + '_using', cap + '_using'))
    wrappers += f"""

section
variable (deserialize : DeserializeMaintenance) (decode : DecodeSlice)
local notation "{standard}" => standardEffects deserialize decode

""" + funs[cap] + '\n\nend\n\n'
    signed = """def toSigned (value : U64) : Result (core.result.Result I64 core.num.error.TryFromIntError) :=
  if (value.val : Int) ≤ I64.max then .ok (.Ok (UScalar.hcast .I64 value)) else .ok (.Err ())

local notation "I64.Insts.CoreConvertTryFromU64TryFromIntError.try_from" => toSigned

"""
    signed += '\n\n'.join([types[sql+'.closure'], *(funs[name] for name in (
        sql + '.closure.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError.call_once',
        sql + '.closure.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError', sql))])

    return head + wrappers + signed + f"\n\nabbrev actualRead := @{cap}\nabbrev actualSql := {sql}\n\nend\nend {path}\n"


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
        'SaveLoadCapacityBindings': capacity_bindings(declarations(good / 'Types.lean'),
                                                       declarations(good / 'Funs.lean')),
        'SaveLoadInflatedReserve': loader_view('InflatedReserve', declarations(bad / 'Funs.lean')),
    }
    for name, source in files.items():
        (out / (name + '.lean')).write_text(source)
    local = ('SaveLoadPayload', 'SaveLoadCapacity', 'SaveLoadReserveWitness')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in (*files, *local))
    (out / 'lakefile.toml').write_text(lake)
    root = 'SaveLoadReserveWitness'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
