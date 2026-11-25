"""Use the exact actual generated function bodies and fully shared parent Serde dictionaries."""

import json
import re
import shutil

from serde_inputs import HERE, retention
from rows_prepare import declarations
from load_prepare import read_exact


def required_view(generated, wrong=False):
    name='controller.lifecycle_inbox.source_work.required_option'
    funs=declarations(generated/'Funs.lean')
    if wrong:
        namespace='SandboxModel.Stage1Protocol.SourceSerdeRequiredWrong'
        header="""import SourceSerdeRequiredView

open Aeneas Aeneas.Std Result ControlFlow SandproofStage1RequestRust
set_option autoImplicit false
set_option linter.style.longLine false

namespace SandboxModel.Stage1Protocol.SourceSerdeRequiredWrong
open SourceSerdeRequiredView
noncomputable section

section
variable (ports : Ports)
local notation "core.option.Option.Insts.Serde_coreDeDeserialize.deserialize" => ports.optionDeserialize

"""
    else:
        namespace='SandboxModel.Stage1Protocol.SourceSerdeRequiredView'
        header="""import SaveLoadDecision

open Aeneas Aeneas.Std Result ControlFlow SandproofStage1RequestRust
set_option autoImplicit false
set_option linter.style.longLine false

namespace SandboxModel.Stage1Protocol.SourceSerdeRequiredView
noncomputable section

/-- The exact named Option Deserialize dictionary operation. Both original
    generic trait dictionaries and the original input are retained. -/
structure Ports where
  optionDeserialize : ∀ {D T E : Type}, serde_core.de.Deserialize T →
    serde_core.de.Deserializer D E → D → Result (core.result.Result (Option T) E)

section
variable (ports : Ports)
local notation "core.option.Option.Insts.Serde_coreDeDeserialize.deserialize" => ports.optionDeserialize

"""
    return header+funs[name]+f'\n\nend\nabbrev required := @{name}\nend\nend {namespace}\n'


def decode_view(generated):
    funs=declarations(generated/'Funs.lean')
    option='core.option.Option.Insts.Serde_coreDeDeserialize'
    record='controller.lifecycle_inbox.source_work.SourceWorkRecord.Insts.Serde_coreDeDeserialize'
    root='controller.lifecycle_inbox.source_work.codec.decode_source_payload'
    namespace='SandboxModel.Stage1Protocol.SourceSerdeDecodeView'
    header="""import SourceSerdeRequiredView

open Aeneas Aeneas.Std Result ControlFlow SandproofStage1RequestRust
set_option autoImplicit false
set_option linter.style.longLine false

namespace SandboxModel.Stage1Protocol.SourceSerdeDecodeView
open SaveLoadCollectorView
noncomputable section

abbrev JsonError := SandproofSaveRetentionRows.serde_json.error.Error
abbrev RecordDecode := ∀ {D E : Type}, serde_core.de.Deserializer D E → D →
  Result (core.result.Result LoadedRecord E)
abbrev SliceDecode := ∀ {T : Type}, serde_core.de.Deserialize T → Slice U8 →
  Result (core.result.Result T JsonError)
structure Ports where
  option : SourceSerdeRequiredView.Ports
  record : RecordDecode
  fromSlice : SliceDecode

section
variable (ports : Ports)
local notation "controller.lifecycle_inbox.source_work.SourceWorkRecord" => LoadedRecord
local notation "serde_json.error.Error" => JsonError
local notation "core.option.Option.Insts.Serde_coreDeDeserialize.deserialize" => ports.option.optionDeserialize
local notation "controller.lifecycle_inbox.source_work.SourceWorkRecord.Insts.Serde_coreDeDeserialize.deserialize" => ports.record
local notation "serde_json.de.from_slice" => ports.fromSlice

"""
    return (header+funs[option]+f'\n\nlocal notation "{option}" => {option} ports\n\n'
            +funs[record]+f'\n\nlocal notation "{record}" => {record} ports\n\n'+funs[root]+f"""

end
abbrev decode := @{root}
abbrev bareDictionary := @{record}
def optionDictionary (ports : Ports) : serde_core.de.Deserialize (Option LoadedRecord) :=
  {option} ports (bareDictionary ports)

end
end {namespace}
""")


def materialize(out, parent, good, bad, decode):
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
        'SourceSerdeRequiredView': required_view(good),
        'SourceSerdeRequiredWrong': required_view(bad, wrong=True),
        'SourceSerdeDecodeView': decode_view(decode),
    }
    for name, source in files.items():
        (out / (name + '.lean')).write_text(source)
    local = ('SourceSerdeSome', 'SourceSerdeDecodeProof', 'SourceSerdeRequiredProof')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in (*files, *local))
    (out / 'lakefile.toml').write_text(lake)
    root = 'SourceSerdeRequiredProof'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
