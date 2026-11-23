"""Reproduce the fixed unload leaf extractions on the current native source."""

from hashlib import sha256
import json
import os
from pathlib import Path
import re
import tarfile

HERE = Path('verification/experiments/stage1-protocol')
CONFIG = HERE / 'unload-current-captures.json'
NAMES = ('startup-joint', 'native', 'ledger', 'origin', 'continued', 'encoder', 'reader')
LOCATION = re.compile(rb"(?m)^    Source: '[^'\n]+', lines \d+:\d+-\d+:\d+(?= -/$|\n)")


def scope(root):
    rows = json.loads((root / CONFIG).read_bytes())
    if set(rows) != set(NAMES):
        raise ValueError('missing or extra current Unload capture roots')
    return rows


def admit(root, folder, case, digest):
    row = scope(root)[case]
    ir = json.loads((folder/'actual.llbc').read_bytes())
    translated = ir['translated']
    options = translated['options']
    fixed = json.loads((root/'verification/foundations-environment.json').read_bytes())
    if (ir['has_errors'] or options['start_from'] != row['roots']
            or options['include'] != row['include'] or options['opaque'] != row['opaque']
            or options['exclude'] or any(options[name] for name in (
                'skip_borrowck', 'no_typecheck', 'monomorphize'))
            or {entry['key'] for entry in translated['target_information']} != {fixed['target']}):
        raise ValueError('current Unload extraction changed scope or compiler checks: ' + case)
    names = {
        '::'.join(piece.get('Ident', ['_'])[0] for piece in decl['item_meta']['name'])
        for decl in translated['fun_decls']
        if decl and decl['item_meta']['opacity'] == 'Transparent' and 'Structured' in decl.get('body', {})
    }
    if not set(row['roots']) <= names:
        raise ValueError('missing transparent current Unload root: ' + case)
    generated = folder/'generated'/row['namespace']
    if {file.name for file in generated.iterdir()} != set(row['generated']):
        raise ValueError('changed emitted Unload file range: ' + case)
    for name, expected in row['generated'].items():
        observed = LOCATION.sub(b'    Source: <location>', (generated/name).read_bytes())
        if sha256(observed).hexdigest() != expected:
            raise ValueError('current Unload definition differs from reviewed extraction: ' + case + '/' + name)
    paths = ('actual.llbc', 'charon.log', 'aeneas.log', 'generated/translation.json')
    return {name: digest(folder/name) for name in (
        *paths, *(f'generated/{row["namespace"]}/{name}' for name in sorted(row['generated'])))}


def run(root, archive, output, tools, command, digest):
    output.mkdir(mode=0o700)
    project = output/'source'
    project.mkdir(mode=0o700)
    with tarfile.open(archive) as source:
        source.extractall(project, filter='data')
    fixed = json.loads((root/'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools/'charon/bin/charon', tools/'bin/aeneas'
    if (digest(charon) != fixed['charon']['binary_sha256']
            or digest(aeneas) != fixed['aeneas']['binary_sha256']):
        raise ValueError('fixed Unload tools changed')
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
               CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output/'target'))
    for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        env.pop(key, None)

    evidence = {}
    for case, row in scope(root).items():
        folder = output/case
        folder.mkdir()
        raw, generated = folder/'actual.llbc', folder/'generated'
        selectors = [arg for flag, key in (
            ('--start-from', 'roots'), ('--include', 'include'), ('--opaque', 'opaque')
        ) for entry in row[key] for arg in (flag, entry)]
        original = None
        if row.get('name_metadata'):
            history = root/HERE/'original-source-rows/original-capture'
            plan = json.loads((history/'name-metadata-plan.json').read_bytes())
            file = project/plan['source']
            original = file.read_bytes()
            replacement = (history/'metadata-view-lib.rs').read_bytes()
            if (sha256(original).hexdigest() != plan['original_sha256']
                    or sha256(replacement).hexdigest() != plan['view_sha256']):
                raise ValueError('unreviewed startup naming view')
            file.write_bytes(replacement)
        if row.get('origin_forwarder'):
            file = project/'controller-service/src/controller/lifecycle_inbox/model.rs'
            original = file.read_bytes()
            body = (root/HERE/'inbox-origin-selection/roots.rs.in').read_bytes()
            if (body.count(b'session.current_execution_origin()') != 1
                    or b'proof_execution_origin' in original):
                raise ValueError('current Unload origin forwarder changed')
            file.write_bytes(original + b'\n' + body)
        try:
            command(env, project, folder, 'charon',
                    [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                     *selectors, '--dest-file', raw, '--', '--manifest-path', 'controller-service/Cargo.toml',
                     '--lib', '--locked', '--offline', '--target', fixed['target']])
        finally:
            if original is not None:
                file.write_bytes(original)
        command(env, project, folder, 'aeneas',
                [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                 '-emit-json', '-namespace', row['namespace'], '-subdir', row['namespace'],
                 '-dest', generated, raw])
        evidence[case] = admit(root, folder, case, digest)
    return evidence
