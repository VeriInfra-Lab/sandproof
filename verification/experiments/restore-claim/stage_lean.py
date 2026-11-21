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
CLONE_EXTERNALS = {
    'core.option.Option.Insts.CoreCloneClone.clone',
    'alloc.string.String.Insts.CoreCloneClone.clone',
}


def fixed_source(project, path, expected_sha256):
    source = (project / path).read_bytes()
    if sha256(source).hexdigest() != expected_sha256:
        raise ValueError(f'the checked proof provider changed: {path}')
    return source.decode()


def replay_equality(project):
    """Reuse the existing equality proofs under this generation's namespace."""
    original = fixed_source(project, PROVIDER, PROVIDER_SHA256)
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


def action_cloning(project):
    """Reuse the action-only Clone theorems in the actual generated namespace."""
    root = Path('verification/experiments/stage1-protocol')
    protocol = fixed_source(
        project, root / 'inbox-record/InboxRecordCloning.lean.in',
        '0026a0ee554a7e7eaba1786de874debb3d03a164832d83f1c56d13809b04f7af')
    restore = fixed_source(
        project, root / 'restore-work/RestoreWorkCloning.lean.in',
        '9480131fcc1d2bcc86bcf9482c5a4059a3dd481cc9e44c6799f981de5efd659d')
    leaves = ''.join(
        '@[simp] theorem ' + name + protocol.split('@[simp] theorem ' + name, 1)[1]
        .split('@[simp] theorem ', 1)[0]
        for name in ('SandboxId_clone', 'ResourceId_clone', 'TenantId_clone'))
    action = '@[simp] theorem vec_clone_exact' + restore.split(
        '@[simp] theorem vec_clone_exact', 1)[1].split(
        '@[simp] theorem sandproof_protocol_lifecycle_manifest_CaptureBinding_clone_exact', 1)[0]
    return ('import SandproofRestoreClaim.Funs\nimport SandboxModel.Foundations.Cloning\n\n'
            'open Aeneas Aeneas.Std SandproofRestoreClaimRust\n'
            'namespace SandboxModel.Stage1Protocol.RestoreClaim.ReusedActionCloning\n\n'
            + leaves + action + 'end SandboxModel.Stage1Protocol.RestoreClaim.ReusedActionCloning\n')


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
    if actual != (CLONE_EXTERNALS if wrong else CLONE_EXTERNALS | EXTERNALS):
        raise ValueError(f'unexpected external functions: {actual}')
    shutil.copyfile(HERE / 'External.lean.in', library / 'FunsExternal.lean')

    folder = project / MODULE
    folder.mkdir(parents=True)
    for name in ('Contract', 'Witness', *(['Wrong'] if wrong else ['Correct', 'Prefix', 'RowWrite', 'Readback'])):
        shutil.copyfile(HERE / (name + '.lean.in'), folder / (name + '.lean'))
    if not wrong:
        (folder / 'ReusedReplayEquality.lean').write_text(replay_equality(project))
        (folder / 'ReusedActionCloning.lean').write_text(action_cloning(project))

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
