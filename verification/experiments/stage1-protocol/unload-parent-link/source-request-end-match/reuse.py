"""Select the original general U16 range proof verbatim, without old business types."""
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parents[1] / 'inbox-config-validation/ConfigValidationLibrary.lean.in'


def input_paths():
    return [SOURCE]


def range_module():
    raw = SOURCE.read_bytes()
    begin = raw.index(b'theorem range_u16_refines (lo hi value : U16) :\n')
    end = raw.index(b'\ntheorem range_u32_refines ', begin)
    proof = raw[begin:end].rstrip() + b'\n'
    return (b'import SourceWorkerGuardValues\n\n'
        b'open Aeneas Aeneas.Std SandproofStage1RequestRust\n'
        b'namespace SandboxModel.Stage1Protocol.SourceRequestEndMatchRange\n'
        b'noncomputable section\n\n' + proof + b'\nend\n'
        b'end SandboxModel.Stage1Protocol.SourceRequestEndMatchRange\n\n'
        b'#audit_controller_proof SandboxModel.Stage1Protocol.SourceRequestEndMatchRange.range_u16_refines\n')
