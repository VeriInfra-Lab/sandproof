"""Re-extract the two original reads and install their checked transaction view."""
import json
import os
from pathlib import Path
import shutil

from codec_binding import CODEC, IMPLEMENTATION, READS, RECORDING, SOURCE, TRAIT, admit, check_refusals
from codec_receipt import FOLDER, GENERATED, HANDWRITTEN, LIB, PACKAGE, PAGE, RAW_PAGES, TEMPLATES, bound_functions, digest, sha
from wiring import SERVICE, require

PROVIDER = Path('verification/experiments/create-transaction-link/current')


def capture(project, folder, tools, fixed, public, original_view, run):
    patterns = [IMPLEMENTATION+'::'+method for method in READS]
    include = [RECORDING+'::'+method for method in patterns]+[CODEC, TRAIT]
    opaque = [SERVICE, 'sandbox_controller_core', 'sandproof_protocol', 'serde_json', 'serde', 'serde_core', 'rusqlite']
    opts = [arg for flag, group in [('--start-from', patterns), ('--include', include), ('--opaque', opaque)]
            for item in group for arg in (flag, item)]
    raw = folder/'actual.llbc'
    os.environ['CARGO_TARGET_DIR'] = str(folder/'target')
    run(project, folder, 'charon', [tools/'aeneas/charon/bin/charon', 'cargo', '--preset=aeneas', '--error-on-warnings',
        '--sysroot', 'default', *opts, '--dest-file', raw, '--', '--manifest-path', 'controller-service/Cargo.toml',
        '--lib', '--locked', '--offline', '--target', fixed['target']])
    view = original_view(json.loads(raw.read_bytes()))
    options = view.data['options']
    require([t['key'] for t in view.data['target_information']] == [fixed['target']]
            and [options[k] for k in ('start_from','include','opaque')] == [patterns,include,opaque]
            and not any(options[k] for k in ('start_from_if_exists','exclude','skip_borrowck','no_typecheck','monomorphize')),
            'unexpected concrete read extraction')
    generated = folder/'generated'
    run(project, folder, 'aeneas', [tools/'aeneas/bin/aeneas', '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
        '-split-files', '-emit-json', '-namespace', LIB, '-subdir', LIB, '-dest', generated, raw])
    original = (project/SOURCE).read_text()
    public_source = original+(project/PROVIDER/'caller-root.rs.in').read_text()+(project/PACKAGE/'public-journal-root.rs.in').read_text()
    selected = admit(view, public, json.loads((generated/'translation.json').read_bytes()), original, public_source)
    check_refusals(view)
    hashes = {n: sha(generated/n) for n in sorted(GENERATED)}
    require(RAW_PAGES.items() <= hashes.items(), 'reviewed decoder generation differs')
    return dict(ir=sha(raw), selected=selected, generated=hashes)


def materialize(project, folder, closure):
    """Install the original recording provider in its own Lake project; check the reachable inputs."""
    source = project/PROVIDER
    binding = json.loads((source/'rust-source-binding.json').read_bytes())
    require(binding['current_file_overrides'].keys() <= binding['files'].keys(), 'unknown recording override')
    expected = binding['files'] | binding['current_file_overrides']
    # The owner proposal already admits the coordinator refactor from #1064.
    coordinator = 'controller-service/src/application/coordinator/lifecycle_intent.rs'
    require({coordinator, 'Cargo.lock', SOURCE, 'controller-service/src/controller/lifecycle_inbox/model.rs',
             'controller-service/src/controller/checkpoint/config.rs'} <= expected.keys(), 'missing recording input')
    expected[coordinator] = 'ff56d460fa987e09463783986742de6be6773ee75a05c88291c96afa0a2cd897'
    for name, checksum in expected.items():
        require(sha(project/name) == checksum, 'recording provider source differs: '+name)
    modules = json.loads((source/'materialization.json').read_bytes())['modules']
    files = {}
    for entry in modules.values():
        dest = project/entry['path']
        dest.parent.mkdir(exist_ok=True, parents=True)
        data = (project/entry['source']).read_bytes()
        require(digest(data) == entry['sha256'], 'recording provider differs: '+entry['path'])
        dest.write_bytes(data)
        files[entry['path']] = entry['sha256']
    config = (source/'lakefile.toml.in').read_text()
    (project/'lakefile.toml').write_text(config+f'\n[[lean_lib]]\nname = "{LIB}"\nsrcDir = "{FOLDER}"\n')
    target = project/FOLDER/LIB
    target.mkdir(parents=True)
    generated = folder/'generated'/LIB
    shutil.copyfile(generated/'Types.lean', target/'Types.lean')
    (target/'Funs.lean').write_bytes(bound_functions((generated/'Funs.lean').read_bytes()))
    for name in HANDWRITTEN:
        shutil.copyfile(project/TEMPLATES/(name+'.lean.in'), target/(name+'.lean'))
    files.update({str(path.relative_to(project)): sha(path) for path in target.glob('*.lean')})
    reachable = closure(project, [PAGE], active_view='lakefile.toml')
    require(all(files.get(name) == value for name,value in reachable.items())
            and {str(p.relative_to(project)) for p in target.glob('*.lean')} <= reachable.keys(),
            'concrete decoder imports differ')
    return sorted(reachable)
