"""Materialize the original maintenance reader proof on the current save carrier."""

from hashlib import sha256
import json
import re
from pathlib import Path
import shutil
import sys

HERE = Path(__file__).resolve().parent
CAPACITY = HERE.with_name('source-save-capacity')
sys.path.insert(0, str(CAPACITY))
from capacity_prepare import declarations
from policy_inputs import retention

POLICY_FILES = ('SourcePolicyReadView', 'SourcePolicyIteration', 'SourcePolicyCapacityBounds',
                'SourcePolicyCapacityValidation', 'SourcePolicyMaintenance', 'SourcePolicyConfigRead',
                'SourcePolicyConfigComposition', 'SourcePolicyValues', 'SourcePolicyCheckedArithmetic',
                'SourcePolicyNormalValues', 'SourcePolicyReadSpec')
LEGACY_ONLY = {'controller.checkpoint.config_using', 'controller.checkpoint.proof_source_policy_read',
               'controller.checkpoint.proof_source_policy_effects'}
EMPTY_PARAMS = tuple('ArrayShared0P0.Insts.' + suffix for suffix in
                     ('RusqliteParamsSealedSealed', 'RusqliteParamsParams.__bind_in', 'RusqliteParamsParams'))


def read_exact(base, plan, name):
    data = (base / name).read_bytes()
    if sha256(data).hexdigest() != plan['files'][name]:
        raise ValueError('checked Lean source differs: ' + name)
    return data


def original_reader_view(provider, generated):
    view = provider.replace('SandproofSourceRowSqlSelect', 'SandproofSaveRetentionRows')
    view = view.replace('abbrev controller.capacity.CapacityConfig := SandboxModel.Stage1Protocol.SourceRecordBoundsView.Capacity',
        'abbrev controller.capacity.CapacityConfig := SandproofSaveRetentionRows.controller.capacity.CapacityConfig')
    start = view.index('local notation "ArrayShared0P0.Insts.RusqliteParamsParams"')
    end = view.index('@[reducible]', start)
    prior = declarations(generated / 'Funs.lean')
    adapter = '''private abbrev EmptyArgument (_ : Type → Type) := Unit
local notation "Dyn" => EmptyArgument

'''
    adapter += '\n\n'.join(
        ('set_option linter.style.nameCheck false in\n' if name.endswith('.__bind_in') else '') + prior[name]
        for name in EMPTY_PARAMS)
    adapter += '''

def policyFromBlob (value : rusqlite.types.value_ref.ValueRef) :
    Result (core.result.Result (alloc.vec.Vec U8) rusqlite.types.from_sql.FromSqlError) :=
  match value with
  | .Blob bytes => .ok (.Ok ⟨bytes⟩)
  | _ => .ok (.Err .invalidType)

local notation "alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql.column_result" => policyFromBlob

'''
    adapter += prior['alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql'] + '\n\n'
    view = view[:start] + adapter + view[end:]
    view = view[:view.index('\ndef controller.checkpoint.config_using\n')]
    return view + '\nend\nend sandproof_controller_service\nend SandproofStage1RequestRust\n'


def standard_lemmas(original):
    body = original[:original.index('theorem actual_effects_root')]
    body += original[original.index('theorem actual_standard_read'):original.index('theorem original_count_from_standard_read')]
    body += 'end\nend SandboxModel.Stage1Protocol.SourcePolicyRead\n\n'
    for name in ('actual_contract', 'actual_standard_read', 'standard_success_uses_validated_policy',
                 'standard_success_uses_full_model'):
        body += '#audit_controller_proof SandboxModel.Stage1Protocol.SourcePolicyRead.' + name + '\n'
    return body


def mutant_reader_view(generated):
    functions = declarations(generated / 'Funs.lean')
    head = """import PolicySaveContract

open Aeneas Aeneas.Std Result ControlFlow
open SandproofSaveRetentionRows hiding toStr
open SandproofStage1RequestRust.sandproof_controller_service
open SandproofStage1RequestRust (toStr)
open SandboxModel.Stage1Protocol.SourcePolicyRead (standardEffects)
open SandboxModel.Stage1Protocol.SourcePolicyReadSpec (DeserializeMaintenance DecodeSlice)
set_option autoImplicit false

namespace SandproofStage1RequestRust.sandproof_controller_service.MutantSavePolicy

noncomputable section
section

variable (deserialize : DeserializeMaintenance) (fromSlice : DecodeSlice)

local notation "controller.checkpoint.config.StandardConfigRead.Insts.Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects" =>
  standardEffects deserialize fromSlice

"""
    read = 'controller.checkpoint.config.read'
    limit = 'controller.checkpoint.config'
    return (head + '\n\n'.join([functions[read + '_using'], functions[read],
        f'local notation "{read}" => {read} deserialize fromSlice', functions[limit],
        f'abbrev mutantRead : DeserializeMaintenance → DecodeSlice → SandboxModel.Stage1Protocol.SourceRecordBoundsView.PolicyRead := @{limit}'])
        + '\n\nend\nend\nend SandproofStage1RequestRust.sandproof_controller_service.MutantSavePolicy\n')


def materialize(out, base, provider, generated, mutant, original):
    from prepare_world_begin import closure
    out.mkdir(mode=0o700)
    for relative in ('.lake/packages', '.tools/lean-mcp'):
        link = out / relative
        link.parent.mkdir()
        link.symlink_to((base / relative).resolve(strict=True))
    plans = [json.loads((directory / 'plan.json').read_bytes()) for directory in (base, provider)]
    for name in plans[0]['files']:
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_exact(base, plans[0], name))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(base / name, path)
    for filename in ('Types.lean', 'Funs.lean'):
        old = declarations(original / filename)
        now = declarations(generated / filename)
        if {name for name, source in old.items() if now.get(name) != source} != (
                LEGACY_ONLY if filename == 'Funs.lean' else set()):
            raise ValueError('original policy declarations have changed: ' + filename)
    for module in POLICY_FILES:
        name = module + '.lean'
        data = read_exact(provider, plans[1], name).decode().replace('SandproofSourceRowSqlSelect', 'SandproofSaveRetentionRows')
        if module == 'SourcePolicyReadView':
            data = original_reader_view(data, generated)
        (out / name).write_text(data)
    (out / 'SourcePolicyStandard.lean').write_text(
        standard_lemmas(read_exact(provider, plans[1], 'SourcePolicyRead.lean').decode()))
    local = ('PolicySaveContract', 'PolicySave', 'PolicyOrderProjection', 'PolicyWitness',
             'PolicyCounterexample', 'PolicyJoint')
    for module in local:
        shutil.copyfile(HERE / (module + '.lean'), out / (module + '.lean'))
    (out / 'PolicyMutantReader.lean').write_text(mutant_reader_view(mutant))
    lake = (base / 'lakefile.toml').read_text()
    for name in (*POLICY_FILES, 'SourcePolicyStandard', 'PolicyMutantReader', *local):
        lake += f'\n[[lean_lib]]\nname = "{name}"\n'
    (out / 'lakefile.toml').write_text(lake)
    root = 'PolicyJoint'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
