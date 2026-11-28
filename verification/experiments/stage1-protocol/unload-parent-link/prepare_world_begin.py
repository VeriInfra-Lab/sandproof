"""Join the admitted generated begin body to the exact #770 provider."""
import argparse
import json
from pathlib import Path
import shutil

from check_revision_pair import checked_commands, generated_inventory
from prepare_started_transaction import HERE, ROOT, closure, digest, materialize
from world_begin_capture import MODULE, SOURCE_COMMIT, SOURCE_INPUTS, SOURCE_TREE, admit, configuration


PROOF = 'UnloadWorldBegin'
VIEW = 'verification/aeneas/unload-world-begin'
DRIVER = 'a290351cb5358ae90c2637b9e6299069dc08b7fb'
ARCHIVE_SHA256 = '5e3c46112d1becd898fb070b4fd10373874184aef6fe40b1727e3fe6381fb824'
IR_SHA256 = '4680c02adfd6301f7de62675d515ea343c6da69b53eb979e0370f83f7f815b59'
GENERATED = {
    'Types.lean': '93da9f88d22fe9d5b6f63fccd982b7717e035701e1eabe046ebace664027b6d1',
    'Funs.lean': 'c1da246513bc558b0178b3fb61e02696053bce5a9312e560b553f9fca9606799',
}
TRANSLATION_SHA256 = 'aab2ff2d21b65dfbf85b87769b681814c870da40cd3ad8cf5420e770e21cb023'


def read_exact(path, expected):
    if path.is_symlink():
        raise ValueError(f'world begin capture input must be a regular file: {path.name}')
    data = path.read_bytes()
    if digest(data) != expected:
        raise ValueError(f'world begin capture input differs: {path.name}')
    return data


def prepare(capture, output, packages, lean_mcp, *, identity=None):
    receipt = json.loads((capture / 'result.json').read_bytes())
    if identity is None:
        source, tree, driver, archive, ir_digest = (
            SOURCE_COMMIT, SOURCE_TREE, DRIVER, ARCHIVE_SHA256, IR_SHA256)
    else:
        source, tree, driver, archive = (
            identity['head'], identity['tree'], identity['driver_head'], identity['source_archive_sha256'])
        ir_digest = receipt['ir_sha256']
    if (receipt['mode'], receipt['head'], receipt['tree'], receipt['driver_head'],
            receipt['complete'], receipt['ir_sha256']) != (
            'world-begin', source, tree, driver, True, ir_digest):
        raise ValueError('the admitted world begin capture is required')
    read_exact(capture / 'source.tar', archive)
    for name, expected in SOURCE_INPUTS.items():
        read_exact(capture / 'source' / name, expected)
        read_exact(ROOT / name, expected)
    ir = capture / 'actual.llbc'
    translation = capture / 'generated'
    read_exact(ir, ir_digest)
    inventory = generated_inventory(translation)
    if inventory != receipt['generated']:
        raise ValueError('world begin generated file inventory differs from the capture')
    if identity is None:
        read_exact(translation / 'translation.json', TRANSLATION_SHA256)
        hashes = GENERATED
    else:
        hashes = {name: inventory[f'{MODULE}/{name}'] for name in GENERATED}
    bodies = {name: read_exact(translation / MODULE / name, expected) for name, expected in hashes.items()}
    profile = ROOT / 'verification/experiments/stage1-protocol/unload-startup-source/capture-profile.json'
    cfg = configuration(json.loads(profile.read_bytes())['parent'])
    environment = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    checked_commands(capture, receipt['commands'], cfg, environment['target'])
    if admit(ir, translation, cfg, environment['target']) != receipt['begin_admission']:
        raise ValueError('original begin capture no longer satisfies admission')

    materialize(output, packages)
    tools = (output / '.tools')
    tools.mkdir()
    (tools / 'lean-mcp').symlink_to(lean_mcp, target_is_directory=True)
    for name in ('tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, destination)
    plan_file = output / 'plan.json'
    provider = json.loads(plan_file.read_bytes())['files']
    with (output / 'lakefile.toml').open('a') as config:
        for library in (MODULE, PROOF):
            config.write(f'\n[[lean_lib]]\nname = "{library}"\nsrcDir = "{VIEW}"\n')

    support = HERE / 'world-begin'
    generated = output / VIEW / MODULE
    generated.mkdir(parents=True)
    for name, data in bodies.items():
        (generated / name).write_bytes(data)
    for name in ('TypesExternal.lean', 'FunsExternal.lean'):
        shutil.copyfile(support / (name + '.in'), generated / name)
    proof = f'{VIEW}/{PROOF}.lean'
    shutil.copyfile(support / (PROOF + '.lean.in'), output / proof)

    added = {f'{VIEW}/{MODULE}/{name}' for name in (*GENERATED, 'TypesExternal.lean', 'FunsExternal.lean')}
    added.add(proof)
    files = closure(output, [proof], active_view='lakefile.toml')
    if {name: value for name, value in files.items() if name not in added} != provider or added - files.keys():
        raise ValueError('unexpected imports or missing files in the original provider closure')
    plan = dict(source=source, driver=driver, ir=ir_digest, root=PROOF, files=files, strict_files=sorted(added),
                modules={name: {'sha256': value} for name, value in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    plan_file.write_text(json.dumps(plan, indent=2) + '\n')
    print(f'{PROOF}: {len(files)} exact Lean inputs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lake-packages', type=Path, required=True)
    parser.add_argument('--lean-mcp', type=Path, required=True)
    args = parser.parse_args()
    prepare(args.capture.resolve(strict=True), args.output.resolve(), args.lake_packages.resolve(strict=True),
            args.lean_mcp.resolve(strict=True))
