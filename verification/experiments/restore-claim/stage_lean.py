"""Install literal tool output and existing proof providers in a private Lake view."""
import argparse
from hashlib import sha256
from pathlib import Path
import shutil


HERE = Path(__file__).resolve().parent
LIBRARY = 'SandproofRestoreClaim'
MODULE = Path('SandboxModel/Lifecycle/Protocol/RestoreClaim')
PROVIDER = Path('SandboxModel/Lifecycle/Protocol/ReplayEquality.lean')
PROVIDER_SHA256 = 'bb2a619f7ce5dad3c6c4effce17595d37a994508e4b64d1c5b246fd559f756ef'
EXTERNALS = {
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
}


def replay_equality(project):
    """Reuse the existing equality proofs under this generation's namespace."""
    source = (project / PROVIDER).read_bytes()
    if sha256(source).hexdigest() != PROVIDER_SHA256:
        raise ValueError('the checked replay proof provider changed')
    original = source.decode()
    start = 'open Aeneas Aeneas.Std SandproofStage1RequestRust'
    end = 'theorem replay_complete'
    _, _, proofs = original.partition(start)
    proofs, marker, _ = proofs.partition(end)
    if not marker or original.count(start) != 1 or 'theorem accepted_eq' not in proofs:
        raise ValueError('the checked replay proof boundary changed')
    return ('import SandproofRestoreClaim.Funs\n\n'
            'open Aeneas Aeneas.Std SandproofRestoreClaimRust'
            + proofs.replace('SandboxModel.Stage1Protocol.Replay',
                              'SandboxModel.Stage1Protocol.RestoreClaim.ReusedReplayEquality')
                    .replace('    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,\n', '')
            + 'end SandboxModel.Stage1Protocol.RestoreClaim.ReusedReplayEquality\n')


def stage(project, generated, wrong):
    scratch_module = project / 'verification' / 'aeneas' / 'restore-claim-private'
    library = scratch_module / LIBRARY
    library.mkdir(parents=True)
    emitted = generated / LIBRARY
    for name in ('Types.lean', 'Funs.lean'):
        shutil.copyfile(emitted / name, library / name)
    template = emitted / 'FunsExternal_Template.lean'
    actual = set()
    if template.is_file():
        actual = {line.split()[1] for line in template.read_text().splitlines()
                  if line.startswith('axiom ')}
    if actual != (set() if wrong else EXTERNALS):
        raise ValueError(f'unexpected external functions: {actual}')
    if not wrong:
        shutil.copyfile(HERE / 'External.lean.in', library / 'FunsExternal.lean')

    folder = project / MODULE
    folder.mkdir(parents=True)
    for name in ('Contract', 'Witness', *(['Wrong'] if wrong else ['Correct', 'Prefix', 'Readback'])):
        shutil.copyfile(HERE / (name + '.lean.in'), folder / (name + '.lean'))
    if not wrong:
        (folder / 'ReusedReplayEquality.lean').write_text(replay_equality(project))

    lakefile = project / 'lakefile.toml'
    with lakefile.open('a') as output:
        output.write(f'\n[[lean_lib]]\nname = "{LIBRARY}"\n'
                     'srcDir = "verification/aeneas/restore-claim-private"\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('generated', type=Path)
    parser.add_argument('--wrong', action='store_true')
    args = parser.parse_args()
    stage(args.project, args.generated, args.wrong)
