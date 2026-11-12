"""The four fixed unload audit packages and their original inputs."""
import hashlib
import json
from pathlib import Path
import re

HERE = Path('verification/experiments/stage1-protocol')
LABEL = 'stage1-unload-audits'
KIND = 'stage1-unload-audits-component'
DRIVER = HERE / 'unload_audit_driver.py'
AUDIT = re.compile(r'^#audit_controller_proof (\S+)$', re.M)
PACKAGES = {
    'unload-startup-source': ('startup', 16, 34),
    'unload-base-provenance': ('base-provenance', 23, 52),
    'unload-continued-observation': ('continued-observation', 2, 6),
    'unload-stored-options': ('stored-options', 3, 11),
}


def digest(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError('missing or linked original unload input: ' + str(path))
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def frozen_inputs(root):
    """Keep the existing reviewed Rust, extraction and proof files pinned."""
    inputs = {}
    for suffix, _, _ in PACKAGES.values():
        manifest = Path('verification') / f'v3-unload-{suffix}-inputs.sha256'
        inputs[str(manifest)] = digest(root / manifest)
        for line in (root / manifest).read_text().splitlines():
            expected, name = line.split(maxsplit=1)
            relative = Path(name)
            if (relative.is_absolute() or '..' in relative.parts
                    or not (root / relative).resolve().is_relative_to(root)
                    or digest(root / relative) != expected):
                raise ValueError('original unload frozen input differs: ' + name)
            inputs[name] = expected
    return inputs


def package_audits(templates, package):
    _, count, declarations = PACKAGES[package]
    directory = HERE / package
    files = {name: entries for name, entries in templates.items()
             if Path(name).is_relative_to(directory)}
    names = [name for entries in files.values() for name in entries]
    if len(files) != count or len(names) != declarations or len(set(names)) != declarations:
        raise ValueError('missing, duplicate or extra fixed unload audit: ' + package)
    return files


def capture_hashes(root, package):
    def read(name):
        return json.loads((root / HERE / package / name).read_text())

    if package == 'unload-startup-source':
        evidence = read('evidence.json')['msra']
        return {name: evidence[name + '_llbc_sha256'] for name in ('parent', 'startup_joint')}
    if package == 'unload-base-provenance':
        return read('evidence.json')['llbc_sha256']
    profile = read('capture-profile.json')
    if package == 'unload-continued-observation':
        return {'original': profile['llbc_sha256'], 'final': profile['final_capture']['llbc_sha256']}
    return {name: row['llbc_sha256'] for name, row in profile['captures'].items()}
