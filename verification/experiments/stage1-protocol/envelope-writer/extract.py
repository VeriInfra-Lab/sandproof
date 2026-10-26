"""Fixed-tool current Writer view or original concrete bounded-call capture."""
from pathlib import Path
import argparse
import shutil
import subprocess

HERE = Path('verification/experiments/stage1-protocol/envelope-writer')
OUT = Path('verification/aeneas/stage1-protocol/envelope-writer')


def extract(mode, label):
    roots = (['candidate::write_entry', 'candidate::finish_entry', 'candidate::encode_entry',
              'candidate::response_entry'] if mode == 'policy'
             else ['accepted_bounded', 'recorded_bounded'])
    raw = Path('artifacts') / (label + '.llbc')
    if raw.exists():
        raise ValueError('preserve prior LLBC and use a fresh output label')
    command = ['.tools/aeneas/charon/bin/charon', 'cargo', '--preset=aeneas',
               '--error-on-warnings', '--sysroot', 'default', '--include',
               'sandproof_controller_service', '--include', 'sandproof_internal',
               '--include', 'sandproof_protocol']
    if mode == 'policy':
        # Use the same transparent current enum from the foundations view.
        # The provider below is a type alias, never an axiom/abstract type.
        command += ['--opaque', 'sandproof_protocol::ContractError']
    for root in roots:
        command += ['--start-from', 'crate::' + root]
    command += ['--dest-file', '../../../../' + str(raw), '--', '--manifest-path',
                str(HERE / 'Cargo.toml'), '--lib', '--locked', '--offline']
    if mode == 'policy':
        command += ['--features', 'candidate']
    subprocess.run(command, check=True)
    if mode == 'calls':
        return
    subprocess.run(['.tools/aeneas/bin/aeneas', '-backend', 'lean', '-abort-on-error',
        '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', 'SandproofBoundedWriterRust',
        '-subdir', 'SandproofBoundedWriter', '-dest', str(OUT), str(raw)], check=True)
    # Generated templates are retained as evidence and never imported.
    template = OUT / 'SandproofBoundedWriter/TypesExternal_Template.lean'
    target = Path('artifacts') / (label + '-TypesExternal_Template.lean')
    if target.exists():
        raise ValueError('do not overwrite a prior external-template artifact')
    template.rename(target)
    shutil.copy2(HERE / 'TypesExternal.lean.in', OUT / 'SandproofBoundedWriter/TypesExternal.lean')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=['policy', 'calls'], required=True)
    parser.add_argument('--label', required=True)
    args = parser.parse_args()
    if not args.label.replace('-', '').isalnum():
        raise ValueError('simple fresh evidence label required')
    extract(args.mode, args.label)
