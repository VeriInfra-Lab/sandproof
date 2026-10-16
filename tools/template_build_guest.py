"""Protected Rust material binding for the existing self-service supervisor."""
import json
from pathlib import Path

if __package__:
    from . import guest_artifacts as artifacts
    from .template_guest import load_profile
else:
    import guest_artifacts as artifacts
    from template_guest import load_profile

DIRECTORY = '/input/rust-guest'
PROFILE = DIRECTORY + '/profile.json'
HELPERS = ('template_guest.py', 'guest_artifacts.py', 'build_base_guest.py',
           'build_ci_guest.py', 'system_acceptance.py', 'check_kvm_host.py')


def equal(left, right):
    # JSON types are part of the identity: True and 1 must not compare equal.
    return json.dumps(left, sort_keys=True) == json.dumps(right, sort_keys=True)


def snapshot(path):
    profile, identity = load_profile(path)
    return {'profile': profile, 'identity': identity}


def recheck(config):
    current = snapshot(config['rust_guest_profile'])
    if not equal(current, config['rust_guest']):
        raise ValueError('protected guest profile changed since supervisor configuration')
    return current


def materials(binding):
    """Only fixed runtime destinations; never send the host source archive."""
    profile, identity = binding['profile'], binding['identity']
    release = Path(profile['release'])
    manifest = release / 'manifest.json'
    observed = artifacts.fingerprint(manifest, artifacts.MANIFEST_LIMIT)
    if observed['sha256'] != identity['manifest_sha256']:
        raise ValueError('guest manifest changed before upload')
    files = [(release / name, DIRECTORY + '/release/' + name, identity['artifacts'][role])
             for role, name in artifacts.BINARIES.items()]
    files.extend([(manifest, DIRECTORY + '/release/manifest.json', observed),
                  (Path(profile['configuration']), DIRECTORY + '/config.json', identity['configuration'])])
    translated = dict(profile, release=DIRECTORY + '/release', configuration=DIRECTORY + '/config.json')
    return files, json.dumps(translated, sort_keys=True)


def capacity(binding):
    files, profile = materials(binding)
    sizes = [expected['bytes'] for _, _, expected in files] + [len(profile.encode())]
    sizes.extend(Path(__file__).with_name(name).stat().st_size for name in HELPERS)
    # Necessary additional builder occupancy, including directory blocks; the
    # worker still checks actual free space after all uploads and before unpack.
    occupancy = sum((size + 4095) // 4096 * 4096 for size in sizes) + 2 * 4096
    return max(sizes), occupancy
