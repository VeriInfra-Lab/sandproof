"""Rebuild the capacity proof from exact, byte-preserved generated declarations."""

from hashlib import sha256
import json
import re
import shutil

from capacity_inputs import CODEC, COUNT_PROVIDER, HERE, retention

OLD = retention.OLD_PACKAGE


def pinned(file):
    return retention.pinned(COUNT_PROVIDER, OLD / file).decode()


def declarations(path):
    found = {}
    for block in path.read_bytes().split(b'\n\n/--'):
        match = re.search(rb'^(?:def|impl_def|structure|inductive)\s+([^\s(:]+)', block, re.M)
        if match:
            begin = block.rfind(b'-/\n', 0, match.start())
            if begin < 0:
                raise ValueError('missing original declaration boundary')
            name = match[1].decode()
            if name in found:
                raise ValueError('duplicate declaration: ' + name)
            found[name] = block[begin + 3:].removesuffix(b'\n\nend ' + path.parent.name.encode() + b'\n').decode()
    return found


def audit_commands(text, namespace):
    return ''.join('#audit_controller_proof SandboxModel.Stage1Protocol.' + namespace + '.' + name + '\n'
                   for name in re.findall(r'^theorem (\w+)', text, re.M))


def materialize(out, variant, base, capture):
    from prepare_world_begin import closure
    out.mkdir(mode=0o700)
    for relative in ('.lake/packages', '.tools/lean-mcp'):
        link = out / relative
        link.parent.mkdir()
        link.symlink_to((base / relative).resolve(strict=True))
    parent = json.loads((base / 'plan.json').read_bytes())
    for name, digest in parent['files'].items():
        data = (base / name).read_bytes()
        if sha256(data).hexdigest() != digest:
            raise ValueError('parent input changed: ' + name)
        file = out / name
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(data)
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        (out / name).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(base / name, out / name)
    types = declarations(capture / 'Types.lean')
    functions = declarations(capture / 'Funs.lean')
    codec = CODEC
    view = pinned('source-record-count/SourceRecordCountView.lean.in')
    view = view.replace('import SourceReservation','''import EncoderComposition

open Aeneas Aeneas.Std
namespace SandproofSaveRetentionRows
''' + '\n\n'.join(types[name] for name in ('controller.capacity.CapacityConfig',
    'controller.checkpoint.config.MaintenanceConfig')) + '''
end SandproofSaveRetentionRows

namespace SandboxModel.Stage1Protocol.SourceRecordBoundsView
abbrev Policy := SandproofSaveRetentionRows.controller.checkpoint.config.MaintenanceConfig
abbrev PolicyRead := SaveRows.Db → Result (core.result.Result (Option Policy) SandproofSaveRetentionRows.JournalError)
end SandboxModel.Stage1Protocol.SourceRecordBoundsView

import_placeholder'''.removesuffix('\nimport_placeholder'))
    view = view.replace('open SandproofSourceRowSqlSelect hiding toStr','open SandproofSaveRetentionRows')
    view = view.replace('-- ACTUAL PER SESSION', functions[codec + 'records_per_session'])
    view = view.replace('-- ACTUAL RESERVED RECORDS', functions[codec + 'reserved_records'])
    (out / 'SourceRecordCountView.lean').write_text(view)

    spec = pinned('source-reservation/SourceReservationSpec.lean.in')
    start = spec.index('theorem try_mk_success_iff')
    end = spec.index('theorem single_typed_fits', start)
    scalar = 'import SourceRecordCountView\n\nopen Aeneas Aeneas.Std\n\n'
    scalar += 'namespace SandboxModel.Stage1Protocol.SourceReservationSpec\n\n' + spec[start:end]
    scalar += 'end SandboxModel.Stage1Protocol.SourceReservationSpec\n\n'
    scalar += audit_commands(spec[start:end], 'SourceReservationSpec')
    (out / 'SourceCountScalar.lean').write_text(scalar)
    contract = pinned('source-record-count/SourceRecordCountSpec.lean.in')
    contract = contract.replace('import SourceRecordCountView','import SourceCountScalar')
    contract = contract.replace('(Policy PolicyRead Config Capacity)', '(Policy PolicyRead)')
    contract = contract.replace('SaveRows.Error', 'SandproofSaveRetentionRows.JournalError')
    (out / 'SourceRecordCountSpec.lean').write_text(contract)
    proof = pinned('source-record-count/SourceRecordCount.lean.in')
    proof = proof.replace('(Policy PolicyRead Config Capacity)', '(Policy PolicyRead)')
    proof = proof.replace('SaveRows.Error', 'SandproofSaveRetentionRows.JournalError')
    keep = proof[proof.index('theorem given_policy'):proof.index('theorem server_session_rows_bounded')]
    proof = proof[:proof.index('theorem original_is_actual')] + keep
    if variant == 'correct':
        proof += 'end SandboxModel.Stage1Protocol.SourceRecordCount\n\n'
        proof += audit_commands(proof, 'SourceRecordCount')
        (out / 'SourceRecordCount.lean').write_text(proof)
    historical = pinned('source-record-bounds/SourceRecordBoundsContract.lean.in')
    parts = [
        ('theorem multiply_exact', 'theorem add_exact'),
        ('theorem subtract_exact', 'theorem deployment_ceiling'),
        ('def sampleCapacity', 'def samplePolicy'),
        ('/-- One original six-column aggregate', 'def sampleScan'),
    ]
    samples = "import CapacityInputs\n\nopen Aeneas Aeneas.Std\n\n"
    samples += "namespace SandboxModel.Stage1Protocol.SourceRecordBoundsContract\n"
    samples += "abbrev Capacity := SandproofSaveRetentionRows.controller.capacity.CapacityConfig\n\n"
    samples += "".join(historical[historical.index(lo):historical.index(hi)] for lo, hi in parts)
    samples += "end SandboxModel.Stage1Protocol.SourceRecordBoundsContract\n"
    samples += '\n' + audit_commands(samples, 'SourceRecordBoundsContract')
    (out / 'CapacityHistoricalSamples.lean').write_text(samples)
    final = 'CapacityCorrect' if variant == 'correct' else 'CapacityWrong'
    local = ['CapacityInputs', 'CapacityWitness', final]
    if variant == 'correct':
        local.append('CapacitySave')
    for name in local:
        shutil.copyfile(HERE / (name + '.lean'), out / (name + '.lean'))
    lake = (base / 'lakefile.toml').read_text()
    added = ['SourceRecordCountView', 'SourceCountScalar', 'SourceRecordCountSpec', 'CapacityHistoricalSamples', *local]
    if variant == 'correct':
        added.append('SourceRecordCount')
    for name in added:
        lake += f'\n[[lean_lib]]\nname = "{name}"\n'
    (out / 'lakefile.toml').write_text(lake)
    files = closure(out, [final + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=final, mode=variant, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files], public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
