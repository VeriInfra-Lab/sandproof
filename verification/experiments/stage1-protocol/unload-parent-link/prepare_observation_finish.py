"""Materialize the new observation tail in an exact reviewed UnloadFinish environment."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import tarfile

import observation_finish_capture as capture
from prepare_started_transaction import PROVIDER
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).resolve().parent / 'observation-commit'
VIEW = 'verification/aeneas/unload-finish'
MODULE = 'SandproofUnloadFinish'
ROOTS = {'tail': 'ObservationFinish', 'reordered': 'ObservationFinishReordered'}
PRIOR_PLANS = {
    'tail': '47628a7203a33beb0bb8c147f3278845cdfabc6c8548054fb0ff88688e64d56b',
    'reordered': '1d7ab41bd96d691e7be08b95e1d229f4c3a9159ec671dde626bf6289c4ce6e6c',
}
LAKEFILE = {
    'tail': 'e343d93e4a0400f2a413b2a3df41506fec6d5f5a2c17a891657fbd36821a98d2',
    'reordered': 'eccacaa0b948fed0736a9b185bf1f36b04528f5abc92a8214e8d3e166d989dca',
}
SUPPORT = {
    'lake-manifest.json': 'feadd669f2533ebb6189f12526b29eb3092331ade8782df28701744d66100264',
    'lean-toolchain': 'efac0b94923b2d8b6840cd35be9177ad0fc5ab2332f4f4311c98712cee92fdee',
    'tools/lean_session.py': '346adcc6a2c78d57fbbbc372838b1438e9692455050e3a15b1e0b77973b3fdc3',
    'scripts/lean-mcp-server.sh': '8969d229a831f7df24654c326c5ebb311b8d40bce466e278d3b0098092fd4d6f',
}
SOURCE_LINE = rb"(?m)^    Source: 'controller-service/src/controller/lifecycle_inbox/source_work\.rs', lines [0-9]+:[0-9]+-[0-9]+:[0-9]+ -/$"
RETURN_DOC = b'/-- [sandproof_controller_service::controller::lifecycle_inbox::source_work::finish_observation_save]:'


def captured(directory, mode):
    report = json.loads((directory / 'result.json').read_bytes())
    if (report['complete'] is not True or report['mode'] != mode
            or (report['source_commit'], report['source_tree']) != (capture.SOURCE, capture.TREE)):
        raise ValueError('the exact complete observation capture is required')
    read_exact(Path(capture.__file__), report['driver_sha256'])
    archive = directory / 'source.tar'
    read_exact(archive, capture.ARCHIVE)
    with tarfile.open(archive) as contents:
        original = contents.extractfile(capture.prepare.SOURCE).read().decode()
    candidate = capture.factor(original, reordered=mode == 'reordered').encode()
    candidate_sha256 = sha256(candidate).hexdigest()
    if candidate_sha256 != report['candidate_source_sha256']:
        raise ValueError('the exact factored candidate is required')
    for name, pin in capture.prepare.SOURCE_INPUTS.items():
        read_exact(directory / 'source' / name, candidate_sha256 if name == capture.prepare.SOURCE else pin)
    binding = json.loads((PROVIDER / 'rust-source-binding.json').read_bytes())['files']
    for name in ('controller-service/src/controller/transaction.rs',
                 'controller-service/src/controller/transaction/memo.rs', 'Cargo.lock'):
        read_exact(directory / 'source' / name, binding[name])
    commands = ['charon'] if mode == 'parent' else ['charon', 'aeneas']
    if ([row['label'] for row in report['commands']] != commands
            or any(row['exit_code'] != 0 or row['bounded_stop'] is not False for row in report['commands'])):
        raise ValueError('the original required extractors must have completed')
    for row in report['commands']:
        read_exact(directory / (row['label'] + '.log'), row['log_sha256'])
    ir = directory / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = directory / 'generated'
    expected = set() if mode == 'parent' else {'translation.json', *(f'{MODULE}/{name}.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (set(report['generated_sha256']) != expected
            or {str(path.relative_to(generated)) for path in generated.rglob('*') if path.is_file()} != expected):
        raise ValueError('a complete exact generated inventory is required')
    files = {name: read_exact(generated / name, pin) for name, pin in report['generated_sha256'].items()}
    fixed = json.loads((capture.ROOT / 'verification/foundations-environment.json').read_bytes())
    if capture.admit(ir, generated, capture.configuration(mode), fixed['target'], mode) != report['admission']:
        raise ValueError('the original observation admission differs')
    return files, report


def checked_body(generated, prior):
    """Compare only old generated definitions. Preserve new original bytes on disk."""
    start, wrapper = generated['Funs.lean'].split(RETURN_DOC)
    if b'def controller.lifecycle_inbox.source_work.finish_observation_save\n' not in wrapper:
        raise ValueError('new actual returned-record definition is required')
    if wrapper.count(b'\ndef ') != 1 or not wrapper.endswith(b'end SandproofUnloadFinish\n'):
        raise ValueError('only one actual return definition may be added')
    calls = {name: re.sub(SOURCE_LINE, b'', data) for name, data in prior.items()}
    if (re.sub(SOURCE_LINE, b'', start + b'end SandproofUnloadFinish\n') != calls['Funs.lean']
            or re.sub(SOURCE_LINE, b'', generated['Types.lean']) != calls['Types.lean']):
        raise ValueError('the old helper and its generated types must match the admitted version')


def materialize(mode, actual, parent, prior, output, packages, lean_mcp):
    generated, report = captured(actual, mode)
    _, parent_report = captured(parent, 'parent')
    if mode == 'tail' and report['candidate_source_sha256'] != parent_report['candidate_source_sha256']:
        raise ValueError('the parent and original tail must use byte-identical Rust')
    plan = json.loads(read_exact(prior / 'plan.json', PRIOR_PLANS[mode]))
    incoming = {name: read_exact(prior / name, pin) for name, pin in plan['files'].items()}
    new = {name: generated[f'{MODULE}/{name}'] for name in ('Funs.lean', 'Types.lean')}
    checked_body(new, {name: incoming[f'{VIEW}/{MODULE}/{name}'] for name in new})
    incoming.update({f'{VIEW}/{MODULE}/{name}': data for name, data in new.items()})
    selected = ROOTS[mode]
    added = ('ObservationFinishResult', selected)
    for stem in added:
        incoming[f'{VIEW}/{stem}.lean'] = (HERE / (stem + '.lean.in')).read_bytes()
    output.mkdir(mode=0o700)
    for name, data in incoming.items():
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    for name, pin in SUPPORT.items():
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_exact(prior / name, pin))
    (output / '.lake').mkdir()
    (output / '.tools').mkdir()
    (output / '.lake/packages').symlink_to(packages, target_is_directory=True)
    (output / '.tools/lean-mcp').symlink_to(lean_mcp, target_is_directory=True)
    lakefile = read_exact(prior / 'lakefile.toml', LAKEFILE[mode]).decode()
    for stem in added:
        lakefile += f'\n[[lean_lib]]\nname = "{stem}"\nsrcDir = "{VIEW}"\n'
    (output / 'lakefile.toml').write_text(lakefile)
    files = closure(output, [f'{VIEW}/{selected}.lean'], active_view='lakefile.toml')
    if files != {name: sha256(data).hexdigest() for name, data in incoming.items()}:
        raise ValueError('the entire prior closure and both new modules must remain reachable')
    plan.update(source=capture.SOURCE, root=selected, mode=mode, files=files,
                modules={name: {'sha256': pin} for name, pin in files.items()},
                build_order=[name.removesuffix('.lean') for name in files],
                strict_files=sorted(set(plan['strict_files']) | {f'{VIEW}/{stem}.lean' for stem in added}),
                observation_tail_ir=report['ir_sha256'], observation_parent_ir=parent_report['ir_sha256'],
                observation_candidate=report['candidate_source_sha256'], prior_plan=PRIOR_PLANS[mode])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(selected, len(files), 'exact physical inputs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', required=True, choices=tuple(ROOTS))
    for name in ('actual', 'parent', 'prior', 'output', 'lake-packages', 'lean-mcp'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    materialize(args.mode, args.actual.resolve(strict=True), args.parent.resolve(strict=True),
                args.prior.resolve(strict=True), args.output.resolve(),
                args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True))
